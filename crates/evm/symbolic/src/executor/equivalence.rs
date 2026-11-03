use super::*;

/// How one side of an equivalence check ended on one path.
struct EquivalenceEnd {
    /// Whether the top-level call returned successfully.
    success: bool,
    /// Return data on success, revert data otherwise (empty for exceptional halts).
    return_data: SymReturnData,
}

impl SymbolicExecutor {
    /// Checks whether two runtime bytecodes behave identically for every modeled input.
    ///
    /// Both codes run at [`SymbolicEquivalenceInput::address`] from the same pre-state on shared
    /// symbolic calldata. They are equivalent when every pair of feasible paths agrees on success,
    /// on return or revert data and, when both succeed, on every storage slot either one wrote.
    /// A counterexample is a solver model and must be replayed concretely before it is reported.
    pub fn run_equivalence(
        &mut self,
        input: SymbolicEquivalenceInput<'_>,
    ) -> SymbolicEquivalenceResult {
        self.reset_run_state(false);
        self.solver.clear_context_caches();
        self.cx = SymCx::new();
        if let Err(err) = self.solver.check_available() {
            return SymbolicEquivalenceResult::Incomplete {
                kind: err.stop_reason(),
                reason: err.to_string(),
                stats: SymbolicStats::default(),
            };
        }

        let mut completed_paths = 0;
        match self.run_equivalence_inner(&input, &mut completed_paths) {
            Ok(Some(counterexample)) => SymbolicEquivalenceResult::Counterexample {
                calldata: counterexample.calldata,
                args: counterexample.args,
                storage: counterexample.storage,
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

        let calldatas = match input.function {
            Some(function) => SymbolicCalldata::variants(function, &self.config, &mut self.cx)?,
            None => {
                if input.calldata_lengths.is_empty() {
                    return Err(SymbolicError::Unsupported(
                        "equivalence check needs a function or calldata lengths",
                    ));
                }
                let max = self.config.max_calldata_bytes as usize;
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

        for calldata in calldatas {
            self.check_timeout()?;
            let root_a = self.equivalence_root(input, calldata.clone(), &code_a);
            let mut side_a = SequenceCall {
                code: code_a.clone(),
                worklist: VecDeque::from([root_a]),
                deferred_worklist: VecDeque::new(),
            };
            while let Some(outcome_a) =
                self.execute_sequence_call_next(&input.executor, &mut side_a, completed_paths)?
            {
                let end_a = self.equivalence_end(&outcome_a);
                let mut state_a = outcome_a.state;

                // B starts from the same pre-state, constrained to A's path.
                let mut root_b = self.equivalence_root(input, calldata.clone(), &code_b);
                root_b.take_noncommitting_check_state(&mut state_a.clone());
                let mut side_b = SequenceCall {
                    code: code_b.clone(),
                    worklist: VecDeque::from([root_b]),
                    deferred_worklist: VecDeque::new(),
                };
                while let Some(outcome_b) =
                    self.execute_sequence_call_next(&input.executor, &mut side_b, completed_paths)?
                {
                    self.check_timeout()?;
                    let end_b = self.equivalence_end(&outcome_b);
                    let mut state_b = outcome_b.state;
                    let mismatch = self.equivalence_mismatch(
                        &input.executor,
                        input.address,
                        &mut state_a,
                        &end_a,
                        &mut state_b,
                        &end_b,
                    )?;
                    if mismatch.as_const() == Some(false) {
                        continue;
                    }
                    let mut constraints = state_b.constraints.clone();
                    constraints.push(mismatch);
                    match self.is_sat_with_state(&state_b, &constraints) {
                        Ok(true) => {
                            return self
                                .materialize_equivalence(input, &calldata, &state_b, &constraints)
                                .map(Some);
                        }
                        Ok(false) => {}
                        Err(SymbolicError::SolverUnknown) => self.defer_solver_unknown(),
                        Err(err) => return Err(err),
                    }
                }
            }
        }
        Ok(None)
    }

    fn equivalence_root(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        calldata: SymbolicCalldata,
        code: &SymCode,
    ) -> PathState {
        let mut root =
            PathState::new(&mut self.cx, input.address, input.sender, U256::ZERO, calldata, false);
        root.apply_executor_env(&mut self.cx, &input.executor);
        root.world.set_storage_layout(self.config.storage_layout);
        root.world.clear_transaction_scoped_state();
        root.world.install_code(input.address, code.clone());
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
        address: Address,
        state_a: &mut PathState,
        end_a: &EquivalenceEnd,
        state_b: &mut PathState,
        end_b: &EquivalenceEnd,
    ) -> Result<SymBoolExpr, SymbolicError> {
        if end_a.success != end_b.success {
            return Ok(SymBoolExpr::constant(&mut self.cx, true));
        }
        let mut diffs =
            vec![return_data_mismatch(&mut self.cx, &end_a.return_data, &end_b.return_data)];
        if end_a.success {
            // Both runs share one pre-state, so storage can only differ at a slot one of them
            // wrote.
            let mut keys = state_a.world.storage_write_keys(address);
            for key in state_b.world.storage_write_keys(address) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
            for key in keys {
                let concrete = key.as_const();
                let left =
                    state_a.world.sload(&mut self.cx, executor, address, key.clone(), concrete)?;
                let right = state_b.world.sload(&mut self.cx, executor, address, key, concrete)?;
                diffs.push(SymBoolExpr::eq(&mut self.cx, left, right).not(&mut self.cx));
            }
        }
        Ok(SymBoolExpr::or(&mut self.cx, diffs))
    }

    fn materialize_equivalence(
        &mut self,
        input: &SymbolicEquivalenceInput<'_>,
        calldata: &SymbolicCalldata,
        state: &PathState,
        constraints: &[SymBoolExpr],
    ) -> Result<SymbolicEquivalenceCounterexample, SymbolicError> {
        let replayable_storage = state.world.replay_storage_symbols();
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
        let storage = state.world.replay_storage_assignments(&model)?;
        Ok(SymbolicEquivalenceCounterexample { calldata: calldata_bytes, args, storage })
    }
}

struct SymbolicEquivalenceCounterexample {
    calldata: Bytes,
    args: Vec<DynSolValue>,
    storage: Vec<SymbolicStorageAssignment>,
}

/// Returns the condition under which two return buffers differ in length or in a byte in range.
fn return_data_mismatch(cx: &mut SymCx, a: &SymReturnData, b: &SymReturnData) -> SymBoolExpr {
    let mut diffs = vec![SymBoolExpr::eq(cx, a.len_word.clone(), b.len_word.clone()).not(cx)];
    for idx in 0..a.len().min(b.len()) {
        let left = a.byte(cx, idx);
        let right = b.byte(cx, idx);
        let differ = SymBoolExpr::eq(cx, left, right).not(cx);
        if differ.as_const() == Some(false) {
            continue;
        }
        let in_range = SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ugt, &a.len_word, U256::from(idx));
        diffs.push(SymBoolExpr::and(cx, vec![in_range, differ]));
    }
    SymBoolExpr::or(cx, diffs)
}
