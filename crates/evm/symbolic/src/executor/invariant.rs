use super::*;

fn record_candidate_limitation(
    limitation: &mut Option<SymbolicInvariantSearchLimitation>,
    error: SymbolicError,
) -> bool {
    let search_exhausted = matches!(
        error,
        SymbolicError::Timeout(_) | SymbolicError::Solver(_) | SymbolicError::SolverQueryLimit(_)
    );
    limitation.get_or_insert_with(|| error.into());
    search_exhausted
}

impl SymbolicExecutor {
    #[expect(clippy::too_many_arguments)]
    pub(super) fn execute_invariant_check<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: PathState,
        invariant_address: Address,
        sender: Address,
        invariant: &Function,
        after_invariant: Option<&Function>,
        completed_paths: &mut usize,
    ) -> Result<Vec<InvariantCheckOutcome>, SymbolicError> {
        let mut call =
            self.prepare_invariant_call(executor, state, invariant_address, sender, invariant)?;

        let mut checked = Vec::new();
        while let Some(outcome) =
            self.execute_sequence_call_next(executor, &mut call, completed_paths)?
        {
            if !matches!(outcome.status, CallStatus::Success) {
                return Ok(vec![InvariantCheckOutcome { failed: true, state: outcome.state }]);
            }

            let Some(after_invariant) = after_invariant else {
                checked.push(InvariantCheckOutcome { failed: false, state: outcome.state });
                continue;
            };

            let mut after_call = self.prepare_invariant_call(
                executor,
                outcome.state,
                invariant_address,
                sender,
                after_invariant,
            )?;
            while let Some(after_outcome) =
                self.execute_sequence_call_next(executor, &mut after_call, completed_paths)?
            {
                let failed = !matches!(after_outcome.status, CallStatus::Success);
                let checked_outcome = InvariantCheckOutcome { failed, state: after_outcome.state };
                if failed {
                    return Ok(vec![checked_outcome]);
                }
                checked.push(checked_outcome);
            }
        }
        Ok(checked)
    }

    fn prepare_invariant_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: PathState,
        invariant_address: Address,
        sender: Address,
        invariant: &Function,
    ) -> Result<SequenceCall, SymbolicError> {
        let calldata = SymbolicCalldata::selector_only(&mut self.cx, invariant)?;
        let call_data = calldata.call_data(&mut self.cx);
        let constraints = calldata.into_constraints();
        self.prepare_sequence_call(
            executor,
            state,
            invariant_address,
            sender,
            invariant,
            call_data,
            constraints,
        )
    }

    pub(super) fn search_invariant_sequence_candidates_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: &SymbolicInvariantCandidateSequenceInput<'_, FEN>,
        candidates: &mut Vec<SymbolicInvariantSequenceCandidate>,
        limitation: &mut Option<SymbolicInvariantSearchLimitation>,
    ) -> Result<(), SymbolicError> {
        if input.invariants.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant has no predicates"));
        }
        let Some(first_call) = input.calls.first() else {
            return Err(SymbolicError::Unsupported("symbolic invariant has no candidate calls"));
        };

        let mut initial_state = PathState::empty(
            &mut self.cx,
            input.invariant_address,
            first_call.sender,
            input.ffi_enabled,
        );
        initial_state.apply_executor_env(&mut self.cx, input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);

        let calldata_variants = input
            .calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                let indexed_prefix =
                    (input.calls.len() > 1).then(|| format!("frontier_handler_{index}"));
                SymbolicCalldata::variants_with_prefix(
                    &call.target.function,
                    &self.config,
                    &mut self.cx,
                    indexed_prefix.as_deref().unwrap_or("frontier_handler"),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut completed_paths = 0;
        let _ = self.search_invariant_candidate_calls(
            input,
            &calldata_variants,
            0,
            initial_state,
            Vec::new(),
            candidates,
            limitation,
            &mut completed_paths,
        )?;
        Ok(())
    }

    #[expect(clippy::too_many_arguments)]
    fn search_invariant_candidate_calls<FEN: FoundryEvmNetwork>(
        &mut self,
        input: &SymbolicInvariantCandidateSequenceInput<'_, FEN>,
        calldata_variants: &[Vec<SymbolicCalldata>],
        call_index: usize,
        state: PathState,
        steps: Vec<SequenceStepTemplate>,
        candidates: &mut Vec<SymbolicInvariantSequenceCandidate>,
        limitation: &mut Option<SymbolicInvariantSearchLimitation>,
        completed_paths: &mut usize,
    ) -> Result<ControlFlow<()>, SymbolicError> {
        let call = input.calls[call_index];
        for calldata in calldata_variants[call_index].iter().cloned() {
            self.check_timeout()?;
            let step = SequenceStepTemplate {
                sender: call.sender,
                address: call.target.address,
                contract_name: call.target.contract_name.clone(),
                function: call.target.function.clone(),
                calldata,
            };
            let call_data = step.calldata.call_data(&mut self.cx);
            let constraints = step.calldata.constraints().to_vec();
            let mut handler = match self.prepare_sequence_call(
                input.executor,
                state.clone(),
                call.target.address,
                call.sender,
                &call.target.function,
                call_data,
                constraints,
            ) {
                Ok(call) => call,
                Err(error) => {
                    if record_candidate_limitation(limitation, error) {
                        return Ok(ControlFlow::Break(()));
                    }
                    continue;
                }
            };
            loop {
                let outcome = match self.execute_sequence_call_next(
                    input.executor,
                    &mut handler,
                    completed_paths,
                ) {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => break,
                    Err(error) => {
                        if record_candidate_limitation(limitation, error) {
                            return Ok(ControlFlow::Break(()));
                        }
                        break;
                    }
                };
                if !matches!(outcome.status, CallStatus::Success) {
                    continue;
                }
                let mut next_steps = steps.clone();
                next_steps.push(step.clone());
                let next_call_index = call_index + 1;
                let flow = if next_call_index < input.calls.len() {
                    self.search_invariant_candidate_calls(
                        input,
                        calldata_variants,
                        next_call_index,
                        outcome.state,
                        next_steps,
                        candidates,
                        limitation,
                        completed_paths,
                    )?
                } else {
                    self.search_invariant_candidate_predicates(
                        input,
                        outcome.state,
                        &next_steps,
                        candidates,
                        limitation,
                        completed_paths,
                    )?
                };
                if flow.is_break() {
                    return Ok(flow);
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    fn search_invariant_candidate_predicates<FEN: FoundryEvmNetwork>(
        &mut self,
        input: &SymbolicInvariantCandidateSequenceInput<'_, FEN>,
        handler_state: PathState,
        steps: &[SequenceStepTemplate],
        candidates: &mut Vec<SymbolicInvariantSequenceCandidate>,
        limitation: &mut Option<SymbolicInvariantSearchLimitation>,
        completed_paths: &mut usize,
    ) -> Result<ControlFlow<()>, SymbolicError> {
        for (invariant_idx, invariant) in input.invariants.iter().enumerate() {
            self.check_timeout()?;
            let mut predicate = match self.prepare_invariant_call(
                input.executor,
                handler_state.clone(),
                input.invariant_address,
                CALLER,
                invariant,
            ) {
                Ok(call) => call,
                Err(error) => {
                    if record_candidate_limitation(limitation, error) {
                        return Ok(ControlFlow::Break(()));
                    }
                    continue;
                }
            };
            let mut stop_after_predicate = false;
            let mut candidate_states = Vec::new();
            loop {
                let predicate_outcome = match self.execute_sequence_call_next(
                    input.executor,
                    &mut predicate,
                    completed_paths,
                ) {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => break,
                    Err(error) => {
                        stop_after_predicate = record_candidate_limitation(limitation, error);
                        break;
                    }
                };
                if !matches!(predicate_outcome.status, CallStatus::Success) {
                    candidate_states.push(predicate_outcome.state);
                    continue;
                }
                let Some(after_invariant) = input.after_invariant else {
                    continue;
                };

                // Concrete invariant checks do not commit predicate state before invoking
                // `afterInvariant`. Retain its path constraints while restoring the unchanged
                // post-handler world.
                let mut after_state = handler_state.clone();
                after_state.constraints = predicate_outcome.state.constraints;
                let mut after = match self.prepare_invariant_call(
                    input.executor,
                    after_state,
                    input.invariant_address,
                    CALLER,
                    after_invariant,
                ) {
                    Ok(call) => call,
                    Err(error) => {
                        if record_candidate_limitation(limitation, error) {
                            stop_after_predicate = true;
                            break;
                        }
                        continue;
                    }
                };
                loop {
                    match self.execute_sequence_call_next(
                        input.executor,
                        &mut after,
                        completed_paths,
                    ) {
                        Ok(Some(outcome)) => {
                            if !matches!(outcome.status, CallStatus::Success) {
                                candidate_states.push(outcome.state);
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            if record_candidate_limitation(limitation, error) {
                                stop_after_predicate = true;
                            }
                            break;
                        }
                    }
                }
                if stop_after_predicate {
                    break;
                }
            }

            for state in candidate_states {
                match self.materialize_sequence(steps, &state) {
                    Ok((steps, storage)) => {
                        candidates.push(SymbolicInvariantSequenceCandidate {
                            invariant_idx,
                            steps,
                            storage,
                        });
                    }
                    Err(error) => {
                        if record_candidate_limitation(limitation, error) {
                            return Ok(ControlFlow::Break(()));
                        }
                    }
                }
            }
            if stop_after_predicate {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn prepare_sequence_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        mut state: PathState,
        target: Address,
        sender: Address,
        _function: &Function,
        calldata: SymCalldata,
        constraints: Vec<SymBoolExpr>,
    ) -> Result<SequenceCall, SymbolicError> {
        state.world.clear_transaction_scoped_state();
        state.mapping_hook_keccak_preimages.clear();
        let code = state.world.extcode(&mut self.cx, executor, target)?;
        state.call_depth = 0;
        state.origin = sender;
        state.origin_word = SymExpr::constant(&mut self.cx, address_word(sender));
        let callvalue = SymExpr::zero(&mut self.cx);
        state.frame =
            CallFrame::new(&mut self.cx, target, target, sender, callvalue, false, calldata);
        state.constraints.extend(constraints);
        Ok(SequenceCall {
            code,
            worklist: VecDeque::from([state]),
            deferred_worklist: VecDeque::new(),
        })
    }

    pub(super) fn execute_sequence_call_next<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        call: &mut SequenceCall,
        completed_paths: &mut usize,
    ) -> Result<Option<CallOutcome>, SymbolicError> {
        if call.worklist.is_empty() && call.deferred_worklist.is_empty() {
            return Ok(None);
        }
        let mut outcomes = self.execute_call_path_batch(
            executor,
            &call.code,
            &mut call.worklist,
            &mut call.deferred_worklist,
            completed_paths,
            CallPathKind::Sequence,
        )?;
        debug_assert!(outcomes.len() <= 1);
        Ok(outcomes.pop())
    }

    pub(super) fn materialize_sequence(
        &mut self,
        steps: &[SequenceStepTemplate],
        state: &PathState,
    ) -> Result<(Vec<SymbolicInvariantStep>, Vec<SymbolicStorageAssignment>), SymbolicError> {
        let replayable_storage = state.world.replay_storage_symbols();
        let model = self.solver.model_with_replayable_storage(
            &mut self.cx,
            &state.constraints,
            &replayable_storage,
        )?;
        let sequence = steps
            .iter()
            .map(|step| {
                let args = step.calldata.model_to_args(&mut self.cx, &model)?;
                let calldata = Bytes::from(step.function.abi_encode_input(&args)?);
                Ok(SymbolicInvariantStep {
                    sender: step.sender,
                    address: step.address,
                    contract_name: step.contract_name.clone(),
                    function_name: step.function.name.clone(),
                    signature: step.function.signature(),
                    args,
                    calldata,
                })
            })
            .collect::<Result<Vec<_>, SymbolicError>>()?;
        let storage = state.world.replay_storage_assignments(&model)?;
        Ok((sequence, storage))
    }
}
