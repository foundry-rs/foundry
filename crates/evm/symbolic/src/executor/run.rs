use super::*;
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
            equivalence: None,
            unenumerable_offsets: HashSet::default(),
            handler_counterexamples: Vec::new(),
        }
    }

    /// Whether an equivalence check leaves out inputs that run out of gas.
    pub(super) fn ignores_out_of_gas(&self) -> bool {
        self.equivalence.is_some_and(|mode| mode.ignore_out_of_gas)
    }

    /// Whether forward jumps over pure instructions merge into one path, as an equivalence check
    /// may ask for.
    pub(super) fn merges_states(&self) -> bool {
        self.equivalence.is_some_and(|mode| mode.merge_states)
    }

    /// Whether `GAS` is an ordinary unconstrained symbol, as in an equivalence check.
    pub(super) const fn symbolic_gas(&self) -> bool {
        self.equivalence.is_some()
    }

    pub(super) fn reset_run_state(&mut self, use_wall_clock_deadline: bool) {
        self.unenumerable_offsets.clear();
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
        self.solver.set_deadline(self.deadline);
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
        debug!(reason, "deferred incomplete");
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
    pub(super) fn deferred_incomplete(&self) -> Option<(SymbolicStopReason, String)> {
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
    pub fn run(&mut self, input: SymbolicRunInput<'_>) -> SymbolicRunResult {
        self.execute_run(input, None)
    }

    /// Searches for concrete inputs that complete after reaching a requested branch outcome.
    ///
    /// Unlike [`Self::run`], this retains replay candidates independently of the proof result, so a
    /// later unsupported path or resource limit does not discard an input found on an earlier
    /// completed path. The caller must concretely replay every candidate before using it.
    pub fn search_branch_target(
        &mut self,
        input: SymbolicRunInput<'_>,
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

    fn execute_run(
        &mut self,
        input: SymbolicRunInput<'_>,
        mut branch_candidates: Option<&mut Vec<SymbolicConcreteInput>>,
    ) -> SymbolicRunResult {
        self.reset_run_state(false);
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
    pub fn run_invariant(
        &mut self,
        input: SymbolicInvariantRunInput<'_>,
    ) -> SymbolicInvariantRunResult {
        self.reset_run_state(true);
        self.handler_counterexamples.clear();
        // A collecting search shares one deadline across many handler paths, so no single query
        // may spend more than a slice of it: an unknown query defers its path instead.
        let query_timeout = self.config.timeout;
        if let Some(deadline) = input.deadline {
            self.deadline = Some(deadline);
            self.solver.set_deadline(self.deadline);
            if input.collect_handler_failures {
                let budget = deadline.saturating_duration_since(Instant::now()).as_secs();
                let slice = u32::try_from(budget / INVARIANT_QUERY_SLICES).unwrap_or(u32::MAX);
                let slice = slice.max(1);
                self.solver.set_query_timeout(Some(
                    query_timeout.filter(|timeout| *timeout > 0).map_or(slice, |t| t.min(slice)),
                ));
            }
        }
        let result = self.run_invariant_collecting(input);
        self.solver.set_query_timeout(query_timeout);
        if let SymbolicInvariantRunResult::Incomplete { kind, reason, .. } = &result {
            debug!(?kind, reason, "symbolic invariant search incomplete");
        }
        result
    }

    fn run_invariant_collecting(
        &mut self,
        input: SymbolicInvariantRunInput<'_>,
    ) -> SymbolicInvariantRunResult {
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

    /// Returns the handler counterexamples the last invariant run collected, each a
    /// [`SymbolicInvariantRunResult::Counterexample`] of kind handler.
    pub fn take_handler_counterexamples(&mut self) -> Vec<SymbolicInvariantRunResult> {
        std::mem::take(&mut self.handler_counterexamples)
    }

    /// Searches for invariant-breaking inputs after one symbolic handler call.
    ///
    /// This is a best-effort candidate search from a concrete state. Returned candidates are
    /// unconfirmed until the caller replays them concretely, and an empty result does not prove
    /// any invariant.
    pub fn search_invariant_candidates(
        &mut self,
        input: SymbolicInvariantCandidateInput<'_>,
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

    pub(super) fn run_inner(
        &mut self,
        input: &SymbolicRunInput<'_>,
        mut branch_candidates: Option<&mut Vec<SymbolicConcreteInput>>,
        completed_paths: &mut usize,
    ) -> Result<SymbolicRunResult, SymbolicError> {
        let account = input
            .executor
            .backend
            .basic_ref(input.target)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(input.target))?;
        let bytecode = account.code.ok_or(SymbolicError::MissingCode(input.target))?;
        let code = SymCode::from_bytecode(&mut self.cx, &bytecode);
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
            root.apply_executor_env(&mut self.cx, &input.executor);
            root.world.set_storage_layout(self.config.storage_layout);
            root.world.clear_transaction_scoped_state();
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
                            &input.executor,
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
                            self.materialize_root_input(input.function, &state)?;
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
    fn materialize_root_input(
        &mut self,
        function: &Function,
        state: &PathState,
    ) -> Result<SymbolicConcreteInput, SymbolicError> {
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
        Ok(SymbolicConcreteInput { args, calldata: calldata_bytes })
    }

    pub(super) fn run_invariant_inner(
        &mut self,
        input: SymbolicInvariantRunInput<'_>,
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
        initial_state.apply_executor_env(&mut self.cx, &input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);
        let initial = SequencePath { state: initial_state, steps: Vec::new() };

        if symbolic_invariant_should_check(0, input.depth, input.check_interval) {
            for outcome in self.execute_invariant_check(
                &input.executor,
                &initial.state,
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
        let mut root_noop_kept = false;
        let mut collected_handlers = Vec::new();
        let mut frontier = vec![initial];
        for depth in 0..input.depth {
            self.check_timeout()?;
            let mut next_frontier = Vec::new();
            // A collecting search shares what is left of the deadline evenly among the calls of
            // this depth still to run, so one expensive handler cannot starve the others.
            let mut calls_left = frontier.len() * input.targets.len() * senders.len();
            for sequence in frontier {
                self.check_timeout()?;
                for (target_idx, target) in input.targets.iter().enumerate() {
                    // Whether every path of this handler's calls from the first sender was
                    // explored and none read the sender: then the other senders reach the same
                    // states and failures, so they are skipped.
                    let mut sender_free = true;
                    for (sender_idx, sender) in senders.iter().copied().enumerate() {
                        self.check_timeout()?;
                        if sender_idx > 0 && sender_free {
                            calls_left = calls_left.saturating_sub(1);
                            continue;
                        }
                        let call_deadline = self
                            .deadline
                            .filter(|_| input.collect_handler_failures)
                            .map(|deadline| {
                                let now = Instant::now();
                                let share = calls_left.max(1) as u32;
                                now + deadline.saturating_duration_since(now) / share
                            });
                        calls_left = calls_left.saturating_sub(1);
                        let prefix = format!("sequence_{depth}_{target_idx}_{sender_idx}");
                        let calldatas = SymbolicCalldata::variants_with_prefix(
                            &target.function,
                            &self.config,
                            &mut self.cx,
                            &prefix,
                        )?;
                        for calldata in calldatas {
                            // Each step may advance the block by up to the campaign's delays,
                            // as the concrete campaign warps and rolls between calls.
                            let mut state = sequence.state.clone();
                            let mut constraints = Vec::new();
                            let mut delay = |name: &str, max: Option<u32>, word: &mut SymExpr| {
                                let max = max.filter(|max| *max > 0)?;
                                let delay = SymExpr::var(&mut self.cx, &format!("{prefix}_{name}"));
                                constraints.push(SymBoolExpr::cmp_word_const(
                                    &mut self.cx,
                                    SymCmpOp::Ule,
                                    &delay,
                                    U256::from(max),
                                ));
                                *word = SymExpr::binop(
                                    &mut self.cx,
                                    SymBinOp::Add,
                                    word.clone(),
                                    delay.clone(),
                                );
                                Some(delay)
                            };
                            let warp =
                                delay("warp", input.max_time_delay, &mut state.block.timestamp);
                            let roll =
                                delay("roll", input.max_block_delay, &mut state.block.number);
                            let step = SequenceStepTemplate {
                                sender,
                                address: target.address,
                                contract_name: target.contract_name.clone(),
                                function: target.function.clone(),
                                calldata,
                                warp,
                                roll,
                            };
                            let calldata = step.calldata.call_data(&mut self.cx);
                            constraints.extend_from_slice(step.calldata.constraints());
                            let mut call = self.prepare_sequence_call(
                                &input.executor,
                                state,
                                target.address,
                                sender,
                                &target.function,
                                calldata,
                                constraints,
                            )?;

                            // The path limit bounds each handler call; the deadline bounds the
                            // whole search.
                            let mut call_paths = 0usize;
                            loop {
                                let outcome = match self.execute_sequence_call_next(
                                    &input.executor,
                                    &mut call,
                                    &mut call_paths,
                                ) {
                                    Ok(Some(outcome)) => outcome,
                                    Ok(None) => break,
                                    // A collecting search gives up on this call alone and keeps
                                    // looking through the other handlers.
                                    Err(SymbolicError::Unsupported(reason))
                                        if input.collect_handler_failures =>
                                    {
                                        self.defer_incomplete(reason);
                                        sender_free = false;
                                        break;
                                    }
                                    Err(SymbolicError::SolverUnknown)
                                        if input.collect_handler_failures =>
                                    {
                                        self.defer_solver_unknown();
                                        sender_free = false;
                                        break;
                                    }
                                    Err(err) => return Err(err),
                                };
                                completed_paths += 1;
                                sender_free &= !outcome.state.sender_observed;
                                if call_deadline.is_some_and(|deadline| Instant::now() >= deadline)
                                {
                                    self.defer_incomplete(
                                        "handler call exceeded its share of the deadline",
                                    );
                                    // This outcome is still checked; the call's other paths are
                                    // not.
                                    call.worklist.clear();
                                    call.deferred_worklist.clear();
                                    sender_free = false;
                                }
                                let mut steps = sequence.steps.clone();
                                steps.push(step.clone());

                                let handler = (target.address, target.function.selector().0);
                                let ignored = input.ignored_handler_failures.contains(&handler)
                                    || collected_handlers.contains(&handler);
                                let status = match outcome.status {
                                    CallStatus::Failure if ignored => CallStatus::Revert,
                                    status => status,
                                };
                                let post_state = match status {
                                    CallStatus::Failure => {
                                        let (sequence, storage) =
                                            self.materialize_sequence(&steps, &outcome.state)?;
                                        let counterexample =
                                            SymbolicInvariantRunResult::Counterexample {
                                                kind: SymbolicInvariantCounterexampleKind::Handler,
                                                sequence,
                                                storage,
                                                stats: self.stats_with_paths(completed_paths),
                                            };
                                        if !input.collect_handler_failures {
                                            return Ok(counterexample);
                                        }
                                        trace!(?handler, "collected symbolic handler failure");
                                        collected_handlers.push(handler);
                                        self.handler_counterexamples.push(counterexample);
                                        // One failure per handler is kept, so this call's other
                                        // paths can only find the same one again.
                                        call.worklist.clear();
                                        call.deferred_worklist.clear();
                                        sender_free = false;
                                        continue;
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
                                // With the invariant checked after every call, a call that left the
                                // world unchanged adds nothing the parent sequence did not: its
                                // state was checked already and its extensions are the parent's.
                                // A root's first no-op child stands for checking the root once.
                                let unchanged = post_state.world.same_state(&sequence.state.world);
                                if unchanged
                                    && input.check_interval == 1
                                    && steps.len() < input.depth
                                {
                                    let root = sequence.steps.is_empty();
                                    if !root || std::mem::replace(&mut root_noop_kept, true) {
                                        continue;
                                    }
                                }
                                if next_frontier.len() >= path_limit {
                                    self.defer_incomplete("symbolic invariant frontier limit");
                                    continue;
                                }
                                if symbolic_invariant_should_check(
                                    steps.len(),
                                    input.depth,
                                    input.check_interval,
                                ) {
                                    // The path limit bounds each check like each handler call.
                                    let mut check_paths = 0usize;
                                    let checked = self.execute_invariant_check(
                                        &input.executor,
                                        &post_state,
                                        input.invariant_address,
                                        input.sender,
                                        input.invariant,
                                        after_invariant_for(steps.len()),
                                        &mut check_paths,
                                    )?;
                                    completed_paths += check_paths;
                                    for mut invariant_outcome in checked {
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

                                // Without a deadline the path limit also bounds the whole search.
                                // Handler failures a collecting search found are kept for the
                                // caller.
                                if input.deadline.is_none() && completed_paths >= path_limit {
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

            debug!(
                depth,
                frontier = next_frontier.len(),
                completed_paths,
                handler_failures = collected_handlers.len(),
                "finished symbolic invariant depth"
            );
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

/// Number of slices of a collecting invariant search's budget; each query may use one.
const INVARIANT_QUERY_SLICES: u64 = 16;
