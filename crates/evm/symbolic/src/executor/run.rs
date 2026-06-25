use super::*;

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
        Self { config, solver: Box::new(solver), deferred_incomplete: None }
    }

    /// Defers an incomplete result until all counterexample-producing modeled paths are explored.
    pub(super) fn defer_incomplete(&mut self, reason: &'static str) {
        self.deferred_incomplete.get_or_insert(DeferredIncomplete::Unsupported(reason));
    }

    /// Defers a solver-unknown result while continuing with decidable sibling paths.
    pub(super) fn defer_solver_unknown(&mut self) {
        self.deferred_incomplete.get_or_insert(DeferredIncomplete::SolverUnknown);
    }

    /// Checks branch feasibility, recording solver-unknown as an incomplete proof path.
    pub(super) fn branch_is_sat_or_defer(
        &mut self,
        constraints: &[BoolExpr],
    ) -> Result<bool, SymbolicError> {
        match self.solver.is_sat_branch(constraints) {
            Ok(feasible) => Ok(feasible),
            Err(SymbolicError::SolverUnknown) => {
                self.defer_solver_unknown();
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Returns and clears any deferred incomplete reason.
    fn take_deferred_incomplete(&mut self) -> Option<(SymbolicStopReason, String)> {
        match self.deferred_incomplete.take()? {
            DeferredIncomplete::Unsupported(reason) => Some((
                SymbolicStopReason::Stuck,
                format!("unsupported symbolic execution feature: {reason}"),
            )),
            DeferredIncomplete::SolverUnknown => {
                Some((SymbolicStopReason::Timeout, "solver returned unknown".to_string()))
            }
        }
    }

    /// Returns staged solver portfolio diagnostics collected by this executor.
    pub fn portfolio_diagnostics(&self) -> Option<PortfolioDiagnostics> {
        self.solver.portfolio_diagnostics().cloned()
    }

    /// Defers verbose solver diagnostics until the caller explicitly takes them.
    pub fn capture_diagnostics(&mut self) {
        self.solver.capture_diagnostics();
    }

    /// Returns and clears deferred verbose solver diagnostics.
    pub fn take_diagnostics(&mut self) -> Option<String> {
        self.solver.take_diagnostics()
    }

    /// Registers a callback invoked after each solver query for live progress rendering.
    pub fn set_query_observer(&mut self, observer: impl Fn(usize) + Send + Sync + 'static) {
        self.solver.set_query_observer(Some(Box::new(observer)));
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
        self.deferred_incomplete = None;
        if let Err(err) = self.solver.check_available() {
            return SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        match self.run_inner(input) {
            Ok(result) => result,
            Err(err) => SymbolicRunResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: self.solver.stats(),
            },
        }
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
        self.deferred_incomplete = None;
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

    /// Runs the `run_inner` symbolic executor helper.
    pub(super) fn run_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicRunInput<'_, FEN>,
    ) -> Result<SymbolicRunResult, SymbolicError> {
        let heuristic_witness_baseline = self.solver.heuristic_witnesses();
        let account = input
            .executor
            .backend()
            .basic_ref(input.target)
            .map_err(|err| SymbolicError::Backend(err.to_string()))?
            .ok_or(SymbolicError::MissingAccount(input.target))?;
        let code =
            account.code.ok_or(SymbolicError::MissingCode(input.target))?.original_bytes().to_vec();
        let code = SymCode::concrete(code);
        let jumpdests = analyze_jumpdests(&code);
        let mut worklist = VecDeque::new();
        for calldata in SymbolicCalldata::variants(input.function, &self.config)? {
            let mut root = PathState::new(
                input.target,
                input.sender,
                input.value,
                calldata,
                input.ffi_enabled,
            );
            root.apply_executor_env(input.executor);
            root.world.set_storage_layout(self.config.storage_layout);
            root.world.clear_transaction_scoped_state();
            worklist.push_back(root);
        }
        let mut completed_paths = 0usize;
        let mut reverted_paths = 0usize;
        let mut normal_paths = 0usize;
        let mut success_input = None;
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = pop_worklist(&mut worklist, self.config.exploration_order) {
            if completed_paths >= path_limit {
                debug!(completed_paths, path_limit, "symbolic path limit reached");
                return Ok(SymbolicRunResult::Incomplete {
                    kind: SymbolicStopReason::Stuck,
                    reason: format!("symbolic path limit exceeded ({path_limit})"),
                    stats: self.stats_with_paths(completed_paths),
                });
            }
            let _path_span =
                trace_span!("symbolic_path", completed_paths, worklist_size = worklist.len())
                    .entered();
            trace!(completed_paths, worklist_size = worklist.len(), "exploring symbolic path");

            loop {
                if state.depth >= depth_limit {
                    debug!(depth = state.depth, depth_limit, "symbolic depth limit reached");
                    return Ok(SymbolicRunResult::Incomplete {
                        kind: SymbolicStopReason::Stuck,
                        reason: format!("symbolic depth limit exceeded ({depth_limit})"),
                        stats: self.stats_with_paths(completed_paths),
                    });
                }
                state.depth += 1;

                let Some(op) = code.opcode(state.pc)? else {
                    if !state.expectations_satisfied() {
                        let (args, calldata_bytes) = self.materialize_stateless_counterexample(
                            state.root_calldata.as_ref().ok_or_else(|| {
                                SymbolicError::Unsupported("missing root symbolic calldata")
                            })?,
                            input.function,
                            &state,
                        )?;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata: calldata_bytes,
                            stats: self.stats_with_paths(completed_paths + 1),
                        });
                    }
                    if input.collect_success_input
                        && success_input.as_ref().is_none_or(|(depth, _)| state.depth > *depth)
                    {
                        success_input = Some((
                            state.depth,
                            self.materialize_stateless_input(
                                state.root_calldata.as_ref().ok_or_else(|| {
                                    SymbolicError::Unsupported("missing root symbolic calldata")
                                })?,
                                input.function,
                                &state,
                            )?,
                        ));
                    }
                    completed_paths += 1;
                    break;
                };

                let _step_span = trace_span!("symbolic_step", pc = state.pc - 1, op).entered();
                match self.step(
                    input.executor,
                    &code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    &mut completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        if !state.expectations_satisfied() {
                            let (args, calldata_bytes) = self
                                .materialize_stateless_counterexample(
                                    state.root_calldata.as_ref().ok_or_else(|| {
                                        SymbolicError::Unsupported("missing root symbolic calldata")
                                    })?,
                                    input.function,
                                    &state,
                                )?;
                            return Ok(SymbolicRunResult::Counterexample {
                                args,
                                calldata: calldata_bytes,
                                stats: self.stats_with_paths(completed_paths + 1),
                            });
                        }
                        if input.collect_success_input
                            && success_input.as_ref().is_none_or(|(depth, _)| state.depth > *depth)
                        {
                            success_input = Some((
                                state.depth,
                                self.materialize_stateless_input(
                                    state.root_calldata.as_ref().ok_or_else(|| {
                                        SymbolicError::Unsupported("missing root symbolic calldata")
                                    })?,
                                    input.function,
                                    &state,
                                )?,
                            ));
                        }
                        completed_paths += 1;
                        normal_paths += 1;
                        break;
                    }
                    StepOutcome::Revert => {
                        completed_paths += 1;
                        reverted_paths += 1;
                        break;
                    }
                    StepOutcome::AssumeRejected => break,
                    StepOutcome::Forked => break,
                    StepOutcome::Failure => {
                        let (args, calldata_bytes) = self.materialize_stateless_counterexample(
                            state.root_calldata.as_ref().ok_or_else(|| {
                                SymbolicError::Unsupported("missing root symbolic calldata")
                            })?,
                            input.function,
                            &state,
                        )?;
                        return Ok(SymbolicRunResult::Counterexample {
                            args,
                            calldata: calldata_bytes,
                            stats: self.stats_with_paths(completed_paths + 1),
                        });
                    }
                }
            }
        }

        if normal_paths == 0 && reverted_paths > 0 {
            debug!(completed_paths, "all symbolic paths reverted");
            return Ok(SymbolicRunResult::Incomplete {
                kind: SymbolicStopReason::RevertAll,
                reason: "all symbolic paths reverted".to_string(),
                stats: self.stats_with_paths(completed_paths),
            });
        }

        if self.heuristic_witnesses_used_since(heuristic_witness_baseline) {
            return Ok(SymbolicRunResult::Incomplete {
                kind: SymbolicStopReason::Timeout,
                reason: Self::hard_arith_heuristic_incomplete_reason(),
                stats: self.stats_with_paths(completed_paths),
            });
        }

        if let Some((kind, reason)) = self.take_deferred_incomplete() {
            return Ok(SymbolicRunResult::Incomplete {
                kind,
                reason,
                stats: self.stats_with_paths(completed_paths),
            });
        }

        debug!(completed_paths, "symbolic execution safe");
        Ok(SymbolicRunResult::Safe {
            stats: self.stats_with_paths(completed_paths),
            success_input: success_input.map(|(_, input)| input),
        })
    }

    /// Runs the `materialize_stateless_counterexample` symbolic executor helper.
    pub(super) fn materialize_stateless_counterexample(
        &mut self,
        calldata: &SymbolicCalldata,
        function: &Function,
        state: &PathState,
    ) -> Result<(Vec<DynSolValue>, Bytes), SymbolicError> {
        debug!(
            constraint_count = state.constraints.len(),
            "materializing counterexample from solver model"
        );
        self.materialize_stateless_input(calldata, function, state)
            .map(|input| (input.args, input.calldata))
    }

    /// Runs the `materialize_stateless_input` symbolic executor helper.
    pub(super) fn materialize_stateless_input(
        &mut self,
        calldata: &SymbolicCalldata,
        function: &Function,
        state: &PathState,
    ) -> Result<SymbolicConcreteInput, SymbolicError> {
        let model = self.solver.model(&state.constraints)?;
        let args = calldata.model_to_args(&model)?;
        let calldata_bytes = Bytes::from(function.abi_encode_input(&args)?);
        Ok(SymbolicConcreteInput { args, calldata: calldata_bytes })
    }

    /// Runs the `run_invariant_inner` symbolic executor helper.
    pub(super) fn run_invariant_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: SymbolicInvariantRunInput<'_, FEN>,
    ) -> Result<SymbolicInvariantRunResult, SymbolicError> {
        let heuristic_witness_baseline = self.solver.heuristic_witnesses();
        if input.targets.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant has no targets"));
        }

        let senders =
            if input.senders.is_empty() { vec![input.sender] } else { input.senders.clone() };
        let mut completed_paths = 0usize;
        let mut initial_state =
            PathState::empty(input.invariant_address, input.sender, input.ffi_enabled);
        initial_state.apply_executor_env(input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);
        let initial = SequencePath { state: initial_state, steps: Vec::new() };

        for outcome in self.execute_invariant_check(
            input.executor,
            initial.state.clone(),
            input.invariant_address,
            input.sender,
            input.invariant,
            input.after_invariant,
            &mut completed_paths,
        )? {
            if outcome.failed {
                let sequence = self.materialize_sequence(&initial.steps, &outcome.state)?;
                return Ok(SymbolicInvariantRunResult::Counterexample {
                    sequence,
                    stats: self.stats_with_paths(completed_paths),
                });
            }
        }

        let path_limit = self.config.path_width() as usize;
        let mut frontier = vec![initial];
        for depth in 0..input.depth {
            let mut next_frontier = Vec::new();
            for sequence in frontier {
                for (target_idx, target) in input.targets.iter().enumerate() {
                    for (sender_idx, sender) in senders.iter().copied().enumerate() {
                        let prefix = format!("sequence_{depth}_{target_idx}_{sender_idx}");
                        let calldatas = SymbolicCalldata::variants_with_prefix(
                            &target.function,
                            &self.config,
                            prefix,
                        )?;
                        for calldata in calldatas {
                            let step = SequenceStepTemplate {
                                sender,
                                address: target.address,
                                contract_name: target.contract_name.clone(),
                                function: target.function.clone(),
                                calldata,
                            };
                            let outcomes = self.execute_sequence_call(
                                input.executor,
                                sequence.state.clone(),
                                target.address,
                                sender,
                                &target.function,
                                step.calldata.call_data(),
                                step.calldata.constraints.clone(),
                                &mut completed_paths,
                            )?;

                            for outcome in outcomes {
                                let mut steps = sequence.steps.clone();
                                steps.push(step.clone());

                                match outcome.status {
                                    TopLevelCallStatus::Failure => {
                                        let sequence =
                                            self.materialize_sequence(&steps, &outcome.state)?;
                                        return Ok(SymbolicInvariantRunResult::Counterexample {
                                            sequence,
                                            stats: self.stats_with_paths(completed_paths),
                                        });
                                    }
                                    TopLevelCallStatus::Revert => {
                                        if input.fail_on_revert {
                                            let sequence =
                                                self.materialize_sequence(&steps, &outcome.state)?;
                                            return Ok(
                                                SymbolicInvariantRunResult::Counterexample {
                                                    sequence,
                                                    stats: self.stats_with_paths(completed_paths),
                                                },
                                            );
                                        }
                                        let mut reverted_state = sequence.state.clone();
                                        reverted_state.world.clear_transaction_scoped_state();
                                        for invariant_outcome in self.execute_invariant_check(
                                            input.executor,
                                            reverted_state,
                                            input.invariant_address,
                                            input.sender,
                                            input.invariant,
                                            input.after_invariant,
                                            &mut completed_paths,
                                        )? {
                                            if invariant_outcome.failed {
                                                let sequence = self.materialize_sequence(
                                                    &steps,
                                                    &invariant_outcome.state,
                                                )?;
                                                return Ok(
                                                    SymbolicInvariantRunResult::Counterexample {
                                                        sequence,
                                                        stats: self
                                                            .stats_with_paths(completed_paths),
                                                    },
                                                );
                                            }
                                            next_frontier.push(SequencePath {
                                                state: invariant_outcome.state,
                                                steps: steps.clone(),
                                            });
                                        }
                                    }
                                    TopLevelCallStatus::Success => {
                                        for invariant_outcome in self.execute_invariant_check(
                                            input.executor,
                                            outcome.state.clone(),
                                            input.invariant_address,
                                            input.sender,
                                            input.invariant,
                                            input.after_invariant,
                                            &mut completed_paths,
                                        )? {
                                            if invariant_outcome.failed {
                                                let sequence = self.materialize_sequence(
                                                    &steps,
                                                    &invariant_outcome.state,
                                                )?;
                                                return Ok(
                                                    SymbolicInvariantRunResult::Counterexample {
                                                        sequence,
                                                        stats: self
                                                            .stats_with_paths(completed_paths),
                                                    },
                                                );
                                            }
                                            next_frontier.push(SequencePath {
                                                state: invariant_outcome.state,
                                                steps: steps.clone(),
                                            });
                                        }
                                    }
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

        if self.heuristic_witnesses_used_since(heuristic_witness_baseline) {
            return Ok(SymbolicInvariantRunResult::Incomplete {
                kind: SymbolicStopReason::Timeout,
                reason: Self::hard_arith_heuristic_incomplete_reason(),
                stats: self.stats_with_paths(completed_paths),
            });
        }

        if let Some((kind, reason)) = self.take_deferred_incomplete() {
            return Ok(SymbolicInvariantRunResult::Incomplete {
                kind,
                reason,
                stats: self.stats_with_paths(completed_paths),
            });
        }

        Ok(SymbolicInvariantRunResult::Safe(self.stats_with_paths(completed_paths)))
    }

    /// Implements the `stats_with_paths` symbolic executor helper.
    pub(super) fn stats_with_paths(&self, paths: usize) -> SymbolicStats {
        let mut stats = self.solver.stats();
        stats.paths = paths;
        stats
    }

    /// Returns whether this run used a hard-arithmetic heuristic witness.
    fn heuristic_witnesses_used_since(&self, baseline: usize) -> bool {
        self.solver.heuristic_witnesses() > baseline
    }

    /// Returns the incomplete reason used when heuristic witnesses cannot certify safety.
    fn hard_arith_heuristic_incomplete_reason() -> String {
        "hard arithmetic heuristic witness used; no replayed counterexample found".to_string()
    }
}
