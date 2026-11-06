use super::*;

/// How one side of an equivalence check ended on one path.
struct EquivalenceEnd {
    /// Whether the top-level call returned successfully.
    success: bool,
    /// Return data on success, revert data otherwise (empty for exceptional halts).
    return_data: SymReturnData,
}

/// One completed path of one program and how it ended.
type ProgramEnd = (EquivalenceEnd, PathState);

/// Symbolic transaction and block environment shared by both programs.
struct EquivalenceEnv {
    address: SymExpr,
    caller: SymExpr,
    origin: SymExpr,
    value: SymExpr,
    timestamp: SymExpr,
    number: SymExpr,
    prevrandao: SymExpr,
    gas_limit: SymExpr,
    basefee: SymExpr,
    blob_basefee: SymExpr,
    chain_id: SymExpr,
    coinbase: SymExpr,
    gas_price: SymExpr,
    world: SymExpr,
    constraints: Vec<SymBoolExpr>,
}

impl EquivalenceEnv {
    fn new(cx: &mut SymCx, input: &SymbolicEquivalenceInput<'_>) -> Self {
        // The programs' account is the concrete account they are installed at, so that an address
        // the programs compute or read from calldata reaches their own code when it equals it.
        let address = SymExpr::constant(cx, address_word(input.address));
        let caller = SymExpr::var(cx, "env_caller");
        let origin = SymExpr::var(cx, "env_origin");
        let coinbase = SymExpr::var(cx, "env_coinbase");
        let value = match input.value {
            Some(value) => SymExpr::constant(cx, value),
            None => SymExpr::var(cx, "env_callvalue"),
        };
        let address_limit = U256::ONE << 160;
        let mut constraints = Vec::new();
        for word in [&address, &caller, &origin, &coinbase] {
            constraints.push(SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ult, word, address_limit));
        }
        // The programs' own account is distinct from the accounts that call them.
        for word in [&caller, &origin] {
            constraints.push(SymBoolExpr::eq(cx, address.clone(), word.clone()).not(cx));
        }
        let timestamp = SymExpr::var(cx, "env_timestamp");
        let number = SymExpr::var(cx, "env_number");
        let gas_limit = SymExpr::var(cx, "env_gaslimit");
        let basefee = SymExpr::var(cx, "env_basefee");
        let chain_id = SymExpr::var(cx, "env_chainid");
        let gas_price = SymExpr::var(cx, "env_gasprice");
        // Block and fee values fit in 64 bits on every chain, as revm stores them.
        for word in [&timestamp, &number, &gas_limit, &basefee, &chain_id] {
            constraints.push(SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ult, word, U256::ONE << 64));
        }
        constraints.push(SymBoolExpr::cmp_word_const(
            cx,
            SymCmpOp::Ult,
            &gas_price,
            U256::ONE << 128,
        ));
        Self {
            address,
            caller,
            origin,
            value,
            timestamp,
            number,
            prevrandao: SymExpr::var(cx, "env_prevrandao"),
            gas_limit,
            basefee,
            blob_basefee: SymExpr::var(cx, "env_blobbasefee"),
            chain_id,
            coinbase,
            gas_price,
            world: SymExpr::var(cx, "unknown_world_0"),
            constraints,
        }
    }
}

/// Returns the distinct `PUSH4` immediates of the codes, which include every selector a
/// Solidity or Vyper dispatcher compares calldata against. Returns none when there are too many
/// for splitting by selector to pay off.
fn dispatcher_selectors(codes: &[&Bytes]) -> Vec<[u8; 4]> {
    let mut selectors = Vec::new();
    for code in codes {
        let mut pc = 0;
        while pc < code.len() {
            let op = code[pc];
            if op == opcode::PUSH4
                && let Some(value) = code.get(pc + 1..pc + 5)
            {
                let value: [u8; 4] = value.try_into().expect("four bytes");
                if !selectors.contains(&value) {
                    selectors.push(value);
                }
            }
            pc += 1 + if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
                usize::from(op - opcode::PUSH1 + 1)
            } else {
                0
            };
        }
    }
    if selectors.len() > MAX_SELECTOR_JOBS { Vec::new() } else { selectors }
}

/// Most selector jobs an equivalence check splits calldata into.
const MAX_SELECTOR_JOBS: usize = 64;

/// First fresh-symbol index of program B, apart from program A's.
const B_SYMBOL_OFFSET: usize = 1 << 40;

/// Most pre-state storage reads whose pairs get congruence lemmas.
const MAX_CONGRUENT_STORAGE_READS: usize = 64;

/// Pair queries collected per solver job before a batch is solved.
const PAIRS_PER_JOB: usize = 1;

/// Error text of a model query on a path that turned out unsatisfiable.
pub(super) const PATH_BECAME_UNSAT: &str = "counterexample path became unsat";

/// Returns whether two paths' constraints cannot both hold because they pin one word to different
/// constants, as different function selectors do. This is a cheap syntactic filter: pairs it keeps
/// still go to the solver.
fn constraints_conflict(left: &[SymBoolExpr], right: &[SymBoolExpr]) -> bool {
    let mut pinned = HashMap::<SymExpr, U256>::default();
    let mut excluded = HashSet::<(SymExpr, U256)>::default();
    for constraint in left.iter().chain(right) {
        let Some((word, value, holds)) = constraint.word_const_equality() else { continue };
        if holds {
            if pinned.insert(word.clone(), value).is_some_and(|previous| previous != value)
                || excluded.contains(&(word.clone(), value))
            {
                return true;
            }
        } else {
            if pinned.get(word) == Some(&value) {
                return true;
            }
            excluded.insert((word.clone(), value));
        }
    }
    false
}

struct SymbolicEquivalenceCounterexample {
    calldata: Bytes,
    args: Vec<DynSolValue>,
    storage: Vec<SymbolicStorageAssignment>,
    env: SymbolicEquivalenceEnv,
    unknown_calls: usize,
}

impl SymbolicExecutor {
    /// Checks whether two runtime bytecodes behave identically for every modeled input.
    ///
    /// Both codes run at one account from the same symbolic pre-state on shared symbolic calldata
    /// and a shared symbolic environment: sender, origin, call value, block values, gas price,
    /// and the balances, code sizes and code hashes of accounts without known code. Each program
    /// reads its own unconstrained `GAS`. Calls into unknown code are abstracted as one fixed
    /// unknown function of their inputs, so equal calls of the two programs get equal answers.
    ///
    /// The programs are equivalent when every pair of feasible paths agrees on success and on
    /// return or revert data and, when both succeed, on the emitted logs and on the storage,
    /// balance, nonce and code of every account either one changed. Gas is not priced, so
    /// balances only move by value transfers and self-destructs.
    ///
    /// A counterexample is a solver model and must be replayed concretely before it is reported.
    pub fn run_equivalence(
        &mut self,
        input: SymbolicEquivalenceInput<'_>,
    ) -> SymbolicEquivalenceResult {
        self.reset_run_state(true);
        self.solver.clear_context_caches();
        self.cx = SymCx::new();
        if input.code_a == input.code_b {
            return SymbolicEquivalenceResult::Equivalent { stats: SymbolicStats::default() };
        }
        if let Err(err) = self.solver.check_available() {
            return SymbolicEquivalenceResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        self.equivalence = Some(EquivalenceMode {
            promise_no_reent: input.promise_no_reent,
            max_returndata: input.max_returndata,
            reads_gas: false,
            ignore_out_of_gas: input.ignore_out_of_gas,
            jobs: input.jobs.max(1),
            merge_states: input.merge_states,
        });
        self.solver.set_jobs(input.jobs);
        self.solver.set_reuse_models(true);
        let mut completed_paths = 0;
        let result = self.run_equivalence_inner(&input, &mut completed_paths);
        let reads_gas = self.equivalence.take().is_some_and(|mode| mode.reads_gas);
        self.solver.set_reuse_models(false);
        match result {
            Ok(Some(counterexample)) => SymbolicEquivalenceResult::Counterexample {
                calldata: counterexample.calldata,
                args: counterexample.args,
                storage: counterexample.storage,
                env: Box::new(counterexample.env),
                unknown_calls: counterexample.unknown_calls,
                reads_gas,
                stats: self.stats_with_paths(completed_paths),
            },
            Ok(None) => match self.deferred_incomplete() {
                Some((kind, reason)) => SymbolicEquivalenceResult::Incomplete {
                    kind,
                    reason,
                    stats: self.stats_with_paths(completed_paths),
                },
                None => SymbolicEquivalenceResult::Equivalent {
                    stats: self.stats_with_paths(completed_paths),
                },
            },
            Err(err) => SymbolicEquivalenceResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.stats_with_paths(completed_paths),
            },
        }
    }

    fn run_equivalence_inner(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        completed_paths: &mut usize,
    ) -> Result<Option<SymbolicEquivalenceCounterexample>, SymbolicError> {
        let code_a = SymCode::from_bytecode(&mut self.cx, &Bytecode::new_raw(input.code_a.clone()));
        let code_b = SymCode::from_bytecode(&mut self.cx, &Bytecode::new_raw(input.code_b.clone()));
        let env = EquivalenceEnv::new(&mut self.cx, input);

        let max = self.config.max_calldata_bytes as usize;
        let calldatas = match input.function {
            Some(function) => SymbolicCalldata::variants(function, &self.config, &mut self.cx)?,
            None if input.calldata_lengths.is_empty() => {
                if input.max_calldata > max {
                    return Err(SymbolicError::Unsupported(
                        "symbolic calldata size exceeds configured max",
                    ));
                }
                let calldata =
                    SymbolicCalldata::symbolic_size(&mut self.cx, input.max_calldata, "calldata");
                // One job per function selector keeps each job's paths, and the pairs between
                // them, to one function.
                let selectors = dispatcher_selectors(&[&input.code_a, &input.code_b]);
                calldata.split_by_selector(&mut self.cx, &selectors, "calldata")
            }
            None => {
                if input.calldata_lengths.iter().any(|len| *len > max) {
                    return Err(SymbolicError::Unsupported(
                        "symbolic calldata size exceeds configured max",
                    ));
                }
                input
                    .calldata_lengths
                    .iter()
                    .map(|len| SymbolicCalldata::raw(&mut self.cx, *len, "calldata"))
                    .collect()
            }
        };

        let jobs = calldatas.len();
        for (job, calldata) in calldatas.into_iter().enumerate() {
            self.check_timeout()?;
            let job_started = Instant::now();
            let paths_before = *completed_paths;
            debug!(job, jobs, "equivalence job started");
            let found = if input.pair_independently {
                self.check_pairs_independently(
                    input,
                    &env,
                    &calldata,
                    (&code_a, &code_b),
                    completed_paths,
                )?
            } else {
                self.check_pairs_constrained(
                    input,
                    &env,
                    &calldata,
                    (&code_a, &code_b),
                    completed_paths,
                )?
            };
            debug!(
                job,
                elapsed = ?job_started.elapsed(),
                paths = *completed_paths - paths_before,
                "equivalence job finished"
            );
            if let Some(found) = found {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// Runs B once under each path of A, so B's exploration sees A's constraints, and compares the
    /// two end states of each resulting pair.
    fn check_pairs_constrained(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        env: &EquivalenceEnv,
        calldata: &SymbolicCalldata,
        (code_a, code_b): (&SymCode, &SymCode),
        completed_paths: &mut usize,
    ) -> Result<Option<SymbolicEquivalenceCounterexample>, SymbolicError> {
        let root_a = self.equivalence_root(input, env, calldata.clone(), code_a);
        let mut side_a = SequenceCall {
            code: code_a.clone(),
            worklist: VecDeque::from([root_a]),
            deferred_worklist: VecDeque::new(),
        };
        let batch = self.solver_jobs().saturating_mul(PAIRS_PER_JOB).max(1);
        while let Some(outcome_a) =
            self.execute_sequence_call_next(&input.executor, &mut side_a, completed_paths)?
        {
            let end_a = self.equivalence_end(&outcome_a);
            let state_a = outcome_a.state;
            let mut ends_a = vec![(end_a, state_a)];
            // B starts from the same pre-state, constrained to A's path, and its fresh symbols
            // come after A's.
            let mut root_b = self.equivalence_root(input, env, calldata.clone(), code_b);
            root_b.take_noncommitting_check_state(&mut ends_a[0].1.clone());
            let mut side_b = SequenceCall {
                code: code_b.clone(),
                worklist: VecDeque::from([root_b]),
                deferred_worklist: VecDeque::new(),
            };
            let mut ends_b = Vec::new();
            let mut pending = Vec::new();
            while let Some(outcome_b) =
                self.execute_sequence_call_next(&input.executor, &mut side_b, completed_paths)?
            {
                self.check_timeout()?;
                let end_b = self.equivalence_end(&outcome_b);
                ends_b.push((end_b, outcome_b.state));
                let index_b = ends_b.len() - 1;
                if let Some(query) =
                    self.pair_query(input, &mut ends_a[0], &mut ends_b[index_b], true)?
                {
                    pending.push((0, index_b, query.0, query.1));
                }
                if pending.len() >= batch
                    && let Some(found) = self.solve_equivalence_pairs(
                        input,
                        env,
                        calldata,
                        (&ends_a, &ends_b),
                        &mut pending,
                    )?
                {
                    return Ok(Some(found));
                }
            }
            if let Some(found) = self.solve_equivalence_pairs(
                input,
                env,
                calldata,
                (&ends_a, &ends_b),
                &mut pending,
            )? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// Like hevm, explores each program once and compares every pair of end states whose path
    /// constraints may hold together.
    fn check_pairs_independently(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        env: &EquivalenceEnv,
        calldata: &SymbolicCalldata,
        (code_a, code_b): (&SymCode, &SymCode),
        completed_paths: &mut usize,
    ) -> Result<Option<SymbolicEquivalenceCounterexample>, SymbolicError> {
        let mut ends_a =
            self.explore_equivalence_side(input, env, calldata, code_a, 0, completed_paths)?;
        let mut ends_b = self.explore_equivalence_side(
            input,
            env,
            calldata,
            code_b,
            B_SYMBOL_OFFSET,
            completed_paths,
        )?;
        let batch = self.solver_jobs().saturating_mul(PAIRS_PER_JOB).max(1);
        let mut pending = Vec::new();
        for index_a in 0..ends_a.len() {
            for index_b in 0..ends_b.len() {
                self.check_timeout()?;
                let (left, right) = (&mut ends_a[index_a], &mut ends_b[index_b]);
                if let Some(query) = self.pair_query(input, left, right, false)? {
                    pending.push((index_a, index_b, query.0, query.1));
                }
                if pending.len() >= batch
                    && let Some(found) = self.solve_equivalence_pairs(
                        input,
                        env,
                        calldata,
                        (&ends_a, &ends_b),
                        &mut pending,
                    )?
                {
                    return Ok(Some(found));
                }
            }
        }
        self.solve_equivalence_pairs(input, env, calldata, (&ends_a, &ends_b), &mut pending)
    }

    /// Returns the query whose satisfiability means the two end states differ on some input
    /// both paths take, with the replayable storage symbols of both paths, or `None` when they
    /// cannot differ. `b_extends_a` says B's path constraints already include A's.
    fn pair_query(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        (end_a, state_a): &mut ProgramEnd,
        (end_b, state_b): &mut ProgramEnd,
        b_extends_a: bool,
    ) -> Result<Option<(Vec<SymBoolExpr>, SymbolicVars)>, SymbolicError> {
        if !b_extends_a && constraints_conflict(&state_a.constraints, &state_b.constraints) {
            return Ok(None);
        }
        let mismatch =
            self.equivalence_mismatch(&input.executor, state_a, end_a, state_b, end_b)?;
        if mismatch.as_const() == Some(false) {
            return Ok(None);
        }
        let mut constraints = if b_extends_a { Vec::new() } else { state_a.constraints.clone() };
        constraints.extend(state_b.constraints.iter().cloned());
        constraints.extend(self.unknown_call_axioms(
            &input.executor,
            &state_a.unknown_calls,
            &state_b.unknown_calls,
        )?);
        constraints.push(mismatch);
        let congruence = hash_congruence_lemmas(&mut self.cx, &constraints);
        constraints.extend(congruence);
        let storage = self.storage_congruence_lemmas(&state_a.world, &state_b.world);
        constraints.extend(storage);
        let mut replayable = state_a.world.replay_storage_symbols();
        replayable.extend(state_b.world.replay_storage_symbols());
        Ok(Some((constraints, replayable)))
    }

    /// Returns lemmas that make the pre-state storage one function of account and slot: two reads
    /// of one account at equal slots see the same value.
    ///
    /// Each read at a symbolic slot is a symbol named by the slot expression's structure, so two
    /// programs that compute one slot differently would otherwise read unrelated values.
    fn storage_congruence_lemmas(
        &mut self,
        world_a: &SymbolicWorld,
        world_b: &SymbolicWorld,
    ) -> Vec<SymBoolExpr> {
        let mut reads = world_a.storage_base_reads(&mut self.cx);
        for read in world_b.storage_base_reads(&mut self.cx) {
            if !reads.iter().any(|(symbol, address, key)| {
                *symbol == read.0 && *address == read.1 && *key == read.2
            }) {
                reads.push(read);
            }
        }
        reads.truncate(MAX_CONGRUENT_STORAGE_READS);
        let mut lemmas = Vec::new();
        for (index, (left_symbol, left_address, left_key)) in reads.iter().enumerate() {
            for (right_symbol, right_address, right_key) in &reads[index + 1..] {
                if left_symbol == right_symbol || left_address != right_address {
                    continue;
                }
                if let (Some(left), Some(right)) = (left_key.as_const(), right_key.as_const())
                    && left != right
                {
                    continue;
                }
                let cx = &mut self.cx;
                let same_slot = SymBoolExpr::eq(cx, left_key.clone(), right_key.clone());
                let left = SymExpr::get_var(cx, *left_symbol);
                let right = SymExpr::get_var(cx, *right_symbol);
                let same_value = SymBoolExpr::eq(cx, left, right);
                let different_slot = same_slot.not(cx);
                lemmas.push(SymBoolExpr::or(cx, vec![different_slot, same_value]));
            }
        }
        lemmas
    }

    /// Returns how many solver queries an equivalence check runs at once.
    fn solver_jobs(&self) -> usize {
        self.equivalence.map_or(1, |mode| mode.jobs)
    }

    /// Explores every path of one program and returns how each one ended.
    ///
    /// `symbol_offset` starts the program's fresh symbols apart from the other program's.
    fn explore_equivalence_side(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        env: &EquivalenceEnv,
        calldata: &SymbolicCalldata,
        code: &SymCode,
        symbol_offset: usize,
        completed_paths: &mut usize,
    ) -> Result<Vec<ProgramEnd>, SymbolicError> {
        let mut root = self.equivalence_root(input, env, calldata.clone(), code);
        root.next_symbol = root.next_symbol.max(symbol_offset);
        let mut side = SequenceCall {
            code: code.clone(),
            worklist: VecDeque::from([root]),
            deferred_worklist: VecDeque::new(),
        };
        let mut ends = Vec::new();
        while let Some(outcome) =
            self.execute_sequence_call_next(&input.executor, &mut side, completed_paths)?
        {
            self.check_timeout()?;
            let end = self.equivalence_end(&outcome);
            ends.push((end, outcome.state));
        }
        Ok(ends)
    }

    /// Solves the pending pair queries in order, at most [`Self::solver_jobs`] at a time, and
    /// returns the first counterexample.
    fn solve_equivalence_pairs(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        env: &EquivalenceEnv,
        calldata: &SymbolicCalldata,
        (ends_a, ends_b): (&[ProgramEnd], &[ProgramEnd]),
        pending: &mut Vec<(usize, usize, Vec<SymBoolExpr>, SymbolicVars)>,
    ) -> Result<Option<SymbolicEquivalenceCounterexample>, SymbolicError> {
        if pending.is_empty() {
            return Ok(None);
        }
        let mut batch = std::mem::take(pending);
        // The first satisfiable pair stops the rest of the batch; pairs it stopped run again only
        // when that pair's counterexample turns out to be spurious.
        while !batch.is_empty() {
            let queries = batch
                .iter()
                .map(|(_, _, constraints, replayable)| (constraints.clone(), replayable.clone()))
                .collect::<Vec<_>>();
            let results = self.solver.is_sat_batch(&mut self.cx, &queries, true);
            let mut stopped = Vec::new();
            for (pair, result) in batch.into_iter().zip(results) {
                let (index_a, index_b, constraints, _) = &pair;
                match result {
                    Ok(true) => {
                        let (_, state_a) = &ends_a[*index_a];
                        let (_, state_b) = &ends_b[*index_b];
                        let unknown_calls =
                            state_a.unknown_calls.len() + state_b.unknown_calls.len();
                        match self.materialize_equivalence(
                            input,
                            env,
                            calldata,
                            (state_a, state_b),
                            constraints,
                            unknown_calls,
                        ) {
                            Ok(found) => return Ok(Some(found)),
                            // Keccak refinement can show a satisfiable abstraction to be spurious.
                            Err(SymbolicError::Solver(reason)) if reason == PATH_BECAME_UNSAT => {}
                            Err(err) => return Err(err),
                        }
                    }
                    Ok(false) => {}
                    Err(SymbolicError::Solver(reason)) if reason == BATCH_STOPPED => {
                        stopped.push(pair);
                    }
                    Err(SymbolicError::SolverUnknown) => self.defer_solver_unknown(),
                    Err(err) => return Err(err),
                }
            }
            batch = stopped;
        }
        Ok(None)
    }

    fn equivalence_root(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        env: &EquivalenceEnv,
        calldata: SymbolicCalldata,
        code: &SymCode,
    ) -> PathState {
        let cx = &mut self.cx;
        let mut root = PathState::new(cx, input.address, input.sender, U256::ZERO, calldata, false);
        root.apply_executor_env(cx, &input.executor);
        root.world.set_storage_layout(self.config.storage_layout);
        root.world.clear_transaction_scoped_state();
        root.world.install_code(input.address, code.clone());
        root.world.enable_symbolic_accounts();
        root.world.alias_symbolic_address(env.caller.clone(), input.sender);

        root.frame.address_word = env.address.clone();
        root.frame.caller_word = env.caller.clone();
        root.frame.callvalue = env.value.clone();
        root.origin_word = env.origin.clone();
        root.gas_price = env.gas_price.clone();
        root.block.timestamp = env.timestamp.clone();
        root.block.number = env.number.clone();
        root.block.difficulty = env.prevrandao.clone();
        root.block.gaslimit = env.gas_limit.clone();
        root.block.basefee = env.basefee.clone();
        root.block.blob_basefee = env.blob_basefee.clone();
        root.block.chain_id = env.chain_id.clone();
        root.block.coinbase_word = Some(env.coinbase.clone());
        root.unknown_world = Some(env.world.clone());
        root.constraints.extend(env.constraints.iter().cloned());
        // Record every log so both sides' emitted events can be compared.
        root.recorded_logs = Some(Vec::new());

        // The sender pays the call value, which it must hold, to the programs' account.
        if env.value.as_const() != Some(U256::ZERO) {
            let sender = root.world.balance_word_for_address(cx, &input.executor, input.sender);
            let target = root.world.balance_word_for_address(cx, &input.executor, input.address);
            let can_pay = SymBoolExpr::cmp(cx, SymCmpOp::Uge, sender.clone(), env.value.clone());
            let max_value = SymExpr::constant(cx, U256::MAX);
            let room = SymExpr::binop(cx, SymBinOp::Sub, max_value, target.clone());
            let fits = SymBoolExpr::cmp(cx, SymCmpOp::Ule, env.value.clone(), room);
            root.constraints.push(can_pay);
            root.constraints.push(fits);
            let sender = SymExpr::binop(cx, SymBinOp::Sub, sender, env.value.clone());
            let target = SymExpr::binop(cx, SymBinOp::Add, target, env.value.clone());
            root.world.set_balance_word(input.sender, sender);
            root.world.set_balance_word(input.address, target);
        }
        root
    }

    fn equivalence_end(&mut self, outcome: &CallOutcome) -> EquivalenceEnd {
        match outcome.status {
            CallStatus::Success => {
                EquivalenceEnd { success: true, return_data: outcome.state.return_data.clone() }
            }
            // Assertion-style reverts are ordinary reverts here: only the revert data matters.
            CallStatus::Revert | CallStatus::Failure => {
                EquivalenceEnd { success: false, return_data: outcome.state.return_data.clone() }
            }
            CallStatus::ExceptionalHalt => {
                EquivalenceEnd { success: false, return_data: SymReturnData::empty(&mut self.cx) }
            }
        }
    }

    /// Returns the condition under which two completed paths are observably different.
    fn equivalence_mismatch(
        &mut self,
        executor: &ExecutorView<'_>,
        state_a: &mut PathState,
        end_a: &EquivalenceEnd,
        state_b: &mut PathState,
        end_b: &EquivalenceEnd,
    ) -> Result<SymBoolExpr, SymbolicError> {
        if end_a.success != end_b.success {
            return Ok(SymBoolExpr::constant(&mut self.cx, true));
        }
        let mut diffs = vec![bytes_mismatch(
            &mut self.cx,
            (&end_a.return_data.len_word, &end_a.return_data.bytes),
            (&end_b.return_data.len_word, &end_b.return_data.bytes),
        )];
        if !end_a.success {
            return Ok(SymBoolExpr::or(&mut self.cx, diffs));
        }

        let logs_a = state_a.recorded_logs.clone().unwrap_or_default();
        let logs_b = state_b.recorded_logs.clone().unwrap_or_default();
        if logs_a.len() != logs_b.len() {
            return Ok(SymBoolExpr::constant(&mut self.cx, true));
        }
        for (log_a, log_b) in logs_a.into_iter().zip(logs_b) {
            let (topics_a, len_a, data_a, emitter_a) = log_a.into_parts();
            let (topics_b, len_b, data_b, emitter_b) = log_b.into_parts();
            if emitter_a != emitter_b || topics_a.len() != topics_b.len() {
                return Ok(SymBoolExpr::constant(&mut self.cx, true));
            }
            for (topic_a, topic_b) in topics_a.iter().zip(topics_b.iter()) {
                let same = SymBoolExpr::eq(&mut self.cx, topic_a.clone(), topic_b.clone());
                diffs.push(same.not(&mut self.cx));
            }
            diffs.push(bytes_mismatch(&mut self.cx, (&len_a, &data_a), (&len_b, &data_b)));
        }

        // Both runs share one pre-state, so an account can only differ where one of them
        // changed it.
        for address in
            union(state_a.world.storage_write_addresses(), state_b.world.storage_write_addresses())
        {
            let keys = union(
                state_a.world.storage_write_keys(address),
                state_b.world.storage_write_keys(address),
            );
            for key in keys {
                let concrete = key.as_const();
                let left =
                    state_a.world.sload(&mut self.cx, executor, address, key.clone(), concrete)?;
                let right = state_b.world.sload(&mut self.cx, executor, address, key, concrete)?;
                diffs.push(SymBoolExpr::eq(&mut self.cx, left, right).not(&mut self.cx));
            }
        }
        for account in union(state_a.world.balance_addresses(), state_b.world.balance_addresses()) {
            let left = state_a.world.balance_word_for_address(&mut self.cx, executor, account);
            let right = state_b.world.balance_word_for_address(&mut self.cx, executor, account);
            diffs.push(SymBoolExpr::eq(&mut self.cx, left, right).not(&mut self.cx));
        }
        for account in union(state_a.world.nonce_addresses(), state_b.world.nonce_addresses()) {
            if state_a.world.nonce(executor, account)? != state_b.world.nonce(executor, account)? {
                return Ok(SymBoolExpr::constant(&mut self.cx, true));
            }
        }
        for account in union(
            state_a.world.created_account_addresses(),
            state_b.world.created_account_addresses(),
        ) {
            let code_a = state_a.world.extcode(&mut self.cx, executor, account)?;
            let code_b = state_b.world.extcode(&mut self.cx, executor, account)?;
            if code_a.len() != code_b.len() {
                return Ok(SymBoolExpr::constant(&mut self.cx, true));
            }
            let bytes_a = code_a.read_byte_exprs(&mut self.cx, 0, code_a.len());
            let bytes_b = code_b.read_byte_exprs(&mut self.cx, 0, code_b.len());
            for (left, right) in bytes_a.into_iter().zip(bytes_b) {
                diffs.push(SymBoolExpr::eq(&mut self.cx, left, right).not(&mut self.cx));
            }
        }
        Ok(SymBoolExpr::or(&mut self.cx, diffs))
    }

    /// Abstracts a call into unknown code: its success, return data and, for a state-changing
    /// call, the ether it sends back and the new state of unknown code are fresh symbols, tied
    /// across the two programs by [`Self::unknown_call_axioms`].
    #[expect(clippy::too_many_arguments)]
    pub(super) fn call_unknown_code(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        kind: CallKind,
        to: Address,
        caller: Address,
        target: SymExpr,
        value: SymExpr,
        input: SymBytes,
        input_len: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        let mode =
            self.equivalence.expect("unknown calls are only abstracted in equivalence checks");
        let changes_state = match kind {
            CallKind::StaticCall => false,
            CallKind::Call if state.is_static => false,
            CallKind::Call if mode.promise_no_reent => true,
            CallKind::Call => {
                return Err(SymbolicError::Unsupported(
                    "CALL into unknown code, which may re-enter (pass --promise-no-reent)",
                ));
            }
            CallKind::DelegateCall | CallKind::CallCode => {
                return Err(SymbolicError::Unsupported(
                    "DELEGATECALL or CALLCODE into unknown code",
                ));
            }
        };
        let opcode = if changes_state { opcode::CALL } else { opcode::STATICCALL };
        let max = mode.max_returndata;
        let cx = &mut self.cx;
        let world =
            state.unknown_world.clone().unwrap_or_else(|| SymExpr::var(cx, "unknown_world_0"));
        let caller_balance = state.world.balance_word_for_address(cx, executor, caller);
        let caller_world = state.world.clone();

        let success = state.fresh_word(cx, "unknown_call_success");
        state.constraints.push(SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ule, &success, U256::ONE));
        let output_len = state.fresh_word(cx, "unknown_call_output_len");
        state.constraints.push(SymBoolExpr::cmp_word_const(
            cx,
            SymCmpOp::Ule,
            &output_len,
            U256::from(max),
        ));
        let words = (0..max.div_ceil(32))
            .map(|_| {
                let word = state.fresh_word(cx, "unknown_call_output");
                SymBytes::word(cx, word)
            })
            .collect::<Vec<_>>();
        let output = SymBytes::concat(cx, words).slice_concrete(cx, 0, max);
        let output = SymBytes::sized(cx, output, output_len.clone(), max);

        let (world_after, ether_in) = if changes_state {
            let world_after = state.fresh_word(cx, "unknown_world");
            let ether_in = state.fresh_word(cx, "unknown_call_ether_in");
            // The callee gets the value if the call succeeds, and may send the caller ether
            // without running the caller's code, such as by self-destructing.
            let moved = SymExpr::binop(cx, SymBinOp::Mul, value.clone(), success.clone());
            let left = SymExpr::binop(cx, SymBinOp::Sub, caller_balance.clone(), moved.clone());
            let max_value = SymExpr::constant(cx, U256::MAX);
            let room = SymExpr::binop(cx, SymBinOp::Sub, max_value, left.clone());
            state.constraints.push(SymBoolExpr::cmp(cx, SymCmpOp::Ule, ether_in.clone(), room));
            if to != caller {
                let callee = state.world.balance_word_for_address(cx, executor, to);
                let callee = SymExpr::binop(cx, SymBinOp::Add, callee, moved);
                state.world.set_balance_word(to, callee);
            }
            let balance = SymExpr::binop(cx, SymBinOp::Add, left, ether_in.clone());
            state.world.set_balance_word(caller, balance);
            (Some(world_after), Some(ether_in))
        } else {
            (None, None)
        };

        state.unknown_calls.push(UnknownCall {
            opcode,
            caller,
            world: world.clone(),
            target,
            value,
            input_len,
            input,
            caller_world,
            caller_balance,
            success: success.clone(),
            output_len: output_len.clone(),
            output: output.clone(),
            world_after: world_after.clone(),
            ether_in,
        });
        state.unknown_world = Some(world_after.unwrap_or(world));
        state.return_data = SymReturnData { len_word: output_len, bytes: output };
        state.copy_call_output_offset(cx, out_offset, out_size)?;
        state.stack.push(success)?;
        Ok(StepOutcome::Continue)
    }

    /// Returns the constraints that make unknown code one fixed function: the n-th call of A and
    /// the n-th call of B with the same inputs get the same answer.
    fn unknown_call_axioms(
        &mut self,
        executor: &ExecutorView<'_>,
        calls_a: &[UnknownCall],
        calls_b: &[UnknownCall],
    ) -> Result<Vec<SymBoolExpr>, SymbolicError> {
        if calls_a.is_empty() || calls_b.is_empty() {
            return Ok(Vec::new());
        }
        let ordinals_a = self.unknown_call_ordinals(executor, calls_a)?;
        let ordinals_b = self.unknown_call_ordinals(executor, calls_b)?;
        let mut axioms = Vec::new();
        for (a, ordinal_a) in calls_a.iter().zip(&ordinals_a) {
            for (b, ordinal_b) in calls_b.iter().zip(&ordinals_b) {
                let same_inputs = self.unknown_inputs_equal(executor, a, b)?;
                if same_inputs.as_const() == Some(false) {
                    continue;
                }
                let cx = &mut self.cx;
                let same_ordinal = SymBoolExpr::eq(cx, ordinal_a.clone(), ordinal_b.clone());
                let same = SymBoolExpr::and(cx, vec![same_inputs, same_ordinal]);
                // Bytes past the returned length are never read, so the answers may as well
                // agree on them too, which keeps this cheap.
                let mut outputs = vec![
                    SymBoolExpr::eq(cx, a.success.clone(), b.success.clone()),
                    SymBoolExpr::eq(cx, a.output_len.clone(), b.output_len.clone()),
                    words_equal(cx, &a.output, &b.output),
                ];
                if let (Some(left), Some(right)) = (&a.world_after, &b.world_after) {
                    outputs.push(SymBoolExpr::eq(cx, left.clone(), right.clone()));
                }
                if let (Some(left), Some(right)) = (&a.ether_in, &b.ether_in) {
                    outputs.push(SymBoolExpr::eq(cx, left.clone(), right.clone()));
                }
                let outputs = SymBoolExpr::and(cx, outputs);
                let differs = same.not(cx);
                axioms.push(SymBoolExpr::or(cx, vec![differs, outputs]));
            }
        }
        Ok(axioms)
    }

    /// Returns, for each call, how many earlier calls of the same program had the same inputs.
    fn unknown_call_ordinals(
        &mut self,
        executor: &ExecutorView<'_>,
        calls: &[UnknownCall],
    ) -> Result<Vec<SymExpr>, SymbolicError> {
        let mut ordinals = Vec::with_capacity(calls.len());
        for (idx, call) in calls.iter().enumerate() {
            let mut ordinal = SymExpr::zero(&mut self.cx);
            for earlier in &calls[..idx] {
                let same = self.unknown_inputs_equal(executor, earlier, call)?;
                if same.as_const() == Some(false) {
                    continue;
                }
                let cx = &mut self.cx;
                let one = SymExpr::one(cx);
                let zero = SymExpr::zero(cx);
                let count = SymExpr::ite(cx, same, one, zero);
                ordinal = SymExpr::binop(cx, SymBinOp::Add, ordinal, count);
            }
            ordinals.push(ordinal);
        }
        Ok(ordinals)
    }

    /// Returns the condition under which two calls into unknown code have the same inputs,
    /// including the caller's storage and balance, which the callee may read.
    fn unknown_inputs_equal(
        &mut self,
        executor: &ExecutorView<'_>,
        a: &UnknownCall,
        b: &UnknownCall,
    ) -> Result<SymBoolExpr, SymbolicError> {
        if a.opcode != b.opcode || a.caller != b.caller {
            return Ok(SymBoolExpr::constant(&mut self.cx, false));
        }
        let cx = &mut self.cx;
        let mut same = vec![
            SymBoolExpr::eq(cx, a.world.clone(), b.world.clone()),
            SymBoolExpr::eq(cx, a.target.clone(), b.target.clone()),
            SymBoolExpr::eq(cx, a.value.clone(), b.value.clone()),
            SymBoolExpr::eq(cx, a.caller_balance.clone(), b.caller_balance.clone()),
            bytes_mismatch(cx, (&a.input_len, &a.input), (&b.input_len, &b.input)).not(cx),
        ];
        let mut world_a = a.caller_world.clone();
        let mut world_b = b.caller_world.clone();
        for key in union(world_a.storage_write_keys(a.caller), world_b.storage_write_keys(b.caller))
        {
            let concrete = key.as_const();
            let left = world_a.sload(&mut self.cx, executor, a.caller, key.clone(), concrete)?;
            let right = world_b.sload(&mut self.cx, executor, b.caller, key, concrete)?;
            same.push(SymBoolExpr::eq(&mut self.cx, left, right));
        }
        Ok(SymBoolExpr::and(&mut self.cx, same))
    }

    fn materialize_equivalence(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        env: &EquivalenceEnv,
        calldata: &SymbolicCalldata,
        (state_a, state_b): (&PathState, &PathState),
        constraints: &[SymBoolExpr],
        unknown_calls: usize,
    ) -> Result<SymbolicEquivalenceCounterexample, SymbolicError> {
        // Storage either program read must replay, so the world carries both programs' slots.
        let mut world = state_b.world.clone();
        world.merge_replay_metadata_from(&state_a.world);
        let replayable_storage = world.replay_storage_symbols();
        let model = self.solver.model_with_replayable_storage(
            &mut self.cx,
            constraints,
            &replayable_storage,
        )?;
        let calldata_bytes = Bytes::from(calldata.model_to_bytes(&mut self.cx, &model)?);
        let args = match input.function {
            Some(_) => calldata.model_to_args(&mut self.cx, &model)?,
            None => Vec::new(),
        };
        let storage = world.replay_storage_assignments(&model)?;
        let word = |expr: &SymExpr| -> Result<U256, SymbolicError> {
            Ok(expr.eval_model_if_complete(&model)?.unwrap_or_default())
        };
        let address =
            |expr: &SymExpr| -> Result<Address, SymbolicError> { Ok(word_to_address(word(expr)?)) };
        let mut balances = Vec::new();
        for account in union(
            vec![input.sender, input.address],
            union(state_a.world.balance_addresses(), state_b.world.balance_addresses()),
        ) {
            let balance = SymExpr::var(&mut self.cx, &format!("balance_{account:?}"));
            balances.push((account, word(&balance)?));
        }
        let env = SymbolicEquivalenceEnv {
            address: address(&env.address)?,
            caller: address(&env.caller)?,
            origin: address(&env.origin)?,
            value: word(&env.value)?,
            timestamp: word(&env.timestamp)?,
            number: word(&env.number)?,
            prevrandao: word(&env.prevrandao)?,
            gas_limit: word(&env.gas_limit)?,
            basefee: word(&env.basefee)?,
            blob_basefee: word(&env.blob_basefee)?,
            chain_id: word(&env.chain_id)?,
            coinbase: address(&env.coinbase)?,
            gas_price: word(&env.gas_price)?,
            balances,
        };
        Ok(SymbolicEquivalenceCounterexample {
            calldata: calldata_bytes,
            args,
            storage,
            env,
            unknown_calls,
        })
    }
}

/// Appends the items of `right` missing from `left`, keeping first-seen order.
fn union<T: PartialEq>(mut left: Vec<T>, right: Vec<T>) -> Vec<T> {
    for item in right {
        if !left.contains(&item) {
            left.push(item);
        }
    }
    left
}

/// Returns the condition under which two byte buffers differ in length or in a byte in range.
fn bytes_mismatch(
    cx: &mut SymCx,
    (len_a, a): (&SymExpr, &SymBytes),
    (len_b, b): (&SymExpr, &SymBytes),
) -> SymBoolExpr {
    let mut diffs = vec![SymBoolExpr::eq(cx, len_a.clone(), len_b.clone()).not(cx)];
    for idx in 0..a.len().max(b.len()) {
        let left = a.byte(cx, idx);
        let right = b.byte(cx, idx);
        let differ = SymBoolExpr::eq(cx, left, right).not(cx);
        if differ.as_const() == Some(false) {
            continue;
        }
        let in_range = SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ugt, len_a, U256::from(idx));
        diffs.push(SymBoolExpr::and(cx, vec![in_range, differ]));
    }
    SymBoolExpr::or(cx, diffs)
}

/// Returns the condition under which the words two buffers were cut from are all equal, which
/// implies equal bytes in range and stays cheap for word-built buffers.
fn words_equal(cx: &mut SymCx, a: &SymBytes, b: &SymBytes) -> SymBoolExpr {
    let (a, b) = (a.without_size(), b.without_size());
    let mut same = Vec::new();
    for start in (0..a.len().max(b.len())).step_by(32) {
        let left = a.word_at(cx, start);
        let right = b.word_at(cx, start);
        if left != right {
            same.push(SymBoolExpr::eq(cx, left, right));
        }
    }
    SymBoolExpr::and(cx, same)
}
