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
        let outcomes = self
            .execute_invariant_call(
                executor,
                state,
                invariant_address,
                sender,
                invariant,
                completed_paths,
                false,
            )?
            .outcomes;

        let mut checked = Vec::new();
        for mut outcome in outcomes {
            if !matches!(outcome.status, CallStatus::Success) {
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            if self.invariant_return_failed(invariant, &mut outcome.state)? {
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            let Some(after_invariant) = after_invariant else {
                checked.push(InvariantCheckOutcome { failed: false, state: outcome.state });
                continue;
            };

            let after_calldata = SymbolicCalldata::selector_only(&mut self.cx, after_invariant)?;
            let calldata = after_calldata.call_data(&mut self.cx);
            let constraints = after_calldata.constraints().to_vec();
            for after_outcome in self
                .execute_sequence_call(
                    executor,
                    outcome.state,
                    invariant_address,
                    sender,
                    after_invariant,
                    calldata,
                    constraints,
                    completed_paths,
                    false,
                )?
                .outcomes
            {
                checked.push(InvariantCheckOutcome {
                    failed: !matches!(after_outcome.status, CallStatus::Success),
                    state: after_outcome.state,
                });
            }
        }
        Ok(checked)
    }

    #[expect(clippy::too_many_arguments)]
    fn execute_invariant_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: PathState,
        invariant_address: Address,
        sender: Address,
        invariant: &Function,
        completed_paths: &mut usize,
        preserve_completed: bool,
    ) -> Result<CallPathOutcomes, SymbolicError> {
        let calldata = SymbolicCalldata::selector_only(&mut self.cx, invariant)?;
        let call_data = calldata.call_data(&mut self.cx);
        let constraints = calldata.into_constraints();
        self.execute_sequence_call(
            executor,
            state,
            invariant_address,
            sender,
            invariant,
            call_data,
            constraints,
            completed_paths,
            preserve_completed,
        )
    }

    pub(super) fn search_invariant_candidates_inner<FEN: FoundryEvmNetwork>(
        &mut self,
        input: &SymbolicInvariantCandidateInput<'_, FEN>,
        candidates: &mut Vec<SymbolicInvariantCandidate>,
        limitation: &mut Option<SymbolicInvariantSearchLimitation>,
    ) -> Result<(), SymbolicError> {
        if input.invariants.is_empty() {
            return Err(SymbolicError::Unsupported("symbolic invariant has no predicates"));
        }
        let mut completed_paths = 0;

        let mut initial_state = PathState::empty(
            &mut self.cx,
            input.invariant_address,
            input.handler_sender,
            input.ffi_enabled,
        );
        initial_state.apply_executor_env(&mut self.cx, input.executor);
        initial_state.world.set_storage_layout(self.config.storage_layout);

        let calldatas = SymbolicCalldata::variants_with_prefix(
            &input.target.function,
            &self.config,
            &mut self.cx,
            "frontier_handler",
        )?;
        'variants: for calldata in calldatas {
            self.check_timeout()?;
            let step = SequenceStepTemplate {
                sender: input.handler_sender,
                address: input.target.address,
                contract_name: input.target.contract_name.clone(),
                function: input.target.function.clone(),
                calldata,
            };
            let call_data = step.calldata.call_data(&mut self.cx);
            let constraints = step.calldata.constraints().to_vec();
            let handler = match self.execute_sequence_call(
                input.executor,
                initial_state.clone(),
                input.target.address,
                input.handler_sender,
                &input.target.function,
                call_data,
                constraints,
                &mut completed_paths,
                true,
            ) {
                Ok(outcomes) => outcomes,
                Err(error) => {
                    if record_candidate_limitation(limitation, error) {
                        break;
                    }
                    continue;
                }
            };
            let stop_after_handler = handler
                .limitation
                .is_some_and(|error| record_candidate_limitation(limitation, error));

            for outcome in handler.outcomes {
                if !matches!(outcome.status, CallStatus::Success) {
                    continue;
                }
                for (invariant_idx, invariant) in input.invariants.iter().enumerate() {
                    self.check_timeout()?;
                    let predicate = match self.execute_invariant_call(
                        input.executor,
                        outcome.state.clone(),
                        input.invariant_address,
                        CALLER,
                        invariant,
                        &mut completed_paths,
                        true,
                    ) {
                        Ok(outcomes) => outcomes,
                        Err(error) => {
                            if record_candidate_limitation(limitation, error) {
                                break 'variants;
                            }
                            continue;
                        }
                    };
                    let stop_after_predicate = predicate
                        .limitation
                        .is_some_and(|error| record_candidate_limitation(limitation, error));
                    for predicate_outcome in predicate.outcomes {
                        if matches!(predicate_outcome.status, CallStatus::Success) {
                            continue;
                        }
                        match self.materialize_sequence(
                            std::slice::from_ref(&step),
                            &predicate_outcome.state,
                        ) {
                            Ok((mut sequence, storage)) => {
                                let step =
                                    sequence.pop().expect("one handler template produces one step");
                                candidates.push(SymbolicInvariantCandidate {
                                    invariant_idx,
                                    step,
                                    storage,
                                });
                            }
                            Err(error) => {
                                if record_candidate_limitation(limitation, error) {
                                    break 'variants;
                                }
                            }
                        }
                    }
                    if stop_after_predicate {
                        break 'variants;
                    }
                }
            }
            if stop_after_handler {
                break;
            }
        }

        Ok(())
    }

    fn invariant_return_failed(
        &mut self,
        invariant: &Function,
        state: &mut PathState,
    ) -> Result<bool, SymbolicError> {
        if invariant.outputs.is_empty() {
            return Ok(false);
        }
        if invariant.outputs.len() != 1 || invariant.outputs[0].selector_type().as_ref() != "bool" {
            return Ok(false);
        }
        if state.return_data.len() < 32 {
            return Ok(true);
        }

        let pass = state.return_data.load_word(&mut self.cx, 0)?.nonzero_bool(&mut self.cx);
        let fail = pass.clone().not(&mut self.cx);
        match fail.as_const() {
            Some(true) => Ok(true),
            Some(false) => Ok(false),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(fail);
                if self.is_sat_with_state(state, &constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    state.constraints.push(pass);
                    Ok(false)
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn execute_sequence_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        mut state: PathState,
        target: Address,
        sender: Address,
        _function: &Function,
        calldata: SymCalldata,
        constraints: Vec<SymBoolExpr>,
        completed_paths: &mut usize,
        preserve_completed: bool,
    ) -> Result<CallPathOutcomes, SymbolicError> {
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
        self.execute_call_paths(
            executor,
            state,
            &code,
            completed_paths,
            CallPathKind::Sequence,
            preserve_completed,
        )
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
