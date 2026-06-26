use super::*;

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
        let calldata = SymbolicCalldata::selector_only(invariant)?;
        let call_data = calldata.call_data();
        let constraints = calldata.into_constraints();
        let outcomes = self.execute_sequence_call(
            executor,
            state,
            invariant_address,
            sender,
            invariant,
            call_data,
            constraints,
            completed_paths,
        )?;

        let mut checked = Vec::new();
        for mut outcome in outcomes {
            if !matches!(outcome.status, TopLevelCallStatus::Success) {
                outcome.status = TopLevelCallStatus::Failure;
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            if self.invariant_return_failed(invariant, &outcome.return_data, &mut outcome.state)? {
                checked.push(InvariantCheckOutcome { failed: true, state: outcome.state });
                continue;
            }

            let Some(after_invariant) = after_invariant else {
                checked.push(InvariantCheckOutcome { failed: false, state: outcome.state });
                continue;
            };

            let after_calldata = SymbolicCalldata::selector_only(after_invariant)?;
            for after_outcome in self.execute_sequence_call(
                executor,
                outcome.state.clone(),
                invariant_address,
                sender,
                after_invariant,
                after_calldata.call_data(),
                after_calldata.constraints().to_vec(),
                completed_paths,
            )? {
                checked.push(InvariantCheckOutcome {
                    failed: !matches!(after_outcome.status, TopLevelCallStatus::Success),
                    state: after_outcome.state,
                });
            }
        }
        Ok(checked)
    }

    pub(super) fn invariant_return_failed(
        &mut self,
        invariant: &Function,
        return_data: &SymReturnData,
        state: &mut PathState,
    ) -> Result<bool, SymbolicError> {
        if invariant.outputs.is_empty() {
            return Ok(false);
        }
        if invariant.outputs.len() != 1 || invariant.outputs[0].selector_type().as_ref() != "bool" {
            return Ok(false);
        }
        if return_data.len() < 32 {
            return Ok(true);
        }

        let pass = return_data.load_word(0)?.nonzero_bool();
        let fail = pass.clone().not();
        match fail.as_const() {
            Some(true) => Ok(true),
            Some(false) => Ok(false),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(fail);
                if self.solver.is_sat(&constraints)? {
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
        constraints: Vec<BoolExpr>,
        completed_paths: &mut usize,
    ) -> Result<Vec<TopLevelCallOutcome>, SymbolicError> {
        state.world.clear_transaction_scoped_state();
        let code = state.world.extcode(executor, target)?;
        let jumpdests = analyze_jumpdests(&code);
        state.call_depth = 0;
        state.origin = sender;
        state.origin_word = SymWord::constant(address_word(sender));
        state.frame =
            CallFrame::new(target, target, target, sender, SymWord::zero(), false, calldata);
        state.constraints.extend(constraints);

        let mut worklist = VecDeque::from([state]);
        let mut outcomes = Vec::new();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;

        while let Some(mut state) = pop_worklist(&mut worklist, self.config.exploration_order) {
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }
            let _path_span =
                trace_span!("symbolic_path", completed_paths, worklist_size = worklist.len())
                    .entered();
            trace!(completed_paths, worklist_size = worklist.len(), "exploring symbolic path");

            loop {
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;

                let Some(op) = code.opcode(state.pc)? else {
                    *completed_paths += 1;
                    outcomes.push(TopLevelCallOutcome {
                        status: if state.expectations_satisfied() {
                            TopLevelCallStatus::Success
                        } else {
                            TopLevelCallStatus::Failure
                        },
                        return_data: state.return_data.clone(),
                        state,
                    });
                    break;
                };

                let _step_span = trace_span!("symbolic_step", pc = state.pc - 1, op).entered();
                match self.step(
                    executor,
                    &code,
                    &jumpdests,
                    &mut state,
                    &mut worklist,
                    completed_paths,
                    op,
                )? {
                    StepOutcome::Continue => {}
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: if state.expectations_satisfied() {
                                TopLevelCallStatus::Success
                            } else {
                                TopLevelCallStatus::Failure
                            },
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Revert => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: TopLevelCallStatus::Revert,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::Failure => {
                        *completed_paths += 1;
                        outcomes.push(TopLevelCallOutcome {
                            status: TopLevelCallStatus::Failure,
                            return_data: state.return_data.clone(),
                            state,
                        });
                        break;
                    }
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                }
            }
        }

        Ok(outcomes)
    }

    pub(super) fn materialize_sequence(
        &mut self,
        steps: &[SequenceStepTemplate],
        state: &PathState,
    ) -> Result<Vec<SymbolicInvariantStep>, SymbolicError> {
        let model = self.solver.model(&state.constraints)?;
        steps
            .iter()
            .map(|step| {
                let args = step.calldata.model_to_args(&model)?;
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
            .collect()
    }
}
