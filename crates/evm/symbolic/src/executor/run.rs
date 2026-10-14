use super::{
    loops::{MAX_EXACT_ITERATIONS, MAX_UNSUMMARIZED_ITERATIONS},
    *,
};
use std::cmp::Reverse;

impl SymbolicExecutor {
    /// Creates a symbolic executor from Foundry's symbolic configuration.
    ///
    /// The configured solver command is not executed here. Solver availability is
    /// checked by [`Self::run`] so construction remains cheap and side-effect free.
    ///
    /// The executor owns an isolated solver backend and symbolic world overlay. Create
    /// a fresh executor when a caller needs independent solver query accounting.
    pub fn new(config: SymbolicConfig) -> Self {
        let solver = SmtLibSubprocessSolver::from_config(&config);
        Self {
            config,
            cx: SymCx::new(),
            solver,
            deferred_incomplete: None,
            deadline: None,
            nested_deferred_mode: DeferredPathMode::Skip,
            stateless_retry_safe: true,
            unknown_callees_of: None,
            symbolic_immutables: Vec::new(),
            storage_max_lengths: Vec::new(),
            loop_summaries: false,
            loop_epochs: 0,
            function_jumps: Default::default(),
            finished_gas: Vec::new(),
            finished_length_violations: Vec::new(),
        }
    }

    fn reset_run_state(&mut self, use_wall_clock_deadline: bool) {
        self.deferred_incomplete = None;
        self.nested_deferred_mode = DeferredPathMode::Skip;
        self.stateless_retry_safe = true;
        self.deadline = if use_wall_clock_deadline {
            self.config
                .timeout
                .filter(|seconds| *seconds > 0)
                .map(|seconds| Instant::now() + Duration::from_secs(seconds.into()))
        } else {
            None
        };
    }

    pub(super) fn check_timeout(&self) -> Result<(), SymbolicError> {
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline
        {
            return Err(SymbolicError::Timeout(self.config.timeout.unwrap_or_default()));
        }
        Ok(())
    }

    /// Defers an incomplete result until all counterexample-producing modeled paths are explored.
    pub(super) fn defer_incomplete(&mut self, reason: &'static str) {
        if self
            .deferred_incomplete
            .is_none_or(|reason| reason == DeferredIncomplete::HardArithmetic)
        {
            self.deferred_incomplete = Some(DeferredIncomplete::Unsupported(reason));
        }
    }

    /// Defers a solver-unknown result while continuing with decidable sibling paths.
    pub(super) fn defer_solver_unknown(&mut self) {
        if self
            .deferred_incomplete
            .is_none_or(|reason| reason == DeferredIncomplete::HardArithmetic)
        {
            self.deferred_incomplete = Some(DeferredIncomplete::SolverUnknown);
        }
    }

    /// Defers an incomplete result for a hard-arithmetic branch skipped by nested execution.
    pub(super) fn defer_hard_arithmetic(&mut self) {
        self.deferred_incomplete.get_or_insert(DeferredIncomplete::HardArithmetic);
    }

    pub(super) fn is_sat_with_state(
        &mut self,
        state: &PathState,
        constraints: &[SymBoolExpr],
    ) -> Result<bool, SymbolicError> {
        let replayable_storage = state.world.replay_storage_symbols();
        self.solver.is_sat_with_replayable_storage(&mut self.cx, constraints, &replayable_storage)
    }

    /// Checks branch feasibility, recording solver-unknown as an incomplete proof path.
    pub(super) fn branch_is_sat_or_defer(
        &mut self,
        state: &PathState,
        constraints: &[SymBoolExpr],
    ) -> Result<bool, SymbolicError> {
        match self.is_sat_with_state(state, constraints) {
            Ok(feasible) => Ok(feasible),
            Err(SymbolicError::SolverUnknown) => {
                self.defer_solver_unknown();
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Returns any deferred incomplete reason.
    fn deferred_incomplete(&self) -> Option<(SymbolicStopReason, String)> {
        match self.deferred_incomplete? {
            DeferredIncomplete::Unsupported(reason) => Some((
                SymbolicStopReason::Stuck,
                format!("unsupported symbolic execution feature: {reason}"),
            )),
            DeferredIncomplete::SolverUnknown => {
                Some((SymbolicStopReason::Timeout, "solver returned unknown".to_string()))
            }
            DeferredIncomplete::HardArithmetic => Some((
                SymbolicStopReason::Timeout,
                "nested hard arithmetic branch requires deferred SMT solving".to_string(),
            )),
        }
    }

    /// Executes one function symbolically against an already-deployed test contract.
    ///
    /// The input executor supplies the deployed bytecode, storage backend, caller, and
    /// target address established by the normal forge test setup flow. This method
    /// does not mutate the concrete executor and does not replay failures itself; when
    /// it returns [`SymbolicRunResult::Counterexample`], callers should replay the
    /// returned arguments through the concrete executor before reporting the failure.
    ///
    /// Unsupported opcodes, unsupported ABI types, missing solver support, and resource
    /// limit exhaustion are reported as [`SymbolicRunResult::Incomplete`].
    ///
    /// Ordinary Solidity `require` reverts prune the current path. Assertion failures,
    /// forge-std assertion reverts, and DSTest failure signals are reported as
    /// counterexample candidates when the failing path is satisfiable.
    pub fn run<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> SymbolicRunResult {
        self.execute_run(input, None)
    }

    /// Returns the gas bounds and length violations of the successful paths the last run
    /// finished.
    ///
    /// After an incomplete run, they cover only the paths explored before it stopped. A gas
    /// bound then says nothing about the other paths, but its witness is still an input to
    /// replay, and a length violation is still an input to confirm.
    pub fn finished_paths(&self) -> (&[SymbolicGasBound], &[SymbolicLengthViolation]) {
        (&self.finished_gas, &self.finished_length_violations)
    }

    /// Searches for concrete inputs that complete after reaching a requested branch outcome.
    ///
    /// Unlike [`Self::run`], this retains replay candidates independently of the proof result, so a
    /// later unsupported path or resource limit does not discard an input found on an earlier
    /// completed path. The caller must concretely replay every candidate before using it.
    pub fn search_branch_target<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> SymbolicBranchTargetSearchResult {
        let mut candidates = Vec::new();
        if input.branch_target.is_none() {
            return SymbolicBranchTargetSearchResult {
                candidates,
                execution: SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::Error,
                    reason: "branch target search requires a branch target".to_string(),
                    stats: SymbolicStats::default(),
                },
            };
        }

        let execution = self.execute_run(input, Some(&mut candidates));
        if let SymbolicRunResult::Counterexample { args, calldata, .. } = &execution {
            candidates.insert(
                0,
                SymbolicConcreteInput { args: args.clone(), calldata: calldata.clone() },
            );
        }
        SymbolicBranchTargetSearchResult { candidates, execution }
    }

    fn execute_run<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
        mut branch_candidates: Option<&mut Vec<SymbolicConcreteInput>>,
    ) -> SymbolicRunResult {
        // A gas check is one transaction: bound its wall-clock time like invariant exploration.
        self.reset_run_state(input.transaction_gas.is_some());
        self.finished_gas.clear();
        self.finished_length_violations.clear();
        self.solver.clear_context_caches();
        self.cx = SymCx::new();
        if let Err(err) = self.solver.check_available() {
            return SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        let mut completed_paths = 0;
        let first =
            match self.run_inner(&input, branch_candidates.as_deref_mut(), &mut completed_paths) {
                Ok(result) => result,
                Err(err) => {
                    return SymbolicRunResult::Incomplete {
                        kind: err.stop_reason(),
                        reason: err.to_string(),
                        stats: self.stats_with_paths(completed_paths),
                    };
                }
            };

        // Preserve the cheap-path priority of the first pass. Only a deterministic stateless
        // proof retries after nested hard arithmetic was its remaining limitation.
        let retry_nested_deferred = branch_candidates.is_none()
            && self.stateless_retry_safe
            && matches!(self.deferred_incomplete, Some(DeferredIncomplete::HardArithmetic))
            && matches!(
                &first,
                SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::RevertAll | SymbolicStopReason::Timeout,
                    ..
                }
            );
        if !retry_nested_deferred {
            return first;
        }

        if let Err(err) = self.check_timeout() {
            return SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.stats_with_paths(completed_paths),
            };
        }

        self.deferred_incomplete = None;
        self.nested_deferred_mode = DeferredPathMode::Yield;
        let prioritized = match self.run_inner(&input, None, &mut completed_paths) {
            Ok(result) => result,
            Err(err) => {
                return SymbolicRunResult::Incomplete {
                    kind: err.stop_reason(),
                    reason: err.to_string(),
                    stats: self.stats_with_paths(completed_paths),
                };
            }
        };
        let drain_nested_deferred = self.stateless_retry_safe
            && matches!(self.deferred_incomplete, Some(DeferredIncomplete::HardArithmetic))
            && matches!(
                &prioritized,
                SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::RevertAll | SymbolicStopReason::Timeout,
                    ..
                }
            );
        if !drain_nested_deferred {
            return prioritized;
        }

        if let Err(err) = self.check_timeout() {
            return SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.stats_with_paths(completed_paths),
            };
        }

        self.deferred_incomplete = None;
        self.nested_deferred_mode = DeferredPathMode::Drain;
        match self.run_inner(&input, None, &mut completed_paths) {
            Ok(result) => result,
            Err(err) => SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.stats_with_paths(completed_paths),
            },
        }
    }

    /// Returns corpus seed indexes that can be modeled by at least one symbolic calldata variant.
    pub fn modeled_corpus_seed_indexes(
        config: &SymbolicConfig,
        function: &Function,
        corpus_seeds: &[SymbolicConcreteInput],
    ) -> Result<Vec<usize>, SymbolicError> {
        let mut cx = SymCx::new();
        let variants = SymbolicCalldata::variants(function, config, &mut cx)?;
        let mut modeled = vec![false; corpus_seeds.len()];
        for calldata in &variants {
            for (idx, seed) in corpus_seeds.iter().enumerate() {
                if !modeled[idx] && calldata.seed_model(&mut cx, seed).is_some() {
                    modeled[idx] = true;
                }
            }
        }
        Ok(modeled
            .into_iter()
            .enumerate()
            .filter_map(|(idx, modeled)| modeled.then_some(idx))
            .collect())
    }

    /// Executes a bounded symbolic invariant call sequence.
    ///
    /// Each sequence step chooses from the concrete target functions and senders supplied by
    /// Foundry's invariant target discovery. Arguments are generated through the same symbolic ABI
    /// model used by stateless symbolic tests, and the symbolic world state is preserved between
    /// steps. Returned counterexamples must still be replayed by the caller before reporting.
    ///
    /// The configured invariant depth limits the number of target calls explored before the
    /// invariant is checked. A depth of zero checks only the invariant against setup state.
    pub fn run_invariant<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> SymbolicInvariantRunResult {
        self.reset_run_state(true);
        self.solver.clear_context_caches();
        self.cx = SymCx::new();
        if let Err(err) = self.solver.check_available() {
            return SymbolicInvariantRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        match self.run_invariant_inner(input) {
            Ok(result) => result,
            Err(err) => SymbolicInvariantRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.solver.stats(),
            },
        }
    }

    /// Searches for invariant-breaking inputs after one symbolic handler call.
    ///
    /// This is a best-effort candidate search from a concrete state. Returned candidates are
    /// unconfirmed until the caller replays them concretely, and an empty result does not prove
    /// any invariant.
    pub fn search_invariant_candidates<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantCandidateInput<'_, FEN>,
    ) -> SymbolicInvariantCandidateSearchResult {
        self.reset_run_state(true);
        self.solver.clear_context_caches();
        self.cx = SymCx::new();
        if let Err(error) = self.solver.check_available() {
            return SymbolicInvariantCandidateSearchResult {
                candidates: Vec::new(),
                limitation: Some(error.into()),
            };
        }

        let mut candidates = Vec::new();
        let mut limitation = None;
        if let Err(error) =
            self.search_invariant_candidates_inner(&input, &mut candidates, &mut limitation)
        {
            limitation = Some(error.into());
        }
        // Deferred hard-arithmetic branches are now sent to SMT before candidate search finishes.
        // Only branches that nested execution could not escalate remain incomplete.
        if limitation.is_none()
            && let Some((kind, reason)) = self.deferred_incomplete()
        {
            limitation = Some(SymbolicInvariantSearchLimitation { kind, reason });
        }

        SymbolicInvariantCandidateSearchResult { candidates, limitation }
    }

    pub(super) fn run_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: &SymbolicRunInput<'_, FEN>,
        mut branch_candidates: Option<&mut Vec<SymbolicConcreteInput>>,
        completed_paths: &mut usize,
    ) -> Result<SymbolicRunResult, SymbolicError> {
        let account = input
            .executor
            .backend()
            .basic_ref(input.target)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(input.target))?;
        let bytecode = account.code.ok_or(SymbolicError::MissingCode(input.target))?;
        let mut code = SymCode::from_bytecode(&mut self.cx, &bytecode);
        self.symbolic_immutables = input
            .transaction_gas
            .as_ref()
            .map(|transaction| transaction.immutables.clone())
            .unwrap_or_default();
        if !self.symbolic_immutables.is_empty() {
            code = code.with_symbolic_words(&mut self.cx, &self.symbolic_immutables);
        }
        self.unknown_callees_of = input
            .transaction_gas
            .as_ref()
            .is_some_and(|transaction| transaction.unknown_callees)
            .then_some(input.target);
        self.storage_max_lengths = input
            .transaction_gas
            .as_ref()
            .map(|transaction| transaction.max_lengths.clone())
            .unwrap_or_default();
        // Summaries need every callee to be unknown, so a loop cannot change other accounts.
        self.loop_summaries = self.unknown_callees_of.is_some();
        self.function_jumps = input
            .transaction_gas
            .as_ref()
            .map(|transaction| transaction.function_jumps.clone())
            .unwrap_or_default();
        let mut roots = Vec::new();
        for calldata in SymbolicCalldata::variants(input.function, &self.config, &mut self.cx)? {
            let corpus_seed_models = input
                .corpus_seeds
                .iter()
                .filter_map(|seed| calldata.seed_model(&mut self.cx, seed).map(Arc::new))
                .collect();
            let mut root = PathState::new(
                &mut self.cx,
                input.target,
                input.sender,
                input.value,
                calldata,
                input.ffi_enabled,
            );
            root.set_corpus_seed_models(corpus_seed_models);
            root.set_branch_target(input.branch_target);
            root.apply_executor_env(&mut self.cx, input.executor);
            root.world.set_storage_layout(self.config.storage_layout);
            root.world.clear_transaction_scoped_state();
            if input.transaction_gas.is_some() {
                // Any account can send the transaction: the sender is symbolic.
                let sender = SymExpr::var(&mut self.cx, TRANSACTION_SENDER);
                root.constraints.push(SymBoolExpr::cmp_word_const(
                    &mut self.cx,
                    SymCmpOp::Ult,
                    &sender,
                    U256::ONE << 160,
                ));
                root.frame.caller_word = sender.clone();
                root.origin_word = sender.clone();
                let calldata = root.frame.calldata.materialize(&mut self.cx);
                root.gas.begin_isolated_call(TRANSACTION_DEPTH, Some(input.target), calldata, 0);
                root.gas.access_account_word(sender);
                if self.unknown_callees_of.is_some() {
                    // Any storage state comes with any balance.
                    let balance = SymExpr::var(&mut self.cx, "target_balance");
                    root.world.set_balance_word(input.target, balance);
                }
                for &(slot, max) in &self.storage_max_lengths {
                    let key = SymExpr::constant(&mut self.cx, slot);
                    let length = root.world.sload(
                        &mut self.cx,
                        input.executor,
                        input.target,
                        key,
                        Some(slot),
                    )?;
                    root.constraints.push(SymBoolExpr::cmp_word_const(
                        &mut self.cx,
                        SymCmpOp::Ule,
                        &length,
                        U256::from(max),
                    ));
                }
            }
            roots.push(root);
        }
        order_roots_by_corpus_seed_count(&mut roots, self.config.exploration_order);
        let mut worklist = roots.into_iter().collect::<VecDeque<_>>();
        let mut deferred_worklist = VecDeque::new();
        let mut reverted_paths = 0usize;
        let mut normal_paths = 0usize;
        let mut success_input = None;
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(next) = self.pop_next_feasible_path(
            &mut worklist,
            &mut deferred_worklist,
            DeferredPathMode::Drain,
        )? {
            let mut state = next.state;
            if *completed_paths >= path_limit {
                debug!(
                    completed_paths = *completed_paths,
                    path_limit, "symbolic path limit reached"
                );
                return Ok(SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::Stuck,
                    reason: format!("symbolic path limit exceeded ({path_limit})"),
                    stats: self.stats_with_paths(*completed_paths),
                });
            }
            if std::mem::take(&mut state.pending_storage_hook_revert) {
                self.collect_branch_candidate(
                    branch_candidates.as_deref_mut(),
                    input.function,
                    &state,
                )?;
                *completed_paths += 1;
                reverted_paths += 1;
                continue;
            }
            let _path_span = trace_span!(
                "symbolic_path",
                completed_paths = *completed_paths,
                worklist_size = worklist.len()
            )
            .entered();
            trace!(
                completed_paths = *completed_paths,
                worklist_size = worklist.len(),
                "exploring symbolic path"
            );

            loop {
                self.check_timeout()?;
                if let Some(head) = state.back_edge.take() {
                    if let Some(&reason) = state.unsummarized_heads.get(&head) {
                        // A loop without a summary runs as before, up to a fixed count.
                        if let Some(reason) = reason {
                            state.unsummarized_iterations += 1;
                            if state.unsummarized_iterations > MAX_UNSUMMARIZED_ITERATIONS {
                                return Ok(SymbolicRunResult::Incomplete {
                                    kind: SymbolicStopReason::Stuck,
                                    reason: format!("loop at pc {head} is not bounded: {reason}"),
                                    stats: self.stats_with_paths(*completed_paths),
                                });
                            }
                        }
                    } else {
                        match self.summarize_loop(
                            input.executor,
                            &code,
                            input.target,
                            &state,
                            completed_paths,
                        )? {
                            // Running a few iterations is exact and terminates.
                            Ok(bound) if bound.iterations <= MAX_EXACT_ITERATIONS => {
                                state.unsummarized_heads.insert(head, None);
                            }
                            Ok(bound) => {
                                self.finish_summarized_path(input, &mut state, bound)?;
                                self.raise_gas_bounds(input.function, &state)?;
                                *completed_paths += 1;
                                normal_paths += 1;
                                break;
                            }
                            Err(reason) => {
                                debug!(head, reason, "loop not summarized");
                                state.unsummarized_heads.insert(head, Some(reason));
                            }
                        }
                    }
                }
                if state.depth >= depth_limit {
                    debug!(depth = state.depth, depth_limit, "symbolic depth limit reached");
                    return Ok(SymbolicRunResult::Incomplete {
                        kind: SymbolicStopReason::Stuck,
                        reason: format!("symbolic depth limit exceeded ({depth_limit})"),
                        stats: self.stats_with_paths(*completed_paths),
                    });
                }
                state.depth += 1;

                let outcome = match code.opcode(&mut self.cx, state.pc)? {
                    Some(op) => {
                        let _step_span = trace_span!("symbolic_step", pc = state.pc, op).entered();
                        self.step(
                            input.executor,
                            &code,
                            code.jump_table(),
                            &mut state,
                            &mut worklist,
                            completed_paths,
                            op,
                        )?
                    }
                    None => StepOutcome::Halt,
                };
                match outcome {
                    StepOutcome::Continue => {}
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                    StepOutcome::Revert => {
                        self.collect_branch_candidate(
                            branch_candidates.as_deref_mut(),
                            input.function,
                            &state,
                        )?;
                        *completed_paths += 1;
                        reverted_paths += 1;
                        break;
                    }
                    StepOutcome::Halt if state.expectations_satisfied() => {
                        let candidate = self.collect_branch_candidate(
                            branch_candidates.as_deref_mut(),
                            input.function,
                            &state,
                        )?;
                        if input.collect_success_input
                            && state.satisfies_branch_target()
                            && state.can_materialize_seed()
                            && success_input.as_ref().is_none_or(|(depth, _)| state.depth > *depth)
                        {
                            let input = match candidate {
                                Some(input) => input,
                                None => self.materialize_root_input(input.function, &state)?,
                            };
                            success_input = Some((state.depth, input));
                        }
                        if input.transaction_gas.is_some() {
                            self.measure_transaction(input.executor, &mut state, input.function)?;
                            self.check_max_lengths(
                                input.executor,
                                input.target,
                                input.function,
                                &mut state,
                            )?;
                        }
                        self.raise_gas_bounds(input.function, &state)?;
                        *completed_paths += 1;
                        normal_paths += 1;
                        break;
                    }
                    StepOutcome::Halt | StepOutcome::ExceptionalHalt | StepOutcome::Failure => {
                        if !state.satisfies_branch_target() {
                            *completed_paths += 1;
                            break;
                        }
                        debug!(
                            constraint_count = state.constraints.len(),
                            "materializing counterexample from solver model"
                        );
                        let SymbolicConcreteInput { args, calldata } =
                            self.materialize_hinted_root_model(input.function, &state)?.0;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata,
                            stats: self.stats_with_paths(*completed_paths + 1),
                        });
                    }
                }
            }
        }

        if normal_paths == 0 && reverted_paths > 0 {
            debug!(completed_paths = *completed_paths, "all symbolic paths reverted");
            return Ok(SymbolicRunResult::Incomplete {
                kind: SymbolicStopReason::RevertAll,
                reason: "all symbolic paths reverted".to_string(),
                stats: self.stats_with_paths(*completed_paths),
            });
        }

        if let Some((kind, reason)) = self.deferred_incomplete() {
            return Ok(SymbolicRunResult::Incomplete {
                kind,
                reason,
                stats: self.stats_with_paths(*completed_paths),
            });
        }

        if normal_paths == 0 {
            return Ok(SymbolicRunResult::Incomplete {
                kind: SymbolicStopReason::Stuck,
                reason: "no successful symbolic paths".to_string(),
                stats: self.stats_with_paths(*completed_paths),
            });
        }

        debug!(completed_paths = *completed_paths, "symbolic execution safe");
        Ok(SymbolicRunResult::Safe {
            stats: self.stats_with_paths(*completed_paths),
            success_input: success_input.map(|(_, input)| input),
            gas: self.finished_gas.clone(),
            length_violations: self.finished_length_violations.clone(),
        })
    }

    fn collect_branch_candidate(
        &mut self,
        candidates: Option<&mut Vec<SymbolicConcreteInput>>,
        function: &Function,
        state: &PathState,
    ) -> Result<Option<SymbolicConcreteInput>, SymbolicError> {
        let Some(candidates) = candidates else {
            return Ok(None);
        };
        if !state.satisfies_branch_target() || !state.can_materialize_seed() {
            return Ok(None);
        }

        let input = self.materialize_root_input(function, state)?;
        candidates.push(input.clone());
        Ok(Some(input))
    }

    /// Materializes the root call input for `state` from a solver model.
    pub(super) fn materialize_root_input(
        &mut self,
        function: &Function,
        state: &PathState,
    ) -> Result<SymbolicConcreteInput, SymbolicError> {
        self.materialize_root_model(function, state).map(|(input, _)| input)
    }

    /// Materializes the root call input for `state`, preferring a model that also satisfies the
    /// gas meter's witness hints, so a gas witness or counterexample reaches its calldata pricing.
    pub(super) fn materialize_hinted_root_model(
        &mut self,
        function: &Function,
        state: &PathState,
    ) -> Result<(SymbolicConcreteInput, SymbolicModel), SymbolicError> {
        if !state.gas.witness_hints().is_empty() {
            let mut hinted = state.clone();
            hinted.constraints.extend(state.gas.witness_hints().iter().cloned());
            if let Ok(materialized) = self.materialize_root_model(function, &hinted) {
                return Ok(materialized);
            }
        }
        self.materialize_root_model(function, state)
    }

    /// Materializes the root call input for `state` and returns the solver model it came from.
    pub(super) fn materialize_root_model(
        &mut self,
        function: &Function,
        state: &PathState,
    ) -> Result<(SymbolicConcreteInput, SymbolicModel), SymbolicError> {
        let calldata = state
            .root_calldata
            .as_ref()
            .ok_or(SymbolicError::Unsupported("missing root symbolic calldata"))?;
        let replayable_storage = state.world.replay_storage_symbols();
        let model = self.solver.model_with_replayable_storage(
            &mut self.cx,
            &state.constraints,
            &replayable_storage,
        )?;
        let args = calldata.model_to_args(&mut self.cx, &model)?;
        let calldata_bytes = Bytes::from(function.abi_encode_input(&args)?);
        Ok((SymbolicConcreteInput { args, calldata: calldata_bytes }, model))
    }

    pub(super) fn run_invariant_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> Result<SymbolicInvariantRunResult, SymbolicError> {
        if input.targets.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant has no targets"));
        }

        let mut senders =
            if input.senders.is_empty() { vec![input.sender] } else { input.senders.clone() };
        senders.retain(|sender| !input.excluded_senders.contains(sender));
        if senders.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant senders are excluded"));
        }
        let after_invariant_for = |steps_len: usize| {
            (steps_len == input.depth).then_some(input.after_invariant).flatten()
        };
        let mut completed_paths = 0usize;
        let mut initial_state = PathState::empty(
            &mut self.cx,
            input.invariant_address,
            input.sender,
            input.ffi_enabled,
        );
        initial_state.apply_executor_env(&mut self.cx, input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);
        let initial = SequencePath { state: initial_state, steps: Vec::new() };

        if symbolic_invariant_should_check(0, input.depth, input.check_interval) {
            for outcome in self.execute_invariant_check(
                input.executor,
                initial.state.clone(),
                input.invariant_address,
                input.sender,
                input.invariant,
                after_invariant_for(0),
                &mut completed_paths,
            )? {
                if outcome.failed {
                    let (sequence, storage) =
                        self.materialize_sequence(&initial.steps, &outcome.state)?;
                    return Ok(SymbolicInvariantRunResult::Counterexample {
                        kind: SymbolicInvariantCounterexampleKind::Predicate,
                        sequence,
                        storage,
                        stats: self.stats_with_paths(completed_paths),
                    });
                }
            }
        }

        let path_limit = self.config.path_width() as usize;
        let mut frontier = vec![initial];
        for depth in 0..input.depth {
            self.check_timeout()?;
            let mut next_frontier = Vec::new();
            for sequence in frontier {
                self.check_timeout()?;
                for (target_idx, target) in input.targets.iter().enumerate() {
                    for (sender_idx, sender) in senders.iter().copied().enumerate() {
                        self.check_timeout()?;
                        let prefix = format!("sequence_{depth}_{target_idx}_{sender_idx}");
                        let calldatas = SymbolicCalldata::variants_with_prefix(
                            &target.function,
                            &self.config,
                            &mut self.cx,
                            &prefix,
                        )?;
                        for calldata in calldatas {
                            let step = SequenceStepTemplate {
                                sender,
                                address: target.address,
                                contract_name: target.contract_name.clone(),
                                function: target.function.clone(),
                                calldata,
                            };
                            let calldata = step.calldata.call_data(&mut self.cx);
                            let constraints = step.calldata.constraints().to_vec();
                            let mut call = self.prepare_sequence_call(
                                input.executor,
                                sequence.state.clone(),
                                target.address,
                                sender,
                                &target.function,
                                calldata,
                                constraints,
                            )?;

                            while let Some(outcome) = self.execute_sequence_call_next(
                                input.executor,
                                &mut call,
                                &mut completed_paths,
                            )? {
                                let mut steps = sequence.steps.clone();
                                steps.push(step.clone());

                                let post_state = match outcome.status {
                                    CallStatus::Failure => {
                                        let (sequence, storage) =
                                            self.materialize_sequence(&steps, &outcome.state)?;
                                        return Ok(SymbolicInvariantRunResult::Counterexample {
                                            kind: SymbolicInvariantCounterexampleKind::Handler,
                                            sequence,
                                            storage,
                                            stats: self.stats_with_paths(completed_paths),
                                        });
                                    }
                                    CallStatus::Revert | CallStatus::ExceptionalHalt => {
                                        if input.fail_on_revert {
                                            let (sequence, storage) =
                                                self.materialize_sequence(&steps, &outcome.state)?;
                                            return Ok(
                                                SymbolicInvariantRunResult::Counterexample {
                                                    kind: SymbolicInvariantCounterexampleKind::Predicate,
                                                    sequence,
                                                    storage,
                                                    stats: self.stats_with_paths(completed_paths),
                                                },
                                            );
                                        }
                                        // A reverted top-level call cannot change persistent
                                        // state, but it still consumes one invariant sequence
                                        // step. Preserve the pre-call world together with the
                                        // reverted branch constraints so end-only and periodic
                                        // invariant checks observe the same call schedule as the
                                        // concrete campaign.
                                        let mut reverted_state = sequence.state.clone();
                                        reverted_state
                                            .take_reverted_top_level_effects(outcome.state);
                                        reverted_state
                                    }
                                    CallStatus::Success => outcome.state,
                                };
                                if symbolic_invariant_should_check(
                                    steps.len(),
                                    input.depth,
                                    input.check_interval,
                                ) {
                                    for mut invariant_outcome in self.execute_invariant_check(
                                        input.executor,
                                        post_state.clone(),
                                        input.invariant_address,
                                        input.sender,
                                        input.invariant,
                                        after_invariant_for(steps.len()),
                                        &mut completed_paths,
                                    )? {
                                        if invariant_outcome.failed {
                                            let (sequence, storage) = self.materialize_sequence(
                                                &steps,
                                                &invariant_outcome.state,
                                            )?;
                                            return Ok(SymbolicInvariantRunResult::Counterexample {
                                                kind: SymbolicInvariantCounterexampleKind::Predicate,
                                                sequence,
                                                storage,
                                                stats: self.stats_with_paths(completed_paths),
                                            });
                                        }
                                        let mut state = post_state.clone();
                                        state.take_noncommitting_check_state(
                                            &mut invariant_outcome.state,
                                        );
                                        next_frontier
                                            .push(SequencePath { state, steps: steps.clone() });
                                    }
                                } else {
                                    next_frontier.push(SequencePath { state: post_state, steps });
                                }

                                if completed_paths >= path_limit {
                                    return Ok(SymbolicInvariantRunResult::Incomplete {
                                        kind: SymbolicStopReason::Stuck,
                                        reason: format!(
                                            "symbolic path limit exceeded ({path_limit})"
                                        ),
                                        stats: self.stats_with_paths(completed_paths),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            if next_frontier.is_empty() {
                break;
            }
            frontier = next_frontier;
        }

        if let Some((kind, reason)) = self.deferred_incomplete() {
            return Ok(SymbolicInvariantRunResult::Incomplete {
                kind,
                reason,
                stats: self.stats_with_paths(completed_paths),
            });
        }

        Ok(SymbolicInvariantRunResult::Safe(self.stats_with_paths(completed_paths)))
    }

    pub(super) fn stats_with_paths(&self, paths: usize) -> SymbolicStats {
        let mut stats = self.solver.stats();
        stats.paths = paths;
        stats
    }
}

fn order_roots_by_corpus_seed_count(roots: &mut [PathState], order: SymbolicExplorationOrder) {
    let Some((first, rest)) = roots.split_first() else {
        return;
    };
    if rest.iter().all(|root| root.corpus_seed_model_count() == first.corpus_seed_model_count()) {
        return;
    }

    match order {
        SymbolicExplorationOrder::Bfs => {
            roots.sort_by_key(|root| Reverse(root.corpus_seed_model_count()));
        }
        SymbolicExplorationOrder::Dfs => {
            roots.sort_by_key(PathState::corpus_seed_model_count);
        }
    }
}

const fn symbolic_invariant_should_check(
    sequence_len: usize,
    depth: usize,
    check_interval: u32,
) -> bool {
    sequence_len == depth
        || (check_interval != 0
            && sequence_len != 0
            && sequence_len.is_multiple_of(check_interval as usize))
}
