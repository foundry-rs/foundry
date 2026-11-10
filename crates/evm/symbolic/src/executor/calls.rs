use super::*;
use alloy_primitives::{Log, LogData};
use alloy_sol_types::{Revert, SolError, SolType, sol_data};
use foundry_evm::inspectors::cheatcodes::unmatched_emit_message;
use foundry_cheatcodes_spec::Vm::VmCalls;

impl SymbolicExecutor {
    pub(super) fn call(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
    ) -> Result<StepOutcome, SymbolicError> {
        let pre_call_state = (!state.function_mocks.is_empty()
            || !state.expected_calls.is_empty()
            || !state.call_mocks.is_empty()
            || (state.is_static && matches!(kind, CallKind::Call)))
        .then(|| state.clone());
        let call_pc = state.pc.saturating_sub(1);

        let has_value = matches!(kind, CallKind::Call | CallKind::CallCode);
        let in_offset_idx = if has_value { 3 } else { 2 };
        let in_offset = state.stack.peek(in_offset_idx)?.clone();
        let in_size = state.stack.peek(in_offset_idx + 1)?.clone();
        let out_offset = state.stack.peek(in_offset_idx + 2)?.clone();
        let out_size = state.stack.peek(in_offset_idx + 3)?.clone();
        if let Some(outcome) =
            self.guard_memory_range(executor, state, worklist, &in_offset, &in_size)?
        {
            return Ok(outcome);
        }
        if let Some(outcome) =
            self.guard_memory_range(executor, state, worklist, &out_offset, &out_size)?
        {
            return Ok(outcome);
        }

        let gas = state.stack.pop()?;
        let target = state.stack.pop()?;
        ensure_expr_not_gasleft(&target)?;
        let target = target.address_operand(&mut self.cx);
        let target_address = state.world.resolve_address(&target);
        // Gas is not modeled, except for the concrete gas a call gives a precompile, whose cost
        // the engine checks.
        if !gas.is_raw_gasleft()
            && !(gas.as_const().is_some()
                && target_address.is_some_and(|to| is_precompile_for_spec(to, executor.spec_id))
                && state.function_mocks.is_empty()
                && state.call_mocks.is_empty()
                && state.expected_calls.is_empty())
        {
            return Err(SymbolicError::Unsupported("explicit CALL gas limit not modeled"));
        }
        let value = match (kind, target_address) {
            (CallKind::Call, Some(to))
                if to == CHEATCODE_ADDRESS || to == SYMBOLIC_VM_COMPAT_ADDRESS =>
            {
                let value = state.stack.pop()?;
                let value =
                    state.expect_constrained_word(&mut self.cx, value, "symbolic CALL value")?;
                SymExpr::constant(&mut self.cx, value)
            }
            (CallKind::Call, _) => state.stack.pop()?,
            (CallKind::CallCode, _) => state.stack.pop()?,
            (CallKind::StaticCall | CallKind::DelegateCall, _) => SymExpr::zero(&mut self.cx),
        };
        ensure_expr_not_gasleft(&value)?;
        let in_offset = state.stack.pop()?;
        ensure_expr_not_gasleft(&in_offset)?;
        let in_size = state.stack.pop()?;
        ensure_expr_not_gasleft(&in_size)?;
        let in_size = match state.constrained_usize_checked(&mut self.cx, &in_size) {
            Some(Ok(size)) => BoundedCopySize::Concrete(size),
            Some(Err(_)) => {
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = self.solver_upper_bound_usize(
                    state,
                    &in_size,
                    max_limit,
                    "symbolic CALL input size",
                )?;
                BoundedCopySize::Symbolic { size: in_size, max_size }
            }
        };
        let out_offset = state.stack.pop()?;
        ensure_expr_not_gasleft(&out_offset)?;
        let out_size = state.stack.pop()?;
        ensure_expr_not_gasleft(&out_size)?;
        let out_size = match state.constrained_usize_checked(&mut self.cx, &out_size) {
            Some(Ok(size)) => BoundedCopySize::Concrete(size),
            Some(Err(_)) => {
                return Ok(StepOutcome::Revert);
            }
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = self.solver_upper_bound_usize(
                    state,
                    &out_size,
                    max_limit,
                    "symbolic CALL output size",
                )?;
                BoundedCopySize::Symbolic { size: out_size, max_size }
            }
        };

        in_size.expand_memory(&mut self.cx, &mut state.memory, in_offset.clone());
        out_size.expand_memory(&mut self.cx, &mut state.memory, out_offset.clone());

        if state.is_static && matches!(kind, CallKind::Call) {
            match state.constrained_word(&mut self.cx, &value) {
                Some(value) if value.is_zero() => {}
                Some(_) => {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    return Ok(StepOutcome::ExceptionalHalt);
                }
                None => {
                    let zero = SymBoolExpr::eq_word_const(&mut self.cx, &value, U256::ZERO);
                    let (zero_constraints, zero_sat) =
                        self.constraints_with_condition(state, zero.clone())?;
                    let nonzero = zero.not(&mut self.cx);
                    let (nonzero_constraints, nonzero_sat) =
                        self.constraints_with_condition(state, nonzero)?;
                    match (zero_sat, nonzero_sat) {
                        (true, true) => {
                            let mut zero_state = pre_call_state
                                .as_ref()
                                .expect("static calls preserve pre-call state")
                                .clone();
                            zero_state.pc = call_pc;
                            zero_state.constraints = zero_constraints;
                            worklist.push_back(zero_state);
                            state.constraints = nonzero_constraints;
                            state.return_data = SymReturnData::empty(&mut self.cx);
                            return Ok(StepOutcome::ExceptionalHalt);
                        }
                        (true, false) => state.constraints = zero_constraints,
                        (false, true) => {
                            state.constraints = nonzero_constraints;
                            state.return_data = SymReturnData::empty(&mut self.cx);
                            return Ok(StepOutcome::ExceptionalHalt);
                        }
                        (false, false) => return Ok(StepOutcome::AssumeRejected),
                    }
                }
            }
        }

        let call_input = in_size.read_from_memory(&mut self.cx, &state.memory, in_offset.clone());
        // Gas is not modeled, so calldata derived from `GAS` / `gasleft()` must fail closed instead
        // of handing the callee a fabricated gas value.
        if call_input.contains_gasleft(&mut self.cx) {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }

        if let Some(to) = target_address {
            if !state.function_mocks.is_empty() {
                let pre_call_state =
                    pre_call_state.as_ref().expect("function mocks require pre-call state");
                if self.branch_symbolic_function_mock_if_needed(
                    state,
                    worklist,
                    pre_call_state,
                    call_pc,
                    to,
                    &call_input,
                )? {
                    return Ok(StepOutcome::Forked);
                }
            }
            let code_address = if state.function_mocks.is_empty() {
                to
            } else {
                self.function_mock_target(state, to, &call_input)?.unwrap_or(to)
            };
            if !state.expected_calls.is_empty() || !state.call_mocks.is_empty() {
                let pre_call_state =
                    pre_call_state.as_ref().expect("call mocks require pre-call state");
                if self.branch_symbolic_call_value_if_needed(
                    state,
                    worklist,
                    pre_call_state,
                    call_pc,
                    code_address,
                    &value,
                    &gas,
                    &call_input,
                )? {
                    return Ok(StepOutcome::Forked);
                }
            }
            let concrete_value = self.transfer_value(state, kind, &value);
            if !state.expected_calls.is_empty() || !state.call_mocks.is_empty() {
                let pre_call_state =
                    pre_call_state.as_ref().expect("call mocks require pre-call state");
                if self.branch_symbolic_call_match_if_needed(
                    state,
                    worklist,
                    pre_call_state,
                    call_pc,
                    code_address,
                    concrete_value,
                    &gas,
                    &call_input,
                )? {
                    return Ok(StepOutcome::Forked);
                }
            }
            return self.call_concrete_target(
                executor,
                state,
                worklist,
                completed_paths,
                kind,
                to,
                Some(target),
                value,
                gas,
                in_offset,
                in_size,
                out_offset,
                out_size,
            );
        }

        self.call_symbolic_target(
            executor,
            state,
            worklist,
            completed_paths,
            kind,
            target,
            value,
            gas,
            in_offset,
            in_size,
            out_offset,
            out_size,
        )
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn branch_symbolic_call_value_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        code_address: Address,
        value: &SymExpr,
        gas: &SymExpr,
        call_input: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&mut self.cx, value).is_some() {
            return Ok(false);
        }

        for candidate in self.call_value_candidates(state, code_address, gas, call_input)? {
            let eq = SymBoolExpr::eq_word_const(&mut self.cx, value, candidate);
            let (eq_constraints, eq_sat) = self.constraints_with_condition(state, eq.clone())?;
            let eq_not = eq.not(&mut self.cx);
            let (neq_constraints, neq_sat) = self.constraints_with_condition(state, eq_not)?;

            match (eq_sat, neq_sat) {
                (true, true) => {
                    let mut eq_state = pre_call_state.clone();
                    eq_state.pc = call_pc;
                    eq_state.constraints = eq_constraints;
                    worklist.push_back(eq_state);

                    let mut neq_state = pre_call_state.clone();
                    neq_state.pc = call_pc;
                    neq_state.constraints = neq_constraints;
                    worklist.push_back(neq_state);
                    return Ok(true);
                }
                (true, false) => {
                    state.constraints = eq_constraints;
                    return Ok(false);
                }
                (false, true) => {
                    state.constraints = neq_constraints;
                }
                (false, false) => return Ok(false),
            }
        }

        Ok(false)
    }

    pub(super) fn branch_symbolic_function_mock_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        callee: Address,
        calldata: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        for condition in self.function_mock_conditions(state, callee, calldata) {
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(super) fn observe_expected_call(
        &mut self,
        state: &mut PathState,
        callee: Address,
        value: Option<U256>,
        gas: &SymExpr,
        calldata: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        if state.expected_calls.is_empty() {
            return Ok(true);
        }
        // Like Forge, the call counts for every expectation it matches.
        let mut observed = true;
        for idx in 0..state.expected_calls.len() {
            if let Some(constraints) = self.expected_call_match_constraints(
                state,
                &state.expected_calls[idx],
                callee,
                value,
                gas,
                calldata,
            )? {
                state.constraints = constraints;
                observed &= state.expected_calls[idx].observe();
            }
        }
        Ok(observed)
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn branch_symbolic_call_match_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        code_address: Address,
        value: Option<U256>,
        gas: &SymExpr,
        calldata: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        for condition in self.call_match_conditions(state, code_address, value, gas, calldata)? {
            if self.branch_symbolic_match_condition_if_needed(
                state,
                worklist,
                pre_call_state,
                call_pc,
                condition,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(super) fn take_call_mock(
        &mut self,
        state: &mut PathState,
        callee: Address,
        value: Option<U256>,
        calldata: &SymBytes,
    ) -> Result<Option<CallMockOutcome>, SymbolicError> {
        let Some((idx, constraints)) = self.select_call_mock(state, callee, value, calldata)?
        else {
            return Ok(None);
        };
        state.constraints = constraints;
        if !state.call_mocks[idx].answers() {
            return Ok(None);
        }
        Ok(Some(state.call_mocks[idx].next_outcome(&mut self.cx)))
    }

    /// Selects the most specific `vm.mockCall` matching this call, with the constraints of the
    /// match, before deciding whether it answers the call, as Forge does.
    fn select_call_mock(
        &mut self,
        state: &PathState,
        callee: Address,
        value: Option<U256>,
        calldata: &SymBytes,
    ) -> Result<Option<(usize, Vec<SymBoolExpr>)>, SymbolicError> {
        let mut best = None;
        for idx in 0..state.call_mocks.len() {
            let Some(constraints) = self.call_mock_match_constraints(
                state,
                &state.call_mocks[idx],
                callee,
                value,
                calldata,
            )?
            else {
                continue;
            };
            let specificity = state.call_mocks[idx].specificity();
            if best.as_ref().is_none_or(
                |(_, best_specificity, _): &(usize, (usize, bool), Vec<SymBoolExpr>)| {
                    specificity > *best_specificity
                },
            ) {
                best = Some((idx, specificity, constraints));
            }
        }
        Ok(best.map(|(idx, _, constraints)| (idx, constraints)))
    }

    /// Returns whether a `vm.mockCall` answers this call.
    fn has_call_mock(
        &mut self,
        state: &PathState,
        callee: Address,
        value: &SymExpr,
        calldata: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        let value = state.constrained_word(&mut self.cx, value);
        Ok(self
            .select_call_mock(state, callee, value, calldata)?
            .is_some_and(|(idx, _)| state.call_mocks[idx].answers()))
    }

    /// Returns the value that `vm.mockCall` and `vm.expectCall` value filters match. Like Forge's
    /// `CallInputs::transfer_value`, a DELEGATECALL has none.
    fn transfer_value(
        &mut self,
        state: &PathState,
        kind: CallKind,
        value: &SymExpr,
    ) -> Option<U256> {
        if matches!(kind, CallKind::DelegateCall) {
            return None;
        }
        state.constrained_word(&mut self.cx, value)
    }

    pub(super) fn branch_symbolic_match_condition_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        condition: SymBoolExpr,
    ) -> Result<bool, SymbolicError> {
        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        let mismatch_condition = condition.not(&mut self.cx);
        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, mismatch_condition)?;

        match (match_sat, mismatch_sat) {
            (true, true) => {
                let mut match_state = pre_call_state.clone();
                match_state.pc = call_pc;
                match_state.constraints = match_constraints;
                worklist.push_back(match_state);

                let mut mismatch_state = pre_call_state.clone();
                mismatch_state.pc = call_pc;
                mismatch_state.constraints = mismatch_constraints;
                worklist.push_back(mismatch_state);
                Ok(true)
            }
            (true, false) => {
                state.constraints = match_constraints;
                Ok(false)
            }
            (false, true) => {
                state.constraints = mismatch_constraints;
                Ok(false)
            }
            (false, false) => Ok(false),
        }
    }

    pub(super) fn function_mock_target(
        &mut self,
        state: &mut PathState,
        callee: Address,
        calldata: &SymBytes,
    ) -> Result<Option<Address>, SymbolicError> {
        for idx in (0..state.function_mocks.len()).rev() {
            if state.function_mocks[idx].calldata_len() != calldata.len() {
                continue;
            }
            let Some(condition) =
                state.function_mocks[idx].match_condition(&mut self.cx, callee, calldata)
            else {
                continue;
            };
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                state.constraints = constraints;
                return Ok(Some(state.function_mocks[idx].target()));
            }
        }
        for idx in (0..state.function_mocks.len()).rev() {
            if state.function_mocks[idx].calldata_len() != 4 {
                continue;
            }
            let Some(condition) =
                state.function_mocks[idx].match_condition(&mut self.cx, callee, calldata)
            else {
                continue;
            };
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                state.constraints = constraints;
                return Ok(Some(state.function_mocks[idx].target()));
            }
        }
        Ok(None)
    }

    pub(super) fn expected_call_match_constraints(
        &mut self,
        state: &PathState,
        expected: &ExpectedCall,
        callee: Address,
        value: Option<U256>,
        gas: &SymExpr,
        calldata: &SymBytes,
    ) -> Result<Option<Vec<SymBoolExpr>>, SymbolicError> {
        let Some(condition) =
            expected.match_condition(&mut self.cx, callee, value, gas, calldata)?
        else {
            return Ok(None);
        };
        self.constraints_for_condition(state, condition)
    }

    pub(super) fn call_mock_match_constraints(
        &mut self,
        state: &PathState,
        mock: &CallMock,
        callee: Address,
        value: Option<U256>,
        calldata: &SymBytes,
    ) -> Result<Option<Vec<SymBoolExpr>>, SymbolicError> {
        let Some(condition) = mock.match_condition(&mut self.cx, callee, value, calldata) else {
            return Ok(None);
        };
        self.constraints_for_condition(state, condition)
    }

    /// Returns whether `expected_revert_matches` holds.
    pub(super) fn expected_revert_matches(
        &mut self,
        state: &mut PathState,
        expected: &ExpectedRevert,
        reverter: Option<Address>,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let Some(condition) = expected.match_condition(&mut self.cx, reverter, return_data) else {
            return Ok(false);
        };

        let (match_constraints, match_sat) =
            self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let mismatch_condition = condition.not(&mut self.cx);
        let (mismatch_constraints, mismatch_sat) =
            self.constraints_with_condition(state, mismatch_condition)?;
        if mismatch_sat {
            state.constraints = mismatch_constraints;
            return Ok(false);
        }

        state.constraints = match_constraints;
        Ok(true)
    }

    /// Finishes a call that runs no callee code, such as a mock, a precompile, an account without
    /// code or a failed value transfer, applying a pending `vm.expectRevert` as Forge's
    /// `call_end` does. Like `join_call_outcome`, a mismatch fails the path.
    ///
    /// Like `join_call_outcome`, a revert under `vm.assumeNoRevert` skips `vm.expectRevert`, and
    /// a matching revert returns `FOUNDRY::ASSUME`.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn finish_codeless_call(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        kind: CallKind,
        status: CallStatus,
        mut return_data: SymReturnData,
        reverter: Address,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        let mut success = status == CallStatus::Success;
        // Like Forge, a call that does not revert checks a pending `vm.expectEmit` unless
        // `vm.expectRevert` handles it.
        let unmet_expected_emit = !matches!(status, CallStatus::Revert)
            && (state.expected_revert.is_none() || (success && executor.internal_expect_revert))
            && state.check_expected_emit_after_call(kind);
        let assumption = state.assume_no_revert_next_call.take();
        let assumed_revert = status == CallStatus::Revert && assumption.is_some();
        if status == CallStatus::Revert {
            state.first_reverted_callee.get_or_insert(reverter);
        }
        if let Some(assumption) = &assumption
            && assumed_revert
            && self.assume_no_revert_matches(
                state,
                assumption,
                assumption.reverted_by.unwrap_or(reverter),
                &return_data,
            )?
        {
            return_data = SymReturnData::from_concrete_bytes(&mut self.cx, MAGIC_ASSUME.to_vec());
        }
        // Like Forge, internal `expectRevert` mode keeps waiting after a successful call.
        if !assumed_revert
            && let Some(mut expected) = state.expected_revert.clone()
            && !(success && executor.internal_expect_revert)
        {
            if success {
                state.expected_revert = None;
                return_data =
                    error_string_return_data(&mut self.cx, "next call did not revert as expected");
            } else {
                // Like Forge, the reverter is the first call that failed since `vm.expectRevert`,
                // which is this call unless an earlier one failed.
                let first_failed_callee = state.first_failed_callee.or(Some(reverter));
                let reverter = expected.reverted_by(Some(reverter), first_failed_callee);
                if !self.expected_revert_matches(state, &expected, reverter, &return_data)? {
                    return Ok(StepOutcome::Failure);
                }
                let done = expected.consume_one();
                state.expected_revert = (!done).then_some(expected);
                return_data =
                    SymReturnData::from_concrete_bytes(&mut self.cx, DUMMY_CALL_OUTPUT.to_vec());
            }
            success = !success;
        } else if !success && !assumed_revert {
            // Like Forge, a failure not consumed here can be the reverter of a pending
            // `vm.expectRevert`.
            state.first_failed_callee.get_or_insert(reverter);
        }
        // Like Forge, the failure is a bare string; `classify_revert` reads it from the pending
        // expectation when the revert reaches the top frame.
        if unmet_expected_emit {
            success = false;
            return_data = self.unmet_expected_emit_return_data(executor, state);
        }
        state.return_data = return_data;
        state.copy_call_output_offset(&mut self.cx, out_offset, out_size)?;
        let success = SymExpr::constant(&mut self.cx, U256::from(success));
        state.stack.push(success)?;
        Ok(StepOutcome::Continue)
    }

    /// Returns whether a revert matches `vm.assumeNoRevert`, or `Unsupported` when both outcomes
    /// are feasible.
    pub(super) fn assume_no_revert_matches(
        &mut self,
        state: &PathState,
        assumption: &AssumeNoRevert,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let Some(filters) = &assumption.filters else {
            return Ok(true);
        };

        let conditions = filters
            .iter()
            .filter_map(|filter| filter.match_condition(&mut self.cx, Some(reverter), return_data))
            .collect::<Vec<_>>();
        if conditions.is_empty() {
            return Ok(false);
        }

        let condition = SymBoolExpr::or(&mut self.cx, conditions);
        let (_, match_sat) = self.constraints_with_condition(state, condition.clone())?;
        if !match_sat {
            return Ok(false);
        }

        let mismatch_condition = condition.not(&mut self.cx);
        let (_, mismatch_sat) = self.constraints_with_condition(state, mismatch_condition)?;
        if mismatch_sat {
            return Err(SymbolicError::Unsupported("ambiguous symbolic vm.assumeNoRevert match"));
        }

        Ok(true)
    }

    pub(super) fn constraints_for_condition(
        &mut self,
        state: &PathState,
        condition: SymBoolExpr,
    ) -> Result<Option<Vec<SymBoolExpr>>, SymbolicError> {
        let (constraints, sat) = self.constraints_with_condition(state, condition)?;
        Ok(sat.then_some(constraints))
    }

    pub(super) fn constraints_with_condition(
        &mut self,
        state: &PathState,
        condition: SymBoolExpr,
    ) -> Result<(Vec<SymBoolExpr>, bool), SymbolicError> {
        match condition.as_const() {
            Some(true) => Ok((state.constraints.clone(), true)),
            Some(false) => Ok((state.constraints.clone(), false)),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                let sat = self.is_sat_with_state(state, &constraints)?;
                Ok((constraints, sat))
            }
        }
    }

    pub(super) fn take_loop_jump(
        &self,
        state: &mut PathState,
        code: &SymCode,
        source_pc: usize,
        dest: usize,
    ) -> bool {
        let Some(bound) = self.config.loop_bound else {
            return true;
        };
        if dest >= source_pc {
            return true;
        }
        // Key the back-edge by the return addresses on the stack, so an internal function called
        // from different sites does not count as one loop.
        let return_pcs = state
            .stack
            .iter()
            .filter_map(|word| word.as_const().and_then(|word| usize::try_from(word).ok()))
            .filter(|pc| code.is_valid_jumpdest(*pc).is_ok_and(|valid| valid))
            .collect();
        let count = state.loop_jumps.entry((dest, return_pcs)).or_default();
        if *count >= bound {
            return false;
        }
        *count += 1;
        true
    }

    pub(super) fn handle_log(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        log: SymbolicLog,
    ) -> Result<StepOutcome, SymbolicError> {
        let Some(mut expected) = state.expected_emit.take() else {
            state.record_log(log);
            return Ok(StepOutcome::Continue);
        };

        if let Some(template) = expected.template().cloned() {
            // Like Forge, a log that does not match keeps the expectation pending for a later
            // log, so a feasible mismatch continues on its own path.
            let condition = match expected.match_condition(&mut self.cx, &template, &log) {
                Some(condition) => condition,
                None if expected.is_forbidden() => {
                    return Err(SymbolicError::Unsupported(
                        "symbolic vm.expectEmit count 0 with symbolic log size",
                    ));
                }
                None => SymBoolExpr::constant(&mut self.cx, false),
            };
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            if !match_sat {
                expected.record_mismatch(&log);
                state.expected_emit = Some(expected);
                state.record_log(log);
                return Ok(StepOutcome::Continue);
            }
            let mismatch = condition.not(&mut self.cx);
            if let Some(constraints) = self.constraints_for_condition(state, mismatch)? {
                let mut mismatch_state = state.clone();
                mismatch_state.constraints = constraints;
                let mut pending = expected.clone();
                pending.record_mismatch(&log);
                mismatch_state.expected_emit = Some(pending);
                mismatch_state.record_log(log.clone());
                worklist.push_back(mismatch_state);
            }
            state.constraints = match_constraints;
            if expected.is_forbidden() {
                // Like Forge, a log matching a `count = 0` expectation reverts the emitting frame.
                state.expected_emit = Some(expected);
                return self.revert_expect_emit(state, "log emitted but expected 0 times");
            }
            if !expected.consume_one() {
                state.expected_emit = Some(expected);
            }
        } else {
            // Like Forge, a log without topics cannot fill the template of a non-anonymous
            // expectation and reverts the emitting frame.
            if !expected.is_anonymous() && !log.has_topics() {
                return self.revert_expect_emit(
                    state,
                    "use vm.expectEmitAnonymous to match anonymous events",
                );
            }
            expected.set_template(log.clone());
            state.expected_emit = Some(expected);
        }

        state.record_log(log);
        Ok(StepOutcome::Continue)
    }

    /// Return data of a call that failed the pending `vm.expectEmit`: an ABI-encoded string
    /// without an error selector, like Forge's `UnmetEmit`. When the last mismatching log is
    /// fixed on this path, the string is the message Forge builds from it. A symbolic message
    /// uses a placeholder and defers incomplete, so a caller that inspects it cannot prove a
    /// path, while an uncaught failure still gives a counterexample.
    fn unmet_expected_emit_return_data(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
    ) -> SymReturnData {
        let expected = state.expected_emit.as_ref().expect("an unmet expectation failed the call");
        let message = match (expected.template(), expected.mismatch()) {
            (Some(template), Some(actual)) => {
                self.concrete_mismatch_message(executor, state, expected, template, actual)
            }
            _ => Some("log != expected log".to_string()),
        };
        let message = message.unwrap_or_else(|| {
            self.defer_incomplete("symbolic expectEmit mismatch message");
            "log != expected log".to_string()
        });
        SymReturnData::from_concrete_bytes(&mut self.cx, sol_data::String::abi_encode(&message))
    }

    /// Builds Forge's message for `actual` not matching `template`, when the values it reads are
    /// fixed on this path.
    fn concrete_mismatch_message(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &PathState,
        expected: &ExpectedEmit,
        template: &SymbolicLog,
        actual: &SymbolicLog,
    ) -> Option<String> {
        let checks = expected.checks();
        let anonymous = expected.is_anonymous();
        let template_topics = state.concrete_log_topics(&mut self.cx, template)?;
        let actual_topics = state.concrete_log_topics(&mut self.cx, actual)?;
        let template_data = state.concrete_log_data(&mut self.cx, template)?;
        let actual_data = state.concrete_log_data(&mut self.cx, actual)?;
        let expected_emitter = match expected.emitter() {
            Some(emitter) => Some(word_to_address(state.constrained_word(&mut self.cx, emitter)?)),
            None => None,
        };
        Some(unmatched_emit_message(
            || executor.signatures,
            checks,
            &LogData::new_unchecked(template_topics, template_data),
            expected_emitter,
            anonymous,
            &Log::new_unchecked(actual.emitter(), actual_topics, actual_data),
        ))
    }

    /// Reverts the current frame with a `vm.expectEmit` failure, as Forge does for the emitting
    /// frame.
    fn revert_expect_emit(
        &mut self,
        state: &mut PathState,
        reason: &str,
    ) -> Result<StepOutcome, SymbolicError> {
        state.return_data = error_string_return_data(&mut self.cx, reason);
        self.classify_revert(state)
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn call_concrete_target(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
        to: Address,
        target_word: Option<SymExpr>,
        value: SymExpr,
        gas: SymExpr,
        in_offset: SymExpr,
        in_size: BoundedCopySize,
        out_offset: SymExpr,
        out_size: BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        // Like Forge, only a CALL or STATICCALL reaches a cheatcode. A DELEGATECALL or CALLCODE
        // runs the cheatcode account's one-byte code instead.
        let compatibility_cheatcode = if matches!(kind, CallKind::Call | CallKind::StaticCall)
            && to == SYMBOLIC_VM_COMPAT_ADDRESS
        {
            let (_, max_input_size, _) = in_size.parts(&mut self.cx);
            if max_input_size < 4 {
                false
            } else if let Some(offset) = in_offset.as_const() {
                let offset = usize::try_from(offset).map_err(|_| {
                    SymbolicError::Unsupported("symbolic compatibility CALL input offset")
                })?;
                let selector = state.memory.read_concrete(&mut self.cx, offset, 4)?;
                let selector: [u8; 4] = selector
                    .try_into()
                    .map_err(|_| SymbolicError::Unsupported("symbolic compatibility selector"))?;
                SymbolicVmCheatcode::from_selector(selector).is_some()
            } else {
                // The existing symbolic-offset path checks whether the selector is pinned.
                true
            }
        } else {
            false
        };
        if (matches!(kind, CallKind::Call | CallKind::StaticCall) && to == CHEATCODE_ADDRESS)
            || compatibility_cheatcode
        {
            if !state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero()) {
                return Err(SymbolicError::Unsupported("value-bearing cheatcode CALL"));
            }
            let (in_size_word, in_size, has_symbolic_in_size) = in_size.parts(&mut self.cx);
            let has_symbolic_input_offset = in_offset.as_const().is_none();
            let concrete_in_offset = if has_symbolic_input_offset {
                None
            } else {
                Some(in_offset.as_usize_or("symbolic cheatcode CALL input offset")?)
            };
            // Forge fails a cheatcode call whose calldata its ABI decoder rejects.
            if to == CHEATCODE_ADDRESS
                && !has_symbolic_in_size
                && let Some(concrete_in_offset) = concrete_in_offset
                && let Ok(input) =
                    state.memory.read_concrete(&mut self.cx, concrete_in_offset, in_size)
                && let Err(error) = decode_cheatcode_call(&input)
            {
                let return_data = error_string_return_data(&mut self.cx, &error.to_string());
                revert_cheatcode_call(&mut self.cx, state, out_offset, &out_size, return_data)?;
                return Ok(StepOutcome::Continue);
            }
            if in_size < 4 {
                return Err(SymbolicError::Unsupported("short cheatcode CALL"));
            }
            // After an allocation of symbolic size the input offset is symbolic, but the input
            // is still the `in_size` bytes there: read them with the offset bounds.
            let symbolic_offset_input = has_symbolic_input_offset.then(|| {
                let minimum_offset = state.lower_bound_usize(&in_offset);
                let maximum_offset = state.upper_bound_usize(&mut self.cx, &in_offset);
                state.memory.read_bytes_offset_with_bounds(
                    &mut self.cx,
                    in_offset.clone(),
                    in_size,
                    minimum_offset,
                    maximum_offset,
                )
            });
            let selector = if let Some(input) = &symbolic_offset_input {
                let selector = input.right_aligned_word(&mut self.cx, 0, 4);
                self.constrained_word_with_solver(state, &selector)?
                    .map(|selector| selector.to_be_bytes::<32>()[28..].try_into().unwrap())
                    .ok_or(SymbolicError::Unsupported("symbolic cheatcode selector"))?
            } else {
                state
                    .memory
                    .read_concrete(
                        &mut self.cx,
                        concrete_in_offset.expect("ordinary cheatcode input offset is concrete"),
                        4,
                    )?
                    .try_into()
                    .map_err(|_| SymbolicError::Unsupported("symbolic cheatcode selector"))?
            };
            let full_word_array_assertion =
                to == CHEATCODE_ADDRESS && is_full_word_array_assertion(selector);
            if has_symbolic_in_size {
                let min_size = if to == CHEATCODE_ADDRESS {
                    foundry_cheatcode_min_input_size(selector)
                } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                    symbolic_vm_cheatcode_min_input_size(selector)
                } else {
                    None
                }
                .ok_or(SymbolicError::Unsupported("symbolic cheatcode CALL input size"))?;
                if min_size > in_size {
                    return Err(SymbolicError::Unsupported("symbolic cheatcode CALL input size"));
                }
                if !full_word_array_assertion
                    && state.lower_bound_usize(&in_size_word) < min_size
                    && !self.assume_expr_at_least(state, &in_size_word, min_size)?
                {
                    return Ok(StepOutcome::AssumeRejected);
                }
            }

            if to == CHEATCODE_ADDRESS
                && let Some(concrete_in_offset) = concrete_in_offset
                && let Some(outcome) = self.branch_assume_cheatcode_if_needed(
                    state,
                    worklist,
                    selector,
                    concrete_in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            if to == CHEATCODE_ADDRESS
                && let Some(concrete_in_offset) = concrete_in_offset
                && let Some(outcome) = self.branch_accesses_cheatcode_if_needed(
                    state,
                    worklist,
                    selector,
                    concrete_in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            if to == CHEATCODE_ADDRESS
                && let Some(concrete_in_offset) = concrete_in_offset
                && let Some(outcome) = self.deploy_code_cheatcode_if_needed(
                    executor,
                    state,
                    worklist,
                    completed_paths,
                    selector,
                    concrete_in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            if to == CHEATCODE_ADDRESS
                && let Some(concrete_in_offset) = concrete_in_offset
                && let Some(outcome) = self.bound_cheatcode_if_needed(
                    state,
                    worklist,
                    selector,
                    concrete_in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            if to == CHEATCODE_ADDRESS
                && let Some(concrete_in_offset) = concrete_in_offset
                && let Some(outcome) = self.branch_cheatcode_argument_check_if_needed(
                    state,
                    worklist,
                    selector,
                    concrete_in_offset,
                    in_size,
                    &out_offset,
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            // The handlers decode the input from memory at a concrete offset. The callee cannot
            // see the caller's memory, so an input read at a symbolic offset is decoded from a
            // scratch memory that holds it at offset 0; the array assertions read the offset.
            let caller_memory =
                symbolic_offset_input.filter(|_| !full_word_array_assertion).map(|input| {
                    let mut scratch = SymMemory::default();
                    scratch.store_bytes(&mut self.cx, 0, input);
                    std::mem::replace(&mut state.memory, scratch)
                });
            let (in_offset, concrete_in_offset) = if caller_memory.is_some() {
                (SymExpr::zero(&mut self.cx), Some(0))
            } else {
                (in_offset, concrete_in_offset)
            };
            let outcome = if to == CHEATCODE_ADDRESS {
                self.handle_foundry_cheatcode(
                    executor,
                    state,
                    selector,
                    &in_offset,
                    &in_size_word,
                    in_size,
                )
            } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                self.handle_symbolic_vm_cheatcode(
                    state,
                    selector,
                    concrete_in_offset.expect("symbolic vm input offset is concrete"),
                )
                .map(CheatcodeOutcome::ContinueData)
            } else {
                Err(SymbolicError::Unsupported("symbolic cheatcode address"))
            };
            let outcome = match outcome {
                Ok(CheatcodeOutcome::Fork { mut passing, failing }) => {
                    if let Some(memory) = &caller_memory {
                        passing.memory = memory.clone();
                    }
                    let empty = SymReturnData::empty(&mut self.cx);
                    complete_cheatcode_call(
                        &mut self.cx,
                        &mut passing,
                        out_offset.clone(),
                        &out_size,
                        empty,
                    )?;
                    worklist.push_back(*passing);
                    Ok(*failing)
                }
                outcome => outcome,
            };
            if let Some(memory) = caller_memory {
                state.memory = memory;
            }
            let return_data = match outcome? {
                CheatcodeOutcome::Continue(ret) => SymReturnData::from_words(&mut self.cx, ret),
                CheatcodeOutcome::ContinueData(ret) => ret,
                CheatcodeOutcome::Revert(ret) => {
                    revert_cheatcode_call(&mut self.cx, state, out_offset, &out_size, ret)?;
                    return Ok(StepOutcome::Continue);
                }
                CheatcodeOutcome::Failure => return Ok(StepOutcome::Failure),
                CheatcodeOutcome::Fork { .. } => unreachable!("assertion fork is unwrapped"),
            };

            state.return_data = return_data;
            state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
            state.stack.push(SymExpr::one(&mut self.cx))?;
            return Ok(StepOutcome::Continue);
        }

        if to == HARDHAT_CONSOLE_ADDRESS {
            // Forge's console inspector reverts a call it cannot decode. It only sees calls whose
            // destination is the console address, so DELEGATECALL and CALLCODE run the console's
            // empty code instead.
            let error = if matches!(kind, CallKind::Call | CallKind::StaticCall) {
                in_size
                    .read_from_memory(&mut self.cx, &state.memory, in_offset)
                    .concrete_bytes(&mut self.cx, "symbolic console input")
                    .ok()
                    .and_then(|input| console::hh::ConsoleCalls::abi_decode(&input).err())
            } else {
                None
            };
            state.return_data = match &error {
                Some(error) => {
                    state.first_failed_callee.get_or_insert(HARDHAT_CONSOLE_ADDRESS);
                    let revert = Revert::from(error.to_string()).abi_encode();
                    SymReturnData::from_concrete_bytes(&mut self.cx, revert)
                }
                None => SymReturnData::empty(&mut self.cx),
            };
            state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
            state.stack.push(SymExpr::constant(&mut self.cx, U256::from(error.is_none())))?;
            return Ok(StepOutcome::Continue);
        }

        let call_input = in_size.read_from_memory(&mut self.cx, &state.memory, in_offset.clone());
        // `vm.mockFunction` swaps the code that runs, and the concrete inspector matches
        // `vm.expectCall` against that address, so resolve the redirect before observing.
        let code_address = self.function_mock_target(state, to, &call_input)?.unwrap_or(to);
        if !state.expected_calls.is_empty() {
            let concrete_value = self.transfer_value(state, kind, &value);
            if !self.observe_expected_call(
                state,
                code_address,
                concrete_value,
                &gas,
                &call_input,
            )? {
                return Ok(StepOutcome::Failure);
            }
        }
        let inherited_prank = state.callers();
        let call_context =
            (!matches!(kind, CallKind::DelegateCall)).then(|| state.prank_for_next_call());
        // An isolated CALL is its own transaction, so it bumps the caller's nonce even when the
        // call fails. Concrete Forge answers a mocked call before isolating it.
        let isolated =
            matches!(kind, CallKind::Call) && state.starts_isolated_transaction(executor);
        // Like Forge's `CallInputs::transfer_to`, also the account a codeless call reverts as.
        let transfer_to = if matches!(kind, CallKind::DelegateCall | CallKind::CallCode) {
            state.address
        } else {
            to
        };
        if matches!(kind, CallKind::Call | CallKind::CallCode) {
            let call_caller = call_context.as_ref().expect("value calls have a call context").0;
            if isolated && !self.has_call_mock(state, code_address, &value, &call_input)? {
                state.world.increment_nonce(executor, call_caller)?;
            }
            if let Some(outcome) = self.prepare_value_transfer(
                executor,
                state,
                worklist,
                kind,
                call_caller,
                transfer_to,
                value.clone(),
                out_offset.clone(),
                &out_size,
            )? {
                return Ok(outcome);
            }
        }
        if !state.call_mocks.is_empty() {
            let concrete_value = self.transfer_value(state, kind, &value);
            if let Some(mock) =
                self.take_call_mock(state, code_address, concrete_value, &call_input)?
            {
                let (return_data, reverts) = mock.into_parts();
                if !reverts && let Some((call_caller, _, _)) = call_context {
                    self.apply_call_value_transfer(executor, state, kind, to, call_caller, value);
                }
                return self.finish_codeless_call(
                    executor,
                    state,
                    kind,
                    if reverts { CallStatus::Revert } else { CallStatus::Success },
                    return_data,
                    transfer_to,
                    out_offset,
                    &out_size,
                );
            }
        }
        if matches!(kind, CallKind::DelegateCall) && state.prank.has_active() {
            return Err(SymbolicError::Unsupported("symbolic prank delegatecall"));
        }
        let (call_caller, call_caller_word, pranked_origin) =
            call_context.unwrap_or_else(|| state.prank_for_next_call());

        // A call beyond the call depth limit fails before it transfers value or runs
        // any code, including a precompile.
        if state.at_call_depth_limit() {
            let return_data = SymReturnData::empty(&mut self.cx);
            return self.finish_codeless_call(
                executor,
                state,
                kind,
                CallStatus::Revert,
                return_data,
                transfer_to,
                out_offset,
                &out_size,
            );
        }

        let spec_id = executor.spec_id;
        if is_precompile_for_spec(code_address, spec_id) {
            let input_len = in_size.size_word(&mut self.cx);
            let input = in_size.read_from_memory(&mut self.cx, &state.memory, in_offset);
            if precompile_number_for_spec(code_address, spec_id) == Some(10) {
                let input_bytes = input.materialize(&mut self.cx);
                return self.execute_kzg_precompile_call(
                    executor,
                    state,
                    worklist,
                    kind,
                    to,
                    call_caller,
                    value,
                    out_offset,
                    &out_size,
                    input_bytes,
                    input_len,
                    gas,
                );
            }
            let (gas, stipend_gas) = self.precompile_call_gas(executor, state, kind, &gas, &value);
            let modexp_lengths = if precompile_number_for_spec(code_address, spec_id) == Some(5) {
                self.modexp_length_ranges(state, &input, &input_len)?
            } else {
                None
            };
            // A path can pin a symbolic input to one value, as `if (a == 1)` does before
            // `sha256(abi.encode(a))`: run the precompile on that value so the result is exact
            // and a counterexample replays, instead of an opaque term over the pinned bytes.
            let pinned_input = input_len
                .as_const()
                .and_then(|len| usize::try_from(len).ok())
                .filter(|len| *len <= input.len())
                .and_then(|len| {
                    let bytes = input.materialize(&mut self.cx);
                    let symbolic = bytes[..len].iter().any(|byte| byte.as_const().is_none());
                    constrained_bytes_at(&mut self.cx, state, &bytes, 0, len)
                        .map(|pinned| (pinned, symbolic))
                });
            let run = |cx: &mut SymCx, gas: u64| match &pinned_input {
                Some((pinned, _)) => execute_precompile(cx, code_address, pinned, spec_id, gas),
                None => execute_symbolic_precompile(
                    cx,
                    code_address,
                    input.clone(),
                    input_len.clone(),
                    spec_id,
                    gas,
                    modexp_lengths,
                ),
            };
            let outcome = run(&mut self.cx, gas)?;
            // A precompile that fails without the stipend may still succeed with it.
            if outcome.is_none()
                && let Some(stipend_gas) = stipend_gas
                && run(&mut self.cx, stipend_gas)?.is_some()
            {
                return Err(SymbolicError::Unsupported(
                    "precompile gas with a symbolic call value",
                ));
            }
            // An earlier call over the same symbolic input keeps its opaque term, so tie that term
            // to the result: repeated-call equality must survive the guard that pinned the input.
            // A constant input has no such term.
            if let Some((_, true)) = &pinned_input
                && let Some(concrete) = &outcome
                && let Ok(Some(symbolic)) = execute_symbolic_precompile(
                    &mut self.cx,
                    code_address,
                    input,
                    input_len,
                    spec_id,
                    gas,
                    modexp_lengths,
                )
            {
                let len_eq =
                    SymBoolExpr::eq(&mut self.cx, symbolic.len_word.clone(), concrete.len_word.clone());
                let mut ties = vec![len_eq];
                for idx in 0..concrete.len() {
                    let modeled = symbolic.bytes.byte(&mut self.cx, idx);
                    let actual = concrete.bytes.byte(&mut self.cx, idx);
                    ties.push(SymBoolExpr::eq(&mut self.cx, modeled, actual));
                }
                state.constraints.extend(ties.into_iter().filter(|equal| equal.as_const() != Some(true)));
            }
            return self.apply_precompile_outcome(
                executor,
                state,
                kind,
                to,
                call_caller,
                value,
                out_offset,
                &out_size,
                outcome,
            );
        }

        let child_code = state.world.extcode(&mut self.cx, executor, code_address)?;
        if child_code.is_empty() {
            self.apply_call_value_transfer(executor, state, kind, to, call_caller, value);
            let return_data = SymReturnData::empty(&mut self.cx);
            return self.finish_codeless_call(
                executor,
                state,
                kind,
                CallStatus::Success,
                return_data,
                transfer_to,
                out_offset,
                &out_size,
            );
        }

        let calldata = in_size.calldata(&mut self.cx, call_input);
        let callee_address_word = state
            .world
            .symbolic_word_for_address(to)
            .or_else(|| {
                target_word
                    .as_ref()
                    .filter(|expr| state.world.resolve_address(expr) == Some(to))
                    .cloned()
            })
            .unwrap_or_else(|| SymExpr::constant(&mut self.cx, address_word(to)));
        let frame = match kind {
            CallKind::Call => {
                let mut frame = CallFrame::new(
                    &mut self.cx,
                    to,
                    to,
                    call_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = call_caller_word;
                frame
            }
            CallKind::StaticCall => {
                let value = SymExpr::zero(&mut self.cx);
                let mut frame =
                    CallFrame::new(&mut self.cx, to, to, call_caller, value, true, calldata);
                frame.address_word = callee_address_word;
                frame.caller_word = call_caller_word;
                frame
            }
            CallKind::DelegateCall => {
                let mut frame = CallFrame::new(
                    &mut self.cx,
                    state.address,
                    state.storage_address,
                    state.caller,
                    state.callvalue.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = state.caller_word.clone();
                frame
            }
            CallKind::CallCode => {
                let mut frame = CallFrame::new(
                    &mut self.cx,
                    state.address,
                    state.storage_address,
                    call_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = call_caller_word;
                frame
            }
        };

        let original_world = state.world.clone();
        let mut child = state.child(frame);
        child.prank.inherited = inherited_prank;
        if isolated {
            child.world.clear_transient_storage();
            child.call_depth_base = child.call_depth;
        }
        if let Some((origin, origin_word)) = pranked_origin {
            child.origin = origin;
            child.origin_word = origin_word;
        }
        self.apply_call_value_transfer(executor, &mut child, kind, to, call_caller, value);
        let outcomes = self.execute_external_call(executor, child, &child_code, completed_paths)?;
        if outcomes.is_empty() {
            return Ok(StepOutcome::AssumeRejected);
        }

        let assume_no_revert = state.assume_no_revert_next_call.take();
        let mut parents = VecDeque::with_capacity(outcomes.len());
        for mut outcome in outcomes {
            let failed_callee =
                (!matches!(outcome.status, CallStatus::Success)).then_some(outcome.state.address);
            if matches!(outcome.status, CallStatus::Revert) {
                outcome.state.first_reverted_callee.get_or_insert(outcome.state.address);
            }
            match self.join_call_outcome(
                executor,
                state,
                &original_world,
                outcome,
                failed_callee,
                assume_no_revert.as_ref(),
            )? {
                JoinedCallOutcome::Failure(parent) => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
                JoinedCallOutcome::ExceptionalHalt(mut parent) => {
                    parent.return_data = if parent.check_expected_emit_after_call(kind) {
                        self.unmet_expected_emit_return_data(executor, &mut parent)
                    } else {
                        SymReturnData::empty(&mut self.cx)
                    };
                    parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                    parent.stack.push(SymExpr::zero(&mut self.cx))?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::ExpectedRevert(mut parent) => {
                    parent.return_data = SymReturnData::from_concrete_bytes(
                        &mut self.cx,
                        DUMMY_CALL_OUTPUT.to_vec(),
                    );
                    parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                    parent.stack.push(SymExpr::one(&mut self.cx))?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::Success { mut parent, child, unmet_expected_revert } => {
                    parent.world = child.world;
                    if isolated {
                        parent.world.restore_transient_storage(&original_world);
                    }
                    let unmet_expected_emit = unmet_expected_revert.is_none()
                        && parent.check_expected_emit_after_call(kind);
                    parent.return_data = match unmet_expected_revert {
                        Some(reason) => error_string_return_data(&mut self.cx, reason),
                        None if unmet_expected_emit => {
                            self.unmet_expected_emit_return_data(executor, &mut parent)
                        }
                        None => child.frame.return_data,
                    };
                    parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                    let success = SymExpr::constant(
                        &mut self.cx,
                        U256::from(unmet_expected_revert.is_none() && !unmet_expected_emit),
                    );
                    parent.stack.push(success)?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::Revert { mut parent, child } => {
                    parent.return_data = child.frame.return_data;
                    parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                    parent.stack.push(SymExpr::zero(&mut self.cx))?;
                    parents.push_back(parent);
                }
            }
        }

        Ok(self.resume_parent_paths(state, worklist, parents))
    }

    #[expect(clippy::too_many_arguments)]
    fn execute_kzg_precompile_call(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        kind: CallKind,
        to: Address,
        call_caller: Address,
        value: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
        input: Vec<SymExpr>,
        input_len: SymExpr,
        gas: SymExpr,
    ) -> Result<StepOutcome, SymbolicError> {
        let (gas, stipend_gas) = self.precompile_call_gas(executor, state, kind, &gas, &value);
        if let Some(outcome) =
            kzg_constrained_outcome(&mut self.cx, state, &input, &input_len, gas)?
        {
            // A precompile that fails without the stipend may still succeed with it, including when
            // the stipend leaves the outcome to the symbolic KZG model.
            if outcome.is_none()
                && let Some(stipend_gas) = stipend_gas
                && !matches!(
                    kzg_constrained_outcome(&mut self.cx, state, &input, &input_len, stipend_gas)?,
                    Some(None)
                )
            {
                return Err(SymbolicError::Unsupported(
                    "precompile gas with a symbolic call value",
                ));
            }
            return self.apply_precompile_outcome(
                executor,
                state,
                kind,
                to,
                call_caller,
                value,
                out_offset,
                out_size,
                outcome,
            );
        }

        let success_condition = kzg_success_witness_condition(&mut self.cx, &input, &input_len);
        let failure_condition =
            kzg_failure_witness_condition(&mut self.cx, state, &input, &input_len);
        let modeled_condition = SymBoolExpr::or(
            &mut self.cx,
            vec![success_condition.clone(), failure_condition.clone()],
        );
        let modeled_condition = modeled_condition.not(&mut self.cx);
        let (_, residual_sat) = self.constraints_with_condition(state, modeled_condition)?;
        if residual_sat {
            self.defer_incomplete(KZG_RESIDUAL_REASON);
        }

        let (success_constraints, success_sat) =
            self.constraints_with_condition(state, success_condition)?;

        let (failure_constraints, failure_sat) =
            self.constraints_with_condition(state, failure_condition)?;

        match (success_sat, failure_sat) {
            (true, true) => {
                let mut failure = state.clone();
                failure.constraints = failure_constraints;
                match self.apply_precompile_outcome(
                    executor,
                    &mut failure,
                    kind,
                    to,
                    call_caller,
                    value.clone(),
                    out_offset.clone(),
                    out_size,
                    None,
                )? {
                    StepOutcome::Continue => worklist.push_back(failure),
                    outcome => {
                        *state = failure;
                        return Ok(outcome);
                    }
                }

                state.constraints = success_constraints;
                let return_data = kzg_success_return_data(&mut self.cx);
                self.apply_precompile_outcome(
                    executor,
                    state,
                    kind,
                    to,
                    call_caller,
                    value,
                    out_offset,
                    out_size,
                    Some(return_data),
                )
            }
            (true, false) => {
                state.constraints = success_constraints;
                let return_data = kzg_success_return_data(&mut self.cx);
                self.apply_precompile_outcome(
                    executor,
                    state,
                    kind,
                    to,
                    call_caller,
                    value,
                    out_offset,
                    out_size,
                    Some(return_data),
                )
            }
            (false, true) => {
                state.constraints = failure_constraints;
                self.apply_precompile_outcome(
                    executor,
                    state,
                    kind,
                    to,
                    call_caller,
                    value,
                    out_offset,
                    out_size,
                    None,
                )
            }
            (false, false) => Err(SymbolicError::Unsupported(KZG_RESIDUAL_REASON)),
        }
    }

    /// Returns the gas a call gives a precompile, from its gas operand and value, and the gas with
    /// the value transfer stipend when the value may or may not be zero.
    fn precompile_call_gas(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &PathState,
        kind: CallKind,
        gas: &SymExpr,
        value: &SymExpr,
    ) -> (u64, Option<u64>) {
        let gas = state.constrained_word(&mut self.cx, gas);
        if !matches!(kind, CallKind::Call | CallKind::CallCode) {
            return (precompile_call_gas(executor.gas_limit, gas, false), None);
        }
        match state.constrained_word(&mut self.cx, value) {
            Some(value) => (precompile_call_gas(executor.gas_limit, gas, !value.is_zero()), None),
            None => (
                precompile_call_gas(executor.gas_limit, gas, false),
                Some(precompile_call_gas(executor.gas_limit, gas, true)),
            ),
        }
    }

    /// Returns the ranges of the modexp length headers on this path, or `None` when the input
    /// length is symbolic.
    ///
    /// A header above `MODEXP_LENGTH_SEARCH_CAP` makes any call run out of gas, so the search
    /// stops there.
    fn modexp_length_ranges(
        &mut self,
        state: &PathState,
        input: &SymBytes,
        input_len: &SymExpr,
    ) -> Result<Option<ModexpLengthRanges>, SymbolicError> {
        const MODEXP_LENGTH_SEARCH_CAP: u64 = 1 << 32;

        let Some(input_len) = input_len.as_const().and_then(|len| usize::try_from(len).ok()) else {
            return Ok(None);
        };
        let mut ranges = [(0, 0); 3];
        for (range, offset) in ranges.iter_mut().zip([0, 32, 64]) {
            let (mut min, mut max) =
                precompile_word_byte_range(&mut self.cx, input, input_len, offset);
            if min != max && offset + 32 <= input_len {
                let word = input.word_at(&mut self.cx, offset);
                if let Ok(bound) = self.solver_upper_bound_usize(
                    state,
                    &word,
                    MODEXP_LENGTH_SEARCH_CAP as usize,
                    "symbolic modexp length header",
                ) {
                    max = max.min(bound as u64);
                }
                let below = |cx: &mut SymCx, bound: u64| {
                    let mut constraints = state.constraints.clone();
                    constraints.push(SymBoolExpr::cmp_word_const(
                        cx,
                        SymCmpOp::Ule,
                        &word,
                        U256::from(bound),
                    ));
                    constraints
                };
                let constraints = below(&mut self.cx, MODEXP_LENGTH_SEARCH_CAP);
                let lower = if self.is_sat_with_state(state, &constraints)? {
                    let mut low =
                        (state.lower_bound_usize(&word) as u64).min(MODEXP_LENGTH_SEARCH_CAP);
                    let mut high = MODEXP_LENGTH_SEARCH_CAP;
                    while low < high {
                        let mid = low + (high - low) / 2;
                        let constraints = below(&mut self.cx, mid);
                        if self.is_sat_with_state(state, &constraints)? {
                            high = mid;
                        } else {
                            low = mid + 1;
                        }
                    }
                    low
                } else {
                    MODEXP_LENGTH_SEARCH_CAP + 1
                };
                min = min.max(lower);
            }
            *range = (min, max);
        }
        Ok(Some(ranges))
    }

    #[expect(clippy::too_many_arguments)]
    /// Applies a precompile call result to the current symbolic state.
    fn apply_precompile_outcome(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        kind: CallKind,
        to: Address,
        call_caller: Address,
        value: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
        outcome: Option<SymReturnData>,
    ) -> Result<StepOutcome, SymbolicError> {
        let reverter = if matches!(kind, CallKind::DelegateCall | CallKind::CallCode) {
            state.address
        } else {
            to
        };
        let (status, return_data) = match outcome {
            Some(return_data) => {
                self.apply_call_value_transfer(executor, state, kind, to, call_caller, value);
                (CallStatus::Success, return_data)
            }
            None => (CallStatus::ExceptionalHalt, SymReturnData::empty(&mut self.cx)),
        };
        self.finish_codeless_call(
            executor,
            state,
            kind,
            status,
            return_data,
            reverter,
            out_offset,
            out_size,
        )
    }

    fn apply_call_value_transfer(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        kind: CallKind,
        to: Address,
        from: Address,
        value: SymExpr,
    ) {
        let to = match kind {
            CallKind::Call => to,
            CallKind::CallCode => state.address,
            CallKind::DelegateCall | CallKind::StaticCall => return,
        };
        state.world.transfer(&mut self.cx, executor, from, to, value, true);
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn prepare_value_transfer(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        kind: CallKind,
        from: Address,
        to: Address,
        value: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        if state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero()) {
            return Ok(None);
        }

        // A sender balance below `value` fails the call with an `OutOfFunds` revert.
        let balance = state.balance(&mut self.cx, executor, from);
        let can_transfer = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Uge, balance, value);
        if can_transfer.as_const() == Some(true) {
            return Ok(None);
        }

        let cannot_transfer = can_transfer.clone().not(&mut self.cx);
        let success_constraints = self.constraints_for_condition(state, can_transfer)?;
        let Some(failure_constraints) = self.constraints_for_condition(state, cannot_transfer)?
        else {
            let Some(constraints) = success_constraints else {
                return Ok(Some(StepOutcome::AssumeRejected));
            };
            state.constraints = constraints;
            return Ok(None);
        };
        let mut failure = state.clone();
        failure.constraints = failure_constraints;
        let return_data = SymReturnData::empty(&mut self.cx);
        let outcome = self.finish_codeless_call(
            executor,
            &mut failure,
            kind,
            CallStatus::Revert,
            return_data,
            to,
            out_offset,
            out_size,
        )?;
        match success_constraints {
            Some(constraints) if matches!(outcome, StepOutcome::Continue) => {
                worklist.push_back(failure);
                state.constraints = constraints;
                Ok(None)
            }
            // Without a feasible transfer, or when the failure ends execution, the failure
            // continues on this path.
            _ => {
                *state = failure;
                Ok(Some(outcome))
            }
        }
    }

    pub(super) fn prepare_create_value_transfer(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        from: Address,
        value: SymExpr,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.balance(&mut self.cx, executor, from);
        let can_pay = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Uge, balance, value);
        match can_pay.as_const() {
            Some(true) => Ok(true),
            Some(false) => {
                self.fail_create_before_frame(executor, state)?;
                Ok(false)
            }
            None => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.is_sat_with_state(state, &success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not(&mut self.cx));
                let failure_sat = self.is_sat_with_state(state, &failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        self.fail_create_before_frame(executor, &mut failure)?;
                        worklist.push_back(failure);

                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (true, false) => {
                        state.constraints = success_constraints;
                        Ok(true)
                    }
                    (false, true) => {
                        state.constraints = failure_constraints;
                        self.fail_create_before_frame(executor, state)?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn call_symbolic_target(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
        target: SymExpr,
        value: SymExpr,
        gas: SymExpr,
        in_offset: SymExpr,
        in_size: BoundedCopySize,
        out_offset: SymExpr,
        out_size: BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        let mut candidates = state.world.symbolic_call_targets(&mut self.cx, executor)?;
        candidates.push(SYMBOLIC_VM_COMPAT_ADDRESS);
        if matches!(kind, CallKind::DelegateCall | CallKind::CallCode) {
            candidates.push(CHEATCODE_ADDRESS);
        }
        candidates.extend(active_precompiles(executor.spec_id).as_map().addresses());
        candidates.sort();
        candidates.dedup();

        let candidate_constraints = candidates
            .iter()
            .map(|address| state.world.symbolic_address_condition(&mut self.cx, &target, *address))
            .collect::<Vec<_>>();
        // Only the real cheatcode address uses Forge's decoder. The compatibility address is an
        // ordinary call target unless its selector names a symbolic-VM operation.
        let cheatcode_condition =
            matches!(kind, CallKind::Call | CallKind::StaticCall).then(|| {
                let address = SymExpr::constant(&mut self.cx, address_word(CHEATCODE_ADDRESS));
                SymBoolExpr::eq(&mut self.cx, target.clone(), address)
            });
        let mut outside_constraints = state.constraints.clone();
        outside_constraints.extend(
            candidate_constraints
                .iter()
                .chain(cheatcode_condition.iter())
                .cloned()
                .map(|condition| condition.not(&mut self.cx)),
        );
        let outside_sat = self.is_sat_with_state(state, &outside_constraints)?;

        if !self.config.symbolic_call_targets && outside_sat {
            return Err(SymbolicError::Unsupported("symbolic CALL target"));
        }

        let mut parents = VecDeque::new();
        if outside_sat {
            let mut branch = state.clone();
            branch.constraints = outside_constraints;

            if matches!(kind, CallKind::DelegateCall) && branch.prank.has_active() {
                return Err(SymbolicError::Unsupported("symbolic prank delegatecall"));
            }
            let (call_caller, _, _) = branch.prank_for_next_call();
            if matches!(kind, CallKind::Call) && branch.starts_isolated_transaction(executor) {
                branch.world.increment_nonce(executor, call_caller)?;
            }
            let mut outcome = None;
            if branch.at_call_depth_limit() {
                // Beyond the call depth limit the call fails before it transfers value. A
                // pending revert cheatcode would match against a symbolic reverter.
                if branch.expected_revert.is_some() || branch.assume_no_revert_next_call.is_some() {
                    return Err(SymbolicError::Unsupported(
                        "revert cheatcode on a symbolic CALL target beyond the call depth limit",
                    ));
                }
                branch.return_data = SymReturnData::empty(&mut self.cx);
                branch.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                branch.stack.push(SymExpr::zero(&mut self.cx))?;
                outcome = Some(StepOutcome::Continue);
            } else if matches!(kind, CallKind::Call | CallKind::CallCode) {
                let transfer_to = if matches!(kind, CallKind::Call) {
                    // A failed transfer would report this placeholder, not the symbolic target, as
                    // the reverter.
                    if branch.expected_revert.as_ref().is_some_and(ExpectedRevert::has_reverter) {
                        return Err(SymbolicError::Unsupported(
                            "symbolic vm.expectRevert reverter on symbolic CALL target",
                        ));
                    }
                    branch.world.symbolic_address_slot(target)
                } else {
                    branch.address
                };
                outcome = self.prepare_value_transfer(
                    executor,
                    &mut branch,
                    &mut parents,
                    kind,
                    call_caller,
                    transfer_to,
                    value.clone(),
                    out_offset.clone(),
                    &out_size,
                )?;
                if outcome.is_none() {
                    branch.world.transfer(
                        &mut self.cx,
                        executor,
                        call_caller,
                        transfer_to,
                        value.clone(),
                        true,
                    );
                }
            }
            let outcome = match outcome {
                Some(outcome) => outcome,
                None => {
                    // A successful call has no reverter to match.
                    let return_data = SymReturnData::empty(&mut self.cx);
                    let reverter = branch.address;
                    self.finish_codeless_call(
                        executor,
                        &mut branch,
                        kind,
                        CallStatus::Success,
                        return_data,
                        reverter,
                        out_offset.clone(),
                        &out_size,
                    )?
                }
            };
            match outcome {
                StepOutcome::Continue => parents.push_back(branch),
                StepOutcome::AssumeRejected => {}
                outcome => {
                    *state = branch;
                    return Ok(outcome);
                }
            }
        }

        let call_input = in_size.read_from_memory(&mut self.cx, &state.memory, in_offset.clone());
        if let Some(cheatcode_condition) = cheatcode_condition {
            let mut cheatcode_branch = state.clone();
            cheatcode_branch.constraints.push(cheatcode_condition);
            if self.is_sat_with_state(&cheatcode_branch, &cheatcode_branch.constraints)? {
                // The decoder error only depends on the selector, except for shorter calldata.
                let input = if call_input.len() < 4 {
                    call_input.clone()
                } else {
                    call_input.slice_concrete(&mut self.cx, 0, 4)
                };
                let input = input.concrete_bytes(&mut self.cx, "symbolic cheatcode selector")?;
                if let Ok(selector) = <[u8; 4]>::try_from(input.as_slice())
                    && VmCalls::signature_by_selector(selector).is_some()
                {
                    return Err(SymbolicError::Unsupported("symbolic cheatcode CALL target"));
                }
                let Err(error) = decode_cheatcode_call(&input) else {
                    return Err(SymbolicError::Unsupported("symbolic cheatcode CALL target"));
                };
                // Like Forge, a cheatcode call leaves `vm.expectRevert` and `vm.assumeNoRevert`
                // pending.
                let return_data = error_string_return_data(&mut self.cx, &error.to_string());
                revert_cheatcode_call(
                    &mut self.cx,
                    &mut cheatcode_branch,
                    out_offset.clone(),
                    &out_size,
                    return_data,
                )?;
                parents.push_back(cheatcode_branch);
            }
        }

        for (to, constraint) in candidates.into_iter().zip(candidate_constraints) {
            let mut branch = state.clone();
            branch.constraints.push(constraint);
            if !self.is_sat_with_state(&branch, &branch.constraints)? {
                continue;
            }

            // Decide mock matches before executing either the mocked or real call.
            for mut branch in
                self.split_symbolic_target_candidate(branch, kind, to, &value, &gas, &call_input)?
            {
                let mut branch_worklist = VecDeque::new();
                match self.call_concrete_target(
                    executor,
                    &mut branch,
                    &mut branch_worklist,
                    completed_paths,
                    kind,
                    to,
                    None,
                    value.clone(),
                    gas.clone(),
                    in_offset.clone(),
                    in_size.clone(),
                    out_offset.clone(),
                    out_size.clone(),
                )? {
                    StepOutcome::Continue => {
                        parents.push_back(branch);
                        parents.extend(branch_worklist);
                    }
                    StepOutcome::AssumeRejected => {}
                    outcome => {
                        *state = branch;
                        return Ok(outcome);
                    }
                }
            }
        }

        let Some(first) = self.pop_next_path(&mut parents) else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(StepOutcome::Continue)
    }

    /// Decides mock and expectation matches before executing a symbolic-target candidate.
    fn split_symbolic_target_candidate(
        &mut self,
        branch: PathState,
        kind: CallKind,
        to: Address,
        value: &SymExpr,
        gas: &SymExpr,
        call_input: &SymBytes,
    ) -> Result<Vec<PathState>, SymbolicError> {
        let mut branches = vec![branch];
        for condition in self.function_mock_conditions(&branches[0], to, call_input) {
            branches = self.split_branches_on(branches, condition)?;
        }

        let mut out = Vec::new();
        for mut branch in branches {
            let code_address = if branch.function_mocks.is_empty() {
                to
            } else {
                self.function_mock_target(&mut branch, to, call_input)?.unwrap_or(to)
            };

            let mut value_branches = vec![branch];
            if value_branches[0].constrained_word(&mut self.cx, value).is_none() {
                let candidates =
                    self.call_value_candidates(&value_branches[0], code_address, gas, call_input)?;
                for candidate in candidates {
                    let eq = SymBoolExpr::eq_word_const(&mut self.cx, value, candidate);
                    value_branches = self.split_branches_on(value_branches, eq)?;
                }
            }

            for branch in value_branches {
                let concrete_value = self.transfer_value(&branch, kind, value);
                let mut match_branches = vec![branch];
                let conditions = self.call_match_conditions(
                    &match_branches[0],
                    code_address,
                    concrete_value,
                    gas,
                    call_input,
                )?;
                for condition in conditions {
                    match_branches = self.split_branches_on(match_branches, condition)?;
                }
                if out.len() + match_branches.len() > self.config.path_width() as usize {
                    return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
                }
                out.extend(match_branches);
            }
        }
        Ok(out)
    }

    /// Splits branches on a match condition, retaining feasible outcomes.
    fn split_branches_on(
        &mut self,
        branches: Vec<PathState>,
        condition: SymBoolExpr,
    ) -> Result<Vec<PathState>, SymbolicError> {
        let mismatch_condition = condition.clone().not(&mut self.cx);
        let mut next = Vec::with_capacity(branches.len());
        for branch in branches {
            let (match_constraints, match_sat) =
                self.constraints_with_condition(&branch, condition.clone())?;
            let (mismatch_constraints, mismatch_sat) =
                self.constraints_with_condition(&branch, mismatch_condition.clone())?;
            match (match_sat, mismatch_sat) {
                (true, true) => {
                    let mut match_branch = branch.clone();
                    match_branch.constraints = match_constraints;
                    next.push(match_branch);
                    let mut mismatch_branch = branch;
                    mismatch_branch.constraints = mismatch_constraints;
                    next.push(mismatch_branch);
                }
                (true, false) => {
                    let mut branch = branch;
                    branch.constraints = match_constraints;
                    next.push(branch);
                }
                (false, true) => {
                    let mut branch = branch;
                    branch.constraints = mismatch_constraints;
                    next.push(branch);
                }
                (false, false) => {}
            }
            if next.len() > self.config.path_width() as usize {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }
        }
        Ok(next)
    }

    /// Function mock conditions in match-precedence order.
    fn function_mock_conditions(
        &mut self,
        state: &PathState,
        callee: Address,
        calldata: &SymBytes,
    ) -> Vec<SymBoolExpr> {
        let mut conditions = Vec::new();
        for calldata_len in [calldata.len(), 4] {
            for idx in (0..state.function_mocks.len()).rev() {
                if state.function_mocks[idx].calldata_len() != calldata_len {
                    continue;
                }
                if let Some(condition) =
                    state.function_mocks[idx].match_condition(&mut self.cx, callee, calldata)
                {
                    conditions.push(condition);
                }
            }
        }
        conditions
    }

    /// Values that can satisfy a call expectation or mock.
    fn call_value_candidates(
        &mut self,
        state: &PathState,
        code_address: Address,
        gas: &SymExpr,
        call_input: &SymBytes,
    ) -> Result<Vec<U256>, SymbolicError> {
        let mut candidates = HashSet::<U256>::default();
        for expected in &state.expected_calls {
            let Some(expected_value) = expected.value() else { continue };
            if self
                .expected_call_match_constraints(
                    state,
                    expected,
                    code_address,
                    Some(expected_value),
                    gas,
                    call_input,
                )?
                .is_some()
            {
                candidates.insert(expected_value);
            }
        }
        for mock in &state.call_mocks {
            let Some(mock_value) = mock.value() else { continue };
            if self
                .call_mock_match_constraints(
                    state,
                    mock,
                    code_address,
                    Some(mock_value),
                    call_input,
                )?
                .is_some()
            {
                candidates.insert(mock_value);
            }
        }
        let mut candidates = candidates.into_iter().collect::<Vec<_>>();
        candidates.sort_unstable();
        Ok(candidates)
    }

    /// Expected call and call mock conditions in match-precedence order.
    fn call_match_conditions(
        &mut self,
        state: &PathState,
        code_address: Address,
        value: Option<U256>,
        gas: &SymExpr,
        calldata: &SymBytes,
    ) -> Result<Vec<SymBoolExpr>, SymbolicError> {
        let mut conditions = Vec::new();
        for expected in &state.expected_calls {
            if let Some(condition) =
                expected.match_condition(&mut self.cx, code_address, value, gas, calldata)?
            {
                conditions.push(condition);
            }
        }
        let mut mocks = (0..state.call_mocks.len()).collect::<Vec<_>>();
        mocks.sort_by_key(|idx| {
            let (len, has_value) = state.call_mocks[*idx].specificity();
            (std::cmp::Reverse(len), std::cmp::Reverse(has_value), *idx)
        });
        for idx in mocks {
            if let Some(condition) =
                state.call_mocks[idx].match_condition(&mut self.cx, code_address, value, calldata)
            {
                conditions.push(condition);
            }
        }
        Ok(conditions)
    }
}

const KZG_POINT_EVALUATION_INPUT_LEN: usize = 192;
const KZG_POINT_EVALUATION_GAS: u64 = 50_000;
const KZG_VERSIONED_HASH_OFFSET: usize = 0;
const KZG_Z_OFFSET: usize = 32;
const KZG_Y_OFFSET: usize = 64;
const KZG_COMMITMENT_OFFSET: usize = 96;
const KZG_PROOF_OFFSET: usize = 144;

const KZG_BLS_MODULUS: [u8; 32] =
    hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001");

/// Versioned hash version for KZG commitments.
const KZG_VERSIONED_HASH_VERSION: u8 = 0x01;

/// `U256(FIELD_ELEMENTS_PER_BLOB).to_be_bytes() ++ BLS_MODULUS.to_bytes32()`.
const KZG_RETURN_VALUE: [u8; 64] = hex!(
    "0000000000000000000000000000000000000000000000000000000000001000"
    "73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001"
);

const KZG_SUCCESS_INPUT: [u8; KZG_POINT_EVALUATION_INPUT_LEN] = hex!(
    "01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b"
    "73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000"
    "1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9"
    "8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7"
    "a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
);

const KZG_INVALID_PROOF: [u8; 48] = [0xff; 48];
const KZG_ZERO_COMMITMENT: [u8; 48] = [0x00; 48];
const KZG_ONE_COMMITMENT: [u8; 48] = [0x01; 48];
const KZG_RESIDUAL_REASON: &str = "symbolic KZG point-evaluation precompile residual not modeled";

fn kzg_success_return_data(cx: &mut SymCx) -> SymReturnData {
    SymReturnData::from_concrete_bytes(cx, KZG_RETURN_VALUE.to_vec())
}

fn kzg_constrained_outcome(
    cx: &mut SymCx,
    state: &PathState,
    input: &[SymExpr],
    input_len: &SymExpr,
    gas: u64,
) -> Result<Option<Option<SymReturnData>>, SymbolicError> {
    // The point evaluation precompile checks its fixed cost before its input.
    if gas < KZG_POINT_EVALUATION_GAS {
        return Ok(Some(None));
    }
    let Some(input_len) = state.constrained_usize(cx, input_len) else {
        return Ok(None);
    };
    if input_len != KZG_POINT_EVALUATION_INPUT_LEN {
        return Ok(Some(None));
    }
    if input_len > input.len() {
        return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
    }

    if let Some(input) = constrained_bytes_at(cx, state, input, 0, input_len) {
        return execute_precompile(cx, Address::with_last_byte(10), &input, SpecId::CANCUN, gas)
            .map(Some);
    }

    if constrained_byte(cx, state, &input[0])
        .is_some_and(|version| version != KZG_VERSIONED_HASH_VERSION)
    {
        return Ok(Some(None));
    }

    if constrained_bytes_at(cx, state, input, KZG_Z_OFFSET, KZG_BLS_MODULUS.len())
        .is_some_and(|z| z == KZG_BLS_MODULUS)
        || constrained_bytes_at(cx, state, input, KZG_Y_OFFSET, KZG_BLS_MODULUS.len())
            .is_some_and(|y| y == KZG_BLS_MODULUS)
        || constrained_bytes_at(cx, state, input, KZG_PROOF_OFFSET, KZG_INVALID_PROOF.len())
            .is_some_and(|proof| proof == KZG_INVALID_PROOF)
    {
        return Ok(Some(None));
    }

    if let Some(commitment) = constrained_bytes_at(cx, state, input, KZG_COMMITMENT_OFFSET, 48) {
        let expected_hash = kzg_to_versioned_hash(&commitment);
        for (idx, expected) in expected_hash.into_iter().enumerate() {
            if constrained_byte(cx, state, &input[idx]).is_some_and(|actual| actual != expected) {
                return Ok(Some(None));
            }
        }
    }

    Ok(None)
}

fn kzg_success_witness_condition(
    cx: &mut SymCx,
    input: &[SymExpr],
    input_len: &SymExpr,
) -> SymBoolExpr {
    let len = expr_eq_condition(cx, input_len, KZG_POINT_EVALUATION_INPUT_LEN);
    let bytes = bytes_eq_condition(cx, input, KZG_VERSIONED_HASH_OFFSET, &KZG_SUCCESS_INPUT);
    SymBoolExpr::and(cx, vec![len, bytes])
}

fn kzg_failure_witness_condition(
    cx: &mut SymCx,
    state: &PathState,
    input: &[SymExpr],
    input_len: &SymExpr,
) -> SymBoolExpr {
    let len_192 = expr_eq_condition(cx, input_len, KZG_POINT_EVALUATION_INPUT_LEN);
    let len_ne_192 = expr_ne_condition(cx, input_len, KZG_POINT_EVALUATION_INPUT_LEN);
    let bad_version = byte_ne_condition(cx, input, 0, KZG_VERSIONED_HASH_VERSION);
    let bad_z = bytes_eq_condition(cx, input, KZG_Z_OFFSET, &KZG_BLS_MODULUS);
    let bad_y = bytes_eq_condition(cx, input, KZG_Y_OFFSET, &KZG_BLS_MODULUS);
    let bad_proof = bytes_eq_condition(cx, input, KZG_PROOF_OFFSET, &KZG_INVALID_PROOF);
    let mut conditions = vec![
        len_ne_192,
        SymBoolExpr::and(cx, vec![len_192.clone(), bad_version]),
        SymBoolExpr::and(cx, vec![len_192.clone(), bad_z]),
        SymBoolExpr::and(cx, vec![len_192.clone(), bad_y]),
        SymBoolExpr::and(cx, vec![len_192.clone(), bad_proof]),
    ];

    if let Some(commitment) = constrained_bytes_at(cx, state, input, KZG_COMMITMENT_OFFSET, 48) {
        let expected_hash = kzg_to_versioned_hash(&commitment);
        let mismatch = kzg_versioned_hash_mismatch_condition(cx, input, &expected_hash);
        conditions.push(SymBoolExpr::and(cx, vec![len_192.clone(), mismatch]));
    }

    let expected_hash = &KZG_SUCCESS_INPUT[KZG_VERSIONED_HASH_OFFSET..KZG_Z_OFFSET];
    let commitment = &KZG_SUCCESS_INPUT[KZG_COMMITMENT_OFFSET..KZG_PROOF_OFFSET];
    let commitment_eq = bytes_eq_condition(cx, input, KZG_COMMITMENT_OFFSET, commitment);
    let hash_byte_mismatch = byte_eq_condition(cx, input, 1, expected_hash[1] ^ 1);
    conditions.push(SymBoolExpr::and(cx, vec![len_192.clone(), commitment_eq, hash_byte_mismatch]));

    for commitment in [&KZG_ZERO_COMMITMENT, &KZG_ONE_COMMITMENT] {
        let expected_hash = kzg_to_versioned_hash(commitment);
        let commitment_eq = bytes_eq_condition(cx, input, KZG_COMMITMENT_OFFSET, commitment);
        let mismatch = kzg_versioned_hash_mismatch_condition(cx, input, &expected_hash);
        conditions.push(SymBoolExpr::and(cx, vec![len_192.clone(), commitment_eq, mismatch]));
    }

    SymBoolExpr::or(cx, conditions)
}

fn kzg_versioned_hash_mismatch_condition(
    cx: &mut SymCx,
    input: &[SymExpr],
    expected_hash: &[u8; 32],
) -> SymBoolExpr {
    bytes_ne_condition(cx, input, KZG_VERSIONED_HASH_OFFSET, expected_hash)
}

fn expr_eq_condition(cx: &mut SymCx, expr: &SymExpr, value: usize) -> SymBoolExpr {
    SymBoolExpr::eq_word_const(cx, expr, U256::from(value))
}

fn expr_ne_condition(cx: &mut SymCx, expr: &SymExpr, value: usize) -> SymBoolExpr {
    let condition = expr_eq_condition(cx, expr, value);
    condition.not(cx)
}

fn byte_eq_condition(cx: &mut SymCx, input: &[SymExpr], offset: usize, value: u8) -> SymBoolExpr {
    match input.get(offset) {
        Some(expr) => expr_eq_condition(cx, expr, value as usize),
        None => SymBoolExpr::constant(cx, false),
    }
}

fn byte_ne_condition(cx: &mut SymCx, input: &[SymExpr], offset: usize, value: u8) -> SymBoolExpr {
    match input.get(offset) {
        Some(expr) => expr_ne_condition(cx, expr, value as usize),
        None => SymBoolExpr::constant(cx, false),
    }
}

fn bytes_eq_condition(
    cx: &mut SymCx,
    input: &[SymExpr],
    offset: usize,
    bytes: &[u8],
) -> SymBoolExpr {
    let Some(end) = offset.checked_add(bytes.len()) else {
        return SymBoolExpr::constant(cx, false);
    };
    if end > input.len() {
        return SymBoolExpr::constant(cx, false);
    }
    let conditions = input[offset..end]
        .iter()
        .zip(bytes)
        .map(|(expr, byte)| expr_eq_condition(cx, expr, *byte as usize))
        .collect();
    SymBoolExpr::and(cx, conditions)
}

fn bytes_ne_condition(
    cx: &mut SymCx,
    input: &[SymExpr],
    offset: usize,
    bytes: &[u8],
) -> SymBoolExpr {
    let Some(end) = offset.checked_add(bytes.len()) else {
        return SymBoolExpr::constant(cx, false);
    };
    if end > input.len() {
        return SymBoolExpr::constant(cx, false);
    }
    let conditions = input[offset..end]
        .iter()
        .zip(bytes)
        .map(|(expr, byte)| expr_ne_condition(cx, expr, *byte as usize))
        .collect();
    SymBoolExpr::or(cx, conditions)
}

fn constrained_bytes_at(
    cx: &mut SymCx,
    state: &PathState,
    input: &[SymExpr],
    offset: usize,
    len: usize,
) -> Option<Vec<u8>> {
    let end = offset.checked_add(len)?;
    let bytes = input.get(offset..end)?;
    bytes.iter().map(|byte| constrained_byte(cx, state, byte)).collect()
}

fn constrained_byte(cx: &mut SymCx, state: &PathState, byte: &SymExpr) -> Option<u8> {
    state.constrained_word(cx, byte).and_then(|byte| u8::try_from(byte).ok())
}

fn ensure_expr_not_gasleft(expr: &SymExpr) -> Result<(), SymbolicError> {
    if expr.contains_gasleft() {
        Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"))
    } else {
        Ok(())
    }
}

/// Returns `KZG_VERSIONED_HASH_VERSION ++ sha256(commitment)[1..]`.
fn kzg_to_versioned_hash(commitment: &[u8]) -> [u8; 32] {
    let mut hash = evm2::precompiles::crypto().sha256(commitment);
    hash[0] = KZG_VERSIONED_HASH_VERSION;
    hash
}
