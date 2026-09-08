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
        let calldata = SymbolicCalldata::selector_only(&mut self.cx, invariant)?;
        let call_data = calldata.call_data(&mut self.cx);
        let constraints = calldata.into_constraints();
        let mut call = self.prepare_sequence_call(
            executor,
            state,
            invariant_address,
            sender,
            invariant,
            call_data,
            constraints,
        )?;

        let mut checked = Vec::new();
        while let Some(mut outcome) =
            self.execute_sequence_call_next(executor, &mut call, completed_paths)?
        {
            if !matches!(outcome.status, CallStatus::Success)
                || self.invariant_return_failed(invariant, &mut outcome.state)?
            {
                return Ok(vec![InvariantCheckOutcome { failed: true, state: outcome.state }]);
            }

            let Some(after_invariant) = after_invariant else {
                checked.push(InvariantCheckOutcome { failed: false, state: outcome.state });
                continue;
            };

            let after_calldata = SymbolicCalldata::selector_only(&mut self.cx, after_invariant)?;
            let calldata = after_calldata.call_data(&mut self.cx);
            let constraints = after_calldata.constraints().to_vec();
            let mut after_call = self.prepare_sequence_call(
                executor,
                outcome.state,
                invariant_address,
                sender,
                after_invariant,
                calldata,
                constraints,
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

    pub(super) fn invariant_return_failed(
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
