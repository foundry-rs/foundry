use super::*;

impl SymbolicExecutor {
    pub(super) fn call(
        &mut self,
        executor: &Executor<impl FoundryEvmNetwork>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        kind: CallKind,
    ) -> Result<StepOutcome, SymbolicError> {
        let pre_call_state = (!state.function_mocks.is_empty()
            || !state.expected_calls.is_empty()
            || !state.call_mocks.is_empty())
        .then(|| state.clone());
        let call_pc = state.pc.saturating_sub(1);
        let gas = state.stack.pop()?;
        if gas.contains_gasleft() && !gas.is_raw_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let target = state.stack.pop()?;
        ensure_expr_not_gasleft(&target)?;
        let target_address = state.world.resolve_address(&target);
        let value = match (kind, target_address) {
            (CallKind::Call, Some(to)) if is_known_cheatcode(to) => {
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
                let max_size = state
                    .upper_bound_usize(&mut self.cx, &in_size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &in_size,
                            max_limit,
                            "symbolic CALL input size",
                        )
                    })?;
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
                let max_size = state
                    .upper_bound_usize(&mut self.cx, &out_size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &out_size,
                            max_limit,
                            "symbolic CALL output size",
                        )
                    })?;
                BoundedCopySize::Symbolic { size: out_size, max_size }
            }
        };

        if state.is_static
            && !state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero())
        {
            state.return_data = SymReturnData::empty(&mut self.cx);
            return Ok(StepOutcome::Revert);
        }

        let call_input = in_size.read_from_memory(&mut self.cx, &state.memory, in_offset.clone());

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
                    to,
                    code_address,
                    &value,
                    &gas,
                    &call_input,
                )? {
                    return Ok(StepOutcome::Forked);
                }
            }
            let concrete_value = state.constrained_word(&mut self.cx, &value);
            if !state.expected_calls.is_empty() || !state.call_mocks.is_empty() {
                let pre_call_state =
                    pre_call_state.as_ref().expect("call mocks require pre-call state");
                if self.branch_symbolic_call_match_if_needed(
                    state,
                    worklist,
                    pre_call_state,
                    call_pc,
                    to,
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
        to: Address,
        code_address: Address,
        value: &SymExpr,
        gas: &SymExpr,
        call_input: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&mut self.cx, value).is_some() {
            return Ok(false);
        }

        let mut candidates = HashSet::<U256>::default();
        for expected in &state.expected_calls {
            let Some(expected_value) = expected.value() else { continue };
            if self
                .expected_call_match_constraints(
                    state,
                    expected,
                    to,
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
        for candidate in candidates {
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
        for idx in (0..state.function_mocks.len()).rev() {
            if state.function_mocks[idx].calldata_len() != calldata.len() {
                continue;
            }
            let Some(condition) =
                state.function_mocks[idx].match_condition(&mut self.cx, callee, calldata)
            else {
                continue;
            };
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

        for idx in (0..state.function_mocks.len()).rev() {
            if state.function_mocks[idx].calldata_len() != 4 {
                continue;
            }
            let Some(condition) =
                state.function_mocks[idx].match_condition(&mut self.cx, callee, calldata)
            else {
                continue;
            };
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
                return Ok(state.expected_calls[idx].observe());
            }
        }
        Ok(true)
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn branch_symbolic_call_match_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        callee: Address,
        code_address: Address,
        value: Option<U256>,
        gas: &SymExpr,
        calldata: &SymBytes,
    ) -> Result<bool, SymbolicError> {
        for idx in 0..state.expected_calls.len() {
            let Some(condition) = state.expected_calls[idx].match_condition(
                &mut self.cx,
                callee,
                value,
                gas,
                calldata,
            )?
            else {
                continue;
            };
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

        let mut mocks = (0..state.call_mocks.len()).collect::<Vec<_>>();
        mocks.sort_by_key(|idx| {
            let (len, has_value) = state.call_mocks[*idx].specificity();
            (std::cmp::Reverse(len), std::cmp::Reverse(has_value), *idx)
        });

        for idx in mocks {
            let Some(condition) =
                state.call_mocks[idx].match_condition(&mut self.cx, code_address, value, calldata)
            else {
                continue;
            };
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
        if state.call_mocks.is_empty() {
            return Ok(None);
        }
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
        let Some((idx, _, constraints)) = best else {
            return Ok(None);
        };
        state.constraints = constraints;
        Ok(Some(state.call_mocks[idx].next_outcome(&mut self.cx)))
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
        reverter: Address,
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

    pub(super) fn assume_no_revert_rejects(
        &mut self,
        state: &mut PathState,
        assumption: &AssumeNoRevert,
        reverter: Address,
        return_data: &SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let AssumeNoRevert::Filtered(filters) = assumption else {
            return Ok(true);
        };

        let conditions = filters
            .iter()
            .filter_map(|filter| filter.match_condition(&mut self.cx, reverter, return_data))
            .collect::<Vec<_>>();
        if conditions.is_empty() {
            return Ok(false);
        }

        let condition = SymBoolExpr::or(&mut self.cx, conditions);
        let (_match_constraints, match_sat) =
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
                if condition.contains_gasleft() {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                let mut constraints = state.constraints.clone();
                constraints.push(condition);
                let sat = self.solver.is_sat(&mut self.cx, &constraints)?;
                Ok((constraints, sat))
            }
        }
    }

    pub(super) fn take_loop_jump(
        &self,
        state: &mut PathState,
        source_pc: usize,
        dest: usize,
    ) -> bool {
        let Some(bound) = self.config.loop_bound else {
            return true;
        };
        if dest >= source_pc {
            return true;
        }
        let count = state.loop_jumps.entry(dest).or_default();
        if *count >= bound {
            return false;
        }
        *count += 1;
        true
    }

    pub(super) fn handle_log(
        &mut self,
        state: &mut PathState,
        log: SymbolicLog,
    ) -> Result<StepOutcome, SymbolicError> {
        let Some(mut expected) = state.expected_emit.take() else {
            state.record_log(log);
            return Ok(StepOutcome::Continue);
        };

        if let Some(template) = expected.template().cloned() {
            if !self.expected_emit_matches(state, &expected, &template, &log)? {
                state.expected_emit = Some(expected);
                state.record_log(log);
                return Ok(StepOutcome::Failure);
            }
            expected.consume_one();
            if !expected.is_satisfied() {
                state.expected_emit = Some(expected);
            }
        } else {
            expected.set_template(log.clone());
            state.expected_emit = Some(expected);
        }

        state.record_log(log);
        Ok(StepOutcome::Continue)
    }

    /// Returns whether `expected_emit_matches` holds.
    pub(super) fn expected_emit_matches(
        &mut self,
        state: &mut PathState,
        expected: &ExpectedEmit,
        template: &SymbolicLog,
        actual: &SymbolicLog,
    ) -> Result<bool, SymbolicError> {
        let Some(condition) = expected.match_condition(&mut self.cx, template, actual) else {
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

    #[expect(clippy::too_many_arguments)]
    pub(super) fn call_concrete_target<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
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
        if is_known_cheatcode(to) {
            if !state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero()) {
                return Err(SymbolicError::Unsupported("value-bearing cheatcode CALL"));
            }
            let (in_size_word, in_size, has_symbolic_in_size) = in_size.parts(&mut self.cx);
            if in_size < 4 {
                return Err(SymbolicError::Unsupported("short cheatcode CALL"));
            }
            let in_offset = in_offset.as_usize_or("symbolic cheatcode CALL input offset")?;
            if !self.assume_expr_at_least(state, &in_size_word, 4)? {
                return Ok(StepOutcome::AssumeRejected);
            }

            let selector = state
                .memory
                .read_concrete(&mut self.cx, in_offset, 4)?
                .try_into()
                .map_err(|_| SymbolicError::Unsupported("symbolic cheatcode selector"))?;
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
                if !self.assume_expr_at_least(state, &in_size_word, min_size)? {
                    return Ok(StepOutcome::AssumeRejected);
                }
            }

            if to == CHEATCODE_ADDRESS
                && let Some(outcome) = self.branch_accesses_cheatcode_if_needed(
                    state,
                    worklist,
                    selector,
                    in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            if to == CHEATCODE_ADDRESS
                && let Some(outcome) = self.deploy_code_cheatcode_if_needed(
                    executor,
                    state,
                    worklist,
                    completed_paths,
                    selector,
                    in_offset,
                    out_offset.clone(),
                    &out_size,
                )?
            {
                return Ok(outcome);
            }

            let return_data = if to == CHEATCODE_ADDRESS {
                match self
                    .handle_foundry_cheatcode(executor, state, selector, in_offset, in_size)?
                {
                    CheatcodeOutcome::Continue(ret) => SymReturnData::from_words(&mut self.cx, ret),
                    CheatcodeOutcome::ContinueData(ret) => ret,
                    CheatcodeOutcome::AssumeRejected => return Ok(StepOutcome::AssumeRejected),
                    CheatcodeOutcome::Failure => return Ok(StepOutcome::Failure),
                }
            } else if to == SYMBOLIC_VM_COMPAT_ADDRESS {
                self.handle_symbolic_vm_cheatcode(state, selector, in_offset)?
            } else {
                return Err(SymbolicError::Unsupported("symbolic cheatcode address"));
            };

            state.return_data = return_data;
            state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
            state.stack.push(SymExpr::one(&mut self.cx))?;
            return Ok(StepOutcome::Continue);
        }

        if is_console(to) {
            state.return_data = SymReturnData::empty(&mut self.cx);
            state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
            state.stack.push(SymExpr::one(&mut self.cx))?;
            return Ok(StepOutcome::Continue);
        }

        let call_input = in_size.read_from_memory(&mut self.cx, &state.memory, in_offset.clone());
        if !state.expected_calls.is_empty() {
            let concrete_value = state.constrained_word(&mut self.cx, &value);
            if !self.observe_expected_call(state, to, concrete_value, &gas, &call_input)? {
                return Ok(StepOutcome::Failure);
            }
        }
        let code_address = self.function_mock_target(state, to, &call_input)?.unwrap_or(to);
        if !state.call_mocks.is_empty() {
            let concrete_value = state.constrained_word(&mut self.cx, &value);
            if let Some(mock) =
                self.take_call_mock(state, code_address, concrete_value, &call_input)?
            {
                if !matches!(kind, CallKind::DelegateCall) {
                    let _ = state.prank_for_next_call();
                }
                let (return_data, reverts) = mock.into_parts();
                state.return_data = return_data;
                state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
                let success = SymExpr::constant(&mut self.cx, U256::from(!reverts));
                state.stack.push(success)?;
                return Ok(StepOutcome::Continue);
            }
        }

        if matches!(kind, CallKind::Call)
            && !self.prepare_value_transfer(
                executor,
                state,
                worklist,
                value.clone(),
                out_offset.clone(),
                &out_size,
            )?
        {
            return Ok(StepOutcome::Continue);
        }

        let spec_id: SpecId = executor.spec_id().into();
        if is_supported_precompile(code_address, spec_id) {
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
                    value,
                    out_offset,
                    &out_size,
                    input_bytes,
                    input_len,
                );
            }
            match execute_symbolic_precompile(
                &mut self.cx,
                code_address,
                input,
                input_len,
                spec_id,
            )? {
                Some(return_data) => {
                    state.return_data = return_data;
                    if matches!(kind, CallKind::Call) {
                        state.world.transfer(&mut self.cx, executor, state.address, to, value);
                    }
                    state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
                    state.stack.push(SymExpr::one(&mut self.cx))?;
                }
                None => {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
                    state.stack.push(SymExpr::zero(&mut self.cx))?;
                }
            }
            return Ok(StepOutcome::Continue);
        }

        let child_code = state.world.extcode(&mut self.cx, executor, code_address)?;
        if child_code.is_empty() {
            if matches!(kind, CallKind::Call) {
                state.world.transfer(&mut self.cx, executor, state.address, to, value);
            }
            state.return_data = SymReturnData::empty(&mut self.cx);
            state.copy_call_output_offset(&mut self.cx, out_offset, &out_size)?;
            state.stack.push(SymExpr::one(&mut self.cx))?;
            return Ok(StepOutcome::Continue);
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
        if matches!(kind, CallKind::DelegateCall) && state.prank.has_active() {
            return Err(SymbolicError::Unsupported("symbolic prank delegatecall"));
        }
        let (pranked_caller, pranked_caller_word, pranked_origin) = state.prank_for_next_call();
        let frame = match kind {
            CallKind::Call => {
                let mut frame = CallFrame::new(
                    &mut self.cx,
                    to,
                    code_address,
                    to,
                    pranked_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = pranked_caller_word;
                frame
            }
            CallKind::StaticCall => {
                let value = SymExpr::zero(&mut self.cx);
                let mut frame = CallFrame::new(
                    &mut self.cx,
                    to,
                    code_address,
                    to,
                    pranked_caller,
                    value,
                    true,
                    calldata,
                );
                frame.address_word = callee_address_word;
                frame.caller_word = pranked_caller_word;
                frame
            }
            CallKind::DelegateCall => {
                let mut frame = CallFrame::new(
                    &mut self.cx,
                    state.address,
                    code_address,
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
                    code_address,
                    state.storage_address,
                    pranked_caller,
                    value.clone(),
                    state.is_static,
                    calldata,
                );
                frame.address_word = state.address_word.clone();
                frame.caller_word = pranked_caller_word;
                frame
            }
        };

        let original_world = state.world.clone();
        let mut child = state.child(frame);
        if let Some((origin, origin_word)) = pranked_origin {
            child.origin = origin;
            child.origin_word = origin_word;
        }
        if matches!(kind, CallKind::Call) {
            child.world.transfer(&mut self.cx, executor, state.address, to, value);
        }
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;
        let outcomes = self.execute_external_call(executor, child, &child_code, completed_paths)?;
        let Some((first, rest)) = outcomes.split_first() else {
            return Ok(StepOutcome::AssumeRejected);
        };

        let mut parents = VecDeque::with_capacity(outcomes.len());
        for outcome in std::iter::once(first).chain(rest.iter()) {
            let mut parent = state.clone();
            parent.constraints = outcome.state.constraints.clone();
            parent.next_symbol = outcome.state.next_symbol;
            parent.inherit_branch_target_progress(&outcome.state);

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, TopLevelCallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    to,
                    &outcome.return_data,
                )?
            {
                continue;
            }

            if let Some(mut expected) = parent.expected_revert.clone() {
                match outcome.status {
                    TopLevelCallStatus::Success => {
                        *state = parent;
                        return Ok(StepOutcome::Failure);
                    }
                    TopLevelCallStatus::Revert | TopLevelCallStatus::Failure => {
                        if !self.expected_revert_matches(
                            &mut parent,
                            &expected,
                            to,
                            &outcome.return_data,
                        )? {
                            *state = parent;
                            return Ok(StepOutcome::Failure);
                        }
                        if expected.consume_one() {
                            parent.expected_revert = None;
                        } else {
                            parent.expected_revert = Some(expected);
                        }
                        parent.access_record = outcome.state.access_record.clone();
                        parent.expected_calls = outcome.state.expected_calls.clone();
                        parent.expected_creates = outcome.state.expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks.clone();
                        parent.function_mocks = outcome.state.function_mocks.clone();
                        parent.world = original_world.clone();
                        parent.return_data = SymReturnData::empty(&mut self.cx);
                        parent.copy_call_output_offset(
                            &mut self.cx,
                            out_offset.clone(),
                            &out_size,
                        )?;
                        parent.stack.push(SymExpr::one(&mut self.cx))?;
                        parents.push_back(parent);
                        continue;
                    }
                }
            }

            parent.world = if matches!(outcome.status, TopLevelCallStatus::Success) {
                outcome.state.world.clone()
            } else {
                original_world.clone()
            };
            match outcome.status {
                TopLevelCallStatus::Success => {
                    parent.block = outcome.state.block.clone();
                    parent.recorded_logs = outcome.state.recorded_logs.clone();
                    parent.access_record = outcome.state.access_record.clone();
                    parent.expected_emit = outcome.state.expected_emit.clone();
                    parent.expected_calls = outcome.state.expected_calls.clone();
                    parent.expected_creates = outcome.state.expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks.clone();
                    parent.function_mocks = outcome.state.function_mocks.clone();
                }
                TopLevelCallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
                TopLevelCallStatus::Revert => {}
            }
            parent.return_data = outcome.return_data.clone();
            parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
            let success = SymExpr::constant(
                &mut self.cx,
                U256::from(matches!(outcome.status, TopLevelCallStatus::Success)),
            );
            parent.stack.push(success)?;
            parents.push_back(parent);
        }

        let Some(first) = self.pop_next_path(&mut parents) else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(StepOutcome::Continue)
    }

    #[expect(clippy::too_many_arguments)]
    fn execute_kzg_precompile_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        kind: CallKind,
        to: Address,
        value: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
        input: Vec<SymExpr>,
        input_len: SymExpr,
    ) -> Result<StepOutcome, SymbolicError> {
        if let Some(outcome) = kzg_constrained_outcome(&mut self.cx, state, &input, &input_len)? {
            self.apply_precompile_outcome(
                executor, state, kind, to, value, out_offset, out_size, outcome,
            )?;
            return Ok(StepOutcome::Continue);
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
                self.apply_precompile_outcome(
                    executor,
                    &mut failure,
                    kind,
                    to,
                    value.clone(),
                    out_offset.clone(),
                    out_size,
                    None,
                )?;
                worklist.push_back(failure);

                state.constraints = success_constraints;
                let return_data = kzg_success_return_data(&mut self.cx);
                self.apply_precompile_outcome(
                    executor,
                    state,
                    kind,
                    to,
                    value,
                    out_offset,
                    out_size,
                    Some(return_data),
                )?;
                Ok(StepOutcome::Continue)
            }
            (true, false) => {
                state.constraints = success_constraints;
                let return_data = kzg_success_return_data(&mut self.cx);
                self.apply_precompile_outcome(
                    executor,
                    state,
                    kind,
                    to,
                    value,
                    out_offset,
                    out_size,
                    Some(return_data),
                )?;
                Ok(StepOutcome::Continue)
            }
            (false, true) => {
                state.constraints = failure_constraints;
                self.apply_precompile_outcome(
                    executor, state, kind, to, value, out_offset, out_size, None,
                )?;
                Ok(StepOutcome::Continue)
            }
            (false, false) => Err(SymbolicError::Unsupported(KZG_RESIDUAL_REASON)),
        }
    }

    #[expect(clippy::too_many_arguments)]
    /// Applies a precompile call result to the current symbolic state.
    fn apply_precompile_outcome<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        kind: CallKind,
        to: Address,
        value: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
        outcome: Option<SymReturnData>,
    ) -> Result<(), SymbolicError> {
        match outcome {
            Some(return_data) => {
                state.return_data = return_data;
                if matches!(kind, CallKind::Call) {
                    state.world.transfer(&mut self.cx, executor, state.address, to, value);
                }
                state.copy_call_output_offset(&mut self.cx, out_offset, out_size)?;
                state.stack.push(SymExpr::one(&mut self.cx))?;
            }
            None => {
                state.return_data = SymReturnData::empty(&mut self.cx);
                state.copy_call_output_offset(&mut self.cx, out_offset, out_size)?;
                state.stack.push(SymExpr::zero(&mut self.cx))?;
            }
        }
        Ok(())
    }

    pub(super) fn prepare_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        value: SymExpr,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.world.balance_word_for_address(&mut self.cx, executor, state.address);
        let can_pay = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Uge, balance, value);
        match can_pay.as_const() {
            Some(true) => Ok(true),
            Some(false) => {
                state.return_data = SymReturnData::empty(&mut self.cx);
                state.copy_call_output_offset(&mut self.cx, out_offset, out_size)?;
                state.stack.push(SymExpr::zero(&mut self.cx))?;
                Ok(false)
            }
            None => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.solver.is_sat(&mut self.cx, &success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not(&mut self.cx));
                let failure_sat = self.solver.is_sat(&mut self.cx, &failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        failure.return_data = SymReturnData::empty(&mut self.cx);
                        failure.copy_call_output_offset(&mut self.cx, out_offset, out_size)?;
                        failure.stack.push(SymExpr::zero(&mut self.cx))?;
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
                        state.return_data = SymReturnData::empty(&mut self.cx);
                        state.copy_call_output_offset(&mut self.cx, out_offset, out_size)?;
                        state.stack.push(SymExpr::zero(&mut self.cx))?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    pub(super) fn prepare_create_value_transfer<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        value: SymExpr,
    ) -> Result<bool, SymbolicError> {
        if state.constrained_word(&mut self.cx, &value).is_some_and(|value| value.is_zero()) {
            return Ok(true);
        }

        let balance = state.world.balance_word_for_address(&mut self.cx, executor, state.address);
        let can_pay = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Uge, balance, value);
        match can_pay.as_const() {
            Some(true) => Ok(true),
            Some(false) => {
                state.return_data = SymReturnData::empty(&mut self.cx);
                state.stack.push(SymExpr::zero(&mut self.cx))?;
                Ok(false)
            }
            None => {
                let mut success_constraints = state.constraints.clone();
                success_constraints.push(can_pay.clone());
                let success_sat = self.solver.is_sat(&mut self.cx, &success_constraints)?;

                let mut failure_constraints = state.constraints.clone();
                failure_constraints.push(can_pay.not(&mut self.cx));
                let failure_sat = self.solver.is_sat(&mut self.cx, &failure_constraints)?;

                match (success_sat, failure_sat) {
                    (true, true) => {
                        let mut failure = state.clone();
                        failure.constraints = failure_constraints;
                        failure.return_data = SymReturnData::empty(&mut self.cx);
                        failure.stack.push(SymExpr::zero(&mut self.cx))?;
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
                        state.return_data = SymReturnData::empty(&mut self.cx);
                        state.stack.push(SymExpr::zero(&mut self.cx))?;
                        Ok(false)
                    }
                    (false, false) => Ok(false),
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn call_symbolic_target<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
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
        candidates.extend((1..=10).map(precompile_address));
        candidates.sort();
        candidates.dedup();
        if candidates.is_empty() {
            return Err(SymbolicError::Unsupported(
                "symbolic CALL target has no known contract candidates",
            ));
        }

        let candidate_constraints = candidates
            .iter()
            .map(|address| {
                let address = SymExpr::constant(&mut self.cx, address_word(*address));
                SymBoolExpr::eq(&mut self.cx, target.clone(), address)
            })
            .collect::<Vec<_>>();
        let mut outside_constraints = state.constraints.clone();
        outside_constraints.extend(
            candidate_constraints.iter().cloned().map(|condition| condition.not(&mut self.cx)),
        );
        let outside_sat = self.solver.is_sat(&mut self.cx, &outside_constraints)?;

        if !self.config.symbolic_call_targets && outside_sat {
            return Err(SymbolicError::Unsupported("symbolic CALL target"));
        }

        let mut parents = VecDeque::new();
        if outside_sat {
            let mut branch = state.clone();
            branch.constraints = outside_constraints;

            if matches!(kind, CallKind::Call) {
                if self.prepare_value_transfer(
                    executor,
                    &mut branch,
                    &mut parents,
                    value.clone(),
                    out_offset.clone(),
                    &out_size,
                )? {
                    let symbolic_target = target;
                    let to = branch.world.symbolic_address_slot(symbolic_target);
                    branch.world.transfer(
                        &mut self.cx,
                        executor,
                        branch.address,
                        to,
                        value.clone(),
                    );
                    branch.return_data = SymReturnData::empty(&mut self.cx);
                    branch.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                    branch.stack.push(SymExpr::one(&mut self.cx))?;
                    parents.push_back(branch);
                }
            } else {
                branch.return_data = SymReturnData::empty(&mut self.cx);
                branch.copy_call_output_offset(&mut self.cx, out_offset.clone(), &out_size)?;
                branch.stack.push(SymExpr::one(&mut self.cx))?;
                parents.push_back(branch);
            }
        }

        for (to, constraint) in candidates.into_iter().zip(candidate_constraints) {
            let mut branch = state.clone();
            branch.constraints.push(constraint);
            if !self.solver.is_sat(&mut self.cx, &branch.constraints)? {
                continue;
            }

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
                outcome => return Ok(outcome),
            }
        }

        let Some(first) = self.pop_next_path(&mut parents) else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(StepOutcome::Continue)
    }
}

const KZG_POINT_EVALUATION_INPUT_LEN: usize = 192;
const KZG_VERSIONED_HASH_OFFSET: usize = 0;
const KZG_Z_OFFSET: usize = 32;
const KZG_Y_OFFSET: usize = 64;
const KZG_COMMITMENT_OFFSET: usize = 96;
const KZG_PROOF_OFFSET: usize = 144;

const KZG_BLS_MODULUS: [u8; 32] =
    hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001");

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
    SymReturnData::from_concrete_bytes(cx, kzg_point_evaluation::RETURN_VALUE.to_vec())
}

fn kzg_constrained_outcome(
    cx: &mut SymCx,
    state: &PathState,
    input: &[SymExpr],
    input_len: &SymExpr,
) -> Result<Option<Option<SymReturnData>>, SymbolicError> {
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
        return execute_precompile(cx, precompile_address(10), &input, SpecId::CANCUN).map(Some);
    }

    if constrained_byte(cx, state, &input[0])
        .is_some_and(|version| version != kzg_point_evaluation::VERSIONED_HASH_VERSION_KZG)
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
        let expected_hash = kzg_point_evaluation::kzg_to_versioned_hash(&commitment);
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
    let bad_version =
        byte_ne_condition(cx, input, 0, kzg_point_evaluation::VERSIONED_HASH_VERSION_KZG);
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
        let expected_hash = kzg_point_evaluation::kzg_to_versioned_hash(&commitment);
        let mismatch = kzg_versioned_hash_mismatch_condition(cx, input, &expected_hash);
        conditions.push(SymBoolExpr::and(cx, vec![len_192.clone(), mismatch]));
    }

    let expected_hash = &KZG_SUCCESS_INPUT[KZG_VERSIONED_HASH_OFFSET..KZG_Z_OFFSET];
    let commitment = &KZG_SUCCESS_INPUT[KZG_COMMITMENT_OFFSET..KZG_PROOF_OFFSET];
    let commitment_eq = bytes_eq_condition(cx, input, KZG_COMMITMENT_OFFSET, commitment);
    let hash_byte_mismatch = byte_eq_condition(cx, input, 1, expected_hash[1] ^ 1);
    conditions.push(SymBoolExpr::and(cx, vec![len_192.clone(), commitment_eq, hash_byte_mismatch]));

    for commitment in [&KZG_ZERO_COMMITMENT, &KZG_ONE_COMMITMENT] {
        let expected_hash = kzg_point_evaluation::kzg_to_versioned_hash(commitment);
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
