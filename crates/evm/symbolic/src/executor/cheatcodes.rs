use foundry_cheatcodes_spec::Vm::*;

use super::*;

impl SymbolicExecutor {
    pub(super) fn handle_assertion(
        &mut self,
        state: &mut PathState,
        pass: SymBoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let fail = pass.clone().not();
        match fail.as_const() {
            Some(true) => return Ok(CheatcodeOutcome::Failure),
            Some(false) => return Ok(CheatcodeOutcome::Continue(Vec::new())),
            None => {}
        }

        if pass.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }

        let mut fail_constraints = state.constraints.clone();
        fail_constraints.push(fail);
        if self.solver.is_sat(&fail_constraints)? {
            state.constraints = fail_constraints;
            return Ok(CheatcodeOutcome::Failure);
        }

        state.constraints.push(pass);
        Ok(CheatcodeOutcome::Continue(Vec::new()))
    }

    pub(super) fn set_expected_revert(
        &mut self,
        state: &mut PathState,
        data: ExpectedRevertData,
        reverter: Option<SymExpr>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_revert = Some(ExpectedRevert::new(data, reverter, remaining));
        CheatcodeOutcome::Continue(Vec::new())
    }

    pub(super) fn set_expected_emit(
        &mut self,
        state: &mut PathState,
        checks: ExpectedEmitChecks,
        emitter: Option<SymExpr>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_emit = Some(ExpectedEmit::new(checks, emitter, remaining));
        CheatcodeOutcome::Continue(Vec::new())
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn set_expected_call(
        &mut self,
        state: &mut PathState,
        callee: SymExpr,
        value: Option<U256>,
        gas: Option<u64>,
        min_gas: Option<u64>,
        data: SymBytes,
        count: Option<u64>,
    ) -> CheatcodeOutcome {
        state.expected_calls.push(ExpectedCall::new(callee, value, gas, min_gas, data, count));
        CheatcodeOutcome::Continue(Vec::new())
    }

    pub(super) fn set_expected_create(
        &mut self,
        state: &mut PathState,
        bytecode: Vec<u8>,
        deployer: SymExpr,
        kind: CreateKind,
    ) -> CheatcodeOutcome {
        state.expected_creates.push(ExpectedCreate::new(bytecode, deployer, kind));
        CheatcodeOutcome::Continue(Vec::new())
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn deploy_code_cheatcode_if_needed<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let args_offset = in_offset + 4;
        let (artifact, constructor_args) = if selector == deployCode_0Call::SELECTOR {
            let artifact =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deployCode")?;
            (artifact, Vec::new())
        } else if selector == deployCode_1Call::SELECTOR {
            let artifact =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deployCode")?;
            let args = read_abi_dynamic_bytes_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.deployCode args",
            )?;
            (artifact, args)
        } else {
            return Ok(None);
        };

        self.deploy_code_cheatcode_call(
            executor,
            state,
            worklist,
            completed_paths,
            artifact,
            constructor_args,
            out_offset,
            out_size,
        )
        .map(Some)
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn deploy_code_cheatcode_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        artifact: String,
        constructor_args: Vec<u8>,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        if state.is_static {
            state.return_data = SymReturnData::default();
            return Ok(StepOutcome::Revert);
        }

        let mut initcode = artifact_code(&artifact, false)?;
        initcode.extend_from_slice(&constructor_args);
        let initcode = SymCode::concrete(initcode);

        let nonce = state.world.nonce(executor, state.address)?;
        let created = state.address.create(nonce);
        let created_word = SymExpr::constant(address_word(created));

        let mut failure_world = state.world.clone();
        failure_world.increment_nonce(executor, state.address)?;
        if failure_world.has_code_or_nonce(executor, created)? {
            state.world = failure_world;
            complete_cheatcode_call(
                state,
                out_offset,
                out_size,
                SymReturnData::from_words(vec![SymExpr::zero()]),
            )?;
            return Ok(StepOutcome::Continue);
        }

        let mut frame = CallFrame::new(
            created,
            created,
            created,
            state.address,
            SymExpr::zero(),
            false,
            SymCalldata::from_bytes(SymBytes::default()),
        );
        frame.address_word = created_word.clone();
        frame.caller_word = state.address_word.clone();
        let mut child = state.child(frame);
        let pending_expected_creates = std::mem::take(&mut child.expected_creates);
        child.world = failure_world.clone();
        child.world.mark_current_transaction_created(created);
        child.world.set_nonce(created, 1);
        child.expected_revert = None;
        child.assume_no_revert_next_call = None;

        let outcomes = self.execute_external_call(executor, child, &initcode, completed_paths)?;
        let Some((first, rest)) = outcomes.split_first() else {
            return Ok(StepOutcome::AssumeRejected);
        };

        let mut parents = VecDeque::with_capacity(outcomes.len());
        for outcome in std::iter::once(first).chain(rest.iter()) {
            let mut parent = state.clone();
            parent.constraints = outcome.state.constraints.clone();
            parent.next_symbol = outcome.state.next_symbol;

            if let Some(assumption) = parent.assume_no_revert_next_call.take()
                && matches!(outcome.status, TopLevelCallStatus::Revert)
                && self.assume_no_revert_rejects(
                    &mut parent,
                    &assumption,
                    created,
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
                            created,
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
                        parent.expected_creates = pending_expected_creates.clone();
                        parent.call_mocks = outcome.state.call_mocks.clone();
                        parent.function_mocks = outcome.state.function_mocks.clone();
                        parent.world = failure_world.clone();
                        complete_cheatcode_call(
                            &mut parent,
                            out_offset.clone(),
                            out_size,
                            SymReturnData::from_words(vec![SymExpr::zero()]),
                        )?;
                        parents.push_back(parent);
                        continue;
                    }
                }
            }

            match outcome.status {
                TopLevelCallStatus::Success => {
                    parent.world = outcome.state.world.clone();
                    parent.block = outcome.state.block.clone();
                    parent.recorded_logs = outcome.state.recorded_logs.clone();
                    parent.access_record = outcome.state.access_record.clone();
                    parent.expected_emit = outcome.state.expected_emit.clone();
                    parent.expected_calls = outcome.state.expected_calls.clone();
                    parent.expected_creates = pending_expected_creates.clone();
                    parent.call_mocks = outcome.state.call_mocks.clone();
                    parent.function_mocks = outcome.state.function_mocks.clone();
                    self.observe_expected_create(
                        &mut parent,
                        state.address,
                        CreateKind::Create,
                        &outcome.return_data,
                    )?;
                    if !parent.world.is_destroyed(created) {
                        parent.world.install_code(created, outcome.return_data.to_code()?);
                        parent.world.set_nonce(created, 1);
                    }
                    complete_cheatcode_call(
                        &mut parent,
                        out_offset.clone(),
                        out_size,
                        SymReturnData::from_words(vec![created_word.clone()]),
                    )?;
                }
                TopLevelCallStatus::Revert => {
                    parent.world = failure_world.clone();
                    parent.return_data = outcome.return_data.clone();
                    parent.copy_call_output_offset(out_offset.clone(), out_size)?;
                    parent.stack.push(SymExpr::zero())?;
                }
                TopLevelCallStatus::Failure => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
            }

            parents.push_back(parent);
        }

        let Some(first) = pop_batch(&mut parents, self.config.exploration_order) else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        spill_batch(parents, worklist, self.config.exploration_order);
        Ok(StepOutcome::Continue)
    }

    pub(super) fn observe_expected_create(
        &mut self,
        state: &mut PathState,
        deployer: Address,
        kind: CreateKind,
        runtime: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if state.expected_creates.is_empty() {
            return Ok(());
        }
        let bytecode = runtime.read_concrete("symbolic expected create bytecode")?;
        let mut mismatch_constraints = None;
        for idx in 0..state.expected_creates.len() {
            let Some(condition) =
                state.expected_creates[idx].match_condition(deployer, kind, &bytecode)
            else {
                continue;
            };
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let (candidate_mismatch_constraints, mismatch_sat) =
                self.constraints_with_condition(state, condition.not())?;

            if match_sat && !mismatch_sat {
                state.constraints = match_constraints;
                state.expected_creates.swap_remove(idx);
                return Ok(());
            }

            if mismatch_sat {
                mismatch_constraints.get_or_insert(candidate_mismatch_constraints);
            }
        }

        if let Some(constraints) = mismatch_constraints {
            state.constraints = constraints;
        }
        Ok(())
    }

    pub(super) fn branch_accesses_cheatcode_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        if selector != accessesCall::SELECTOR {
            return Ok(None);
        }

        let Some(record) = state.access_record.clone() else {
            return Ok(None);
        };
        let target = read_abi_word_arg(&state.memory, in_offset + 4, 0)?;
        if target.as_const().is_some() {
            return Ok(None);
        }

        let addresses = record.addresses();
        if addresses.is_empty() {
            return Ok(None);
        }

        let mut branches = VecDeque::new();
        let mut matched_conditions = Vec::new();
        for address in addresses {
            let condition = target.address_match_condition(address);
            matched_conditions.push(condition.clone());
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                let mut branch = state.clone();
                branch.constraints = constraints;
                complete_cheatcode_call(
                    &mut branch,
                    out_offset.clone(),
                    out_size,
                    accesses_return_data(Some(&record), address),
                )?;
                branches.push_back(branch);
            }
        }

        let unmatched_condition =
            SymBoolExpr::and(matched_conditions.into_iter().map(SymBoolExpr::not).collect());
        if let Some(constraints) = self.constraints_for_condition(state, unmatched_condition)? {
            let mut branch = state.clone();
            branch.constraints = constraints;
            complete_cheatcode_call(
                &mut branch,
                out_offset,
                out_size,
                accesses_return_data(Some(&record), Address::ZERO),
            )?;
            branches.push_back(branch);
        }

        let Some(first_branch) = pop_batch(&mut branches, self.config.exploration_order) else {
            return Ok(Some(StepOutcome::AssumeRejected));
        };
        *state = first_branch;
        spill_batch(branches, worklist, self.config.exploration_order);
        Ok(Some(StepOutcome::Continue))
    }

    pub(super) fn accesses_return_data_for_target(
        &mut self,
        state: &mut PathState,
        target: SymExpr,
    ) -> Result<SymReturnData, SymbolicError> {
        let Some(record) = state.access_record.clone() else {
            return Ok(accesses_return_data(None, Address::ZERO));
        };

        if let Some(target) = target.as_const() {
            return Ok(accesses_return_data(Some(&record), word_to_address(target)));
        }

        let addresses = record.addresses();
        if addresses.is_empty() {
            return Ok(accesses_return_data(Some(&record), Address::ZERO));
        }

        for address in addresses {
            let condition = target.address_match_condition(address);
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let (_, mismatch_sat) = self.constraints_with_condition(state, condition.not())?;

            match (match_sat, mismatch_sat) {
                (true, false) => {
                    state.constraints = match_constraints;
                    return Ok(accesses_return_data(Some(&record), address));
                }
                (true, true) => {
                    return Err(SymbolicError::Unsupported("symbolic vm.accesses address"));
                }
                (false, _) => {}
            }
        }

        Ok(accesses_return_data(Some(&record), Address::ZERO))
    }

    pub(super) fn add_call_mock(
        &mut self,
        state: &mut PathState,
        callee: SymExpr,
        value: Option<U256>,
        data: SymBytes,
        returns: Vec<SymReturnData>,
        reverts: bool,
    ) -> CheatcodeOutcome {
        state.call_mocks.push(CallMock::new(callee, value, data, returns, reverts));
        CheatcodeOutcome::Continue(Vec::new())
    }

    pub(super) fn set_function_mock(
        &mut self,
        state: &mut PathState,
        callee: SymExpr,
        target: Address,
        data: SymBytes,
    ) -> CheatcodeOutcome {
        if let Some(mock) =
            state.function_mocks.iter_mut().find(|mock| mock.matches_definition(&callee, &data))
        {
            mock.set_target(target);
        } else {
            state.function_mocks.push(FunctionMock::new(callee, target, data));
        }
        CheatcodeOutcome::Continue(Vec::new())
    }

    pub(super) fn handle_foundry_cheatcode<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
        in_size: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let args_offset = in_offset + 4;
        match selector {
            assumeCall::SELECTOR => {
                return self.handle_assume(state, in_offset + 4);
            }
            assumeNoRevert_0Call::SELECTOR => {
                if state.assume_no_revert_next_call.is_some() {
                    return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
                }
                state.assume_no_revert_next_call = Some(AssumeNoRevert::Any);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            assumeNoRevert_1Call::SELECTOR => {
                if state.assume_no_revert_next_call.is_some() {
                    return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
                }
                let mut values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Tuple(vec![
                        DynSolType::Address,
                        DynSolType::Bool,
                        DynSolType::Bytes,
                    ])],
                )?;
                let value = values
                    .pop()
                    .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
                state.assume_no_revert_next_call =
                    Some(AssumeNoRevert::Filtered(vec![dyn_potential_revert(&value)?]));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            assumeNoRevert_2Call::SELECTOR => {
                if state.assume_no_revert_next_call.is_some() {
                    return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
                }
                let mut values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                        DynSolType::Address,
                        DynSolType::Bool,
                        DynSolType::Bytes,
                    ])))],
                )?;
                let value = values
                    .pop()
                    .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
                state.assume_no_revert_next_call =
                    Some(AssumeNoRevert::Filtered(dyn_potential_reverts(&value)?));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            skip_0Call::SELECTOR | skip_1Call::SELECTOR => {
                return self.handle_skip(state, in_offset + 4);
            }
            recordLogsCall::SELECTOR => {
                state.recorded_logs = Some(Vec::new());
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            recordCall::SELECTOR => {
                state.access_record = Some(AccessRecord::default());
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            stopRecordCall::SELECTOR => {
                state.access_record = None;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            accessesCall::SELECTOR => {
                let target = read_abi_word_arg(&state.memory, args_offset, 0)?;
                return Ok(CheatcodeOutcome::ContinueData(
                    self.accesses_return_data_for_target(state, target)?,
                ));
            }
            getRecordedLogsCall::SELECTOR => {
                let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
                return Ok(CheatcodeOutcome::ContinueData(recorded_logs_return_data(logs)));
            }
            getRecordedLogsJsonCall::SELECTOR => {
                let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
                return Ok(CheatcodeOutcome::ContinueData(recorded_logs_json_return_data(logs)?));
            }
            expectRevert_0Call::SELECTOR => {
                return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, 1));
            }
            expectRevert_1Call::SELECTOR => {
                let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::prefix(SymBytes::exprs(selector)),
                    None,
                    1,
                ));
            }
            expectRevert_2Call::SELECTOR => {
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    0,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectRevert",
                )?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::exact(SymBytes::exprs(data)),
                    None,
                    1,
                ));
            }
            expectRevert_3Call::SELECTOR => {
                let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::Any,
                    Some(reverter),
                    1,
                ));
            }
            expectRevert_4Call::SELECTOR => {
                let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
                let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::prefix(SymBytes::exprs(selector)),
                    Some(reverter),
                    1,
                ));
            }
            expectRevert_5Call::SELECTOR => {
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    0,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectRevert",
                )?;
                let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::exact(SymBytes::exprs(data)),
                    Some(reverter),
                    1,
                ));
            }
            expectRevert_6Call::SELECTOR => {
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectRevert")?;
                return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, count));
            }
            expectRevert_7Call::SELECTOR => {
                let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::prefix(SymBytes::exprs(selector)),
                    None,
                    count,
                ));
            }
            expectRevert_8Call::SELECTOR => {
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    0,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectRevert",
                )?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::exact(SymBytes::exprs(data)),
                    None,
                    count,
                ));
            }
            expectRevert_9Call::SELECTOR => {
                let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::Any,
                    Some(reverter),
                    count,
                ));
            }
            expectRevert_10Call::SELECTOR => {
                let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
                let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::prefix(SymBytes::exprs(selector)),
                    Some(reverter),
                    count,
                ));
            }
            expectRevert_11Call::SELECTOR => {
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    0,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectRevert",
                )?;
                let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::exact(SymBytes::exprs(data)),
                    Some(reverter),
                    count,
                ));
            }
            expectPartialRevert_0Call::SELECTOR => {
                let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::prefix(SymBytes::exprs(selector)),
                    None,
                    1,
                ));
            }
            expectPartialRevert_1Call::SELECTOR => {
                let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
                let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return Ok(self.set_expected_revert(
                    state,
                    ExpectedRevertData::prefix(SymBytes::exprs(selector)),
                    Some(reverter),
                    1,
                ));
            }
            expectEmit_2Call::SELECTOR => {
                return Ok(self.set_expected_emit(
                    state,
                    ExpectedEmitChecks::default_non_anonymous(),
                    None,
                    1,
                ));
            }
            expectEmit_3Call::SELECTOR => {
                let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
                return Ok(self.set_expected_emit(
                    state,
                    ExpectedEmitChecks::default_non_anonymous(),
                    Some(emitter),
                    1,
                ));
            }
            expectEmit_6Call::SELECTOR => {
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectEmit")?;
                return Ok(self.set_expected_emit(
                    state,
                    ExpectedEmitChecks::default_non_anonymous(),
                    None,
                    count,
                ));
            }
            expectEmit_7Call::SELECTOR => {
                let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectEmit")?;
                return Ok(self.set_expected_emit(
                    state,
                    ExpectedEmitChecks::default_non_anonymous(),
                    Some(emitter),
                    count,
                ));
            }
            expectEmit_0Call::SELECTOR => {
                let checks =
                    ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
                return Ok(self.set_expected_emit(state, checks, None, 1));
            }
            expectEmit_1Call::SELECTOR => {
                let checks =
                    ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
                let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
                return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
            }
            expectEmit_4Call::SELECTOR => {
                let checks =
                    ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectEmit")?;
                return Ok(self.set_expected_emit(state, checks, None, count));
            }
            expectEmit_5Call::SELECTOR => {
                let checks =
                    ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
                let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 5, "symbolic vm.expectEmit")?;
                return Ok(self.set_expected_emit(state, checks, Some(emitter), count));
            }
            expectEmitAnonymous_2Call::SELECTOR => {
                return Ok(self.set_expected_emit(
                    state,
                    ExpectedEmitChecks::default_anonymous(),
                    None,
                    1,
                ));
            }
            expectEmitAnonymous_3Call::SELECTOR => {
                let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
                return Ok(self.set_expected_emit(
                    state,
                    ExpectedEmitChecks::default_anonymous(),
                    Some(emitter),
                    1,
                ));
            }
            expectEmitAnonymous_0Call::SELECTOR => {
                let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
                return Ok(self.set_expected_emit(state, checks, None, 1));
            }
            expectEmitAnonymous_1Call::SELECTOR => {
                let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
                let emitter = read_abi_word_arg(&state.memory, args_offset, 5)?;
                return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
            }
            expectCall_0Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    1,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    None,
                    None,
                    None,
                    SymBytes::exprs(data),
                    None,
                ));
            }
            expectCall_1Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    1,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    None,
                    None,
                    None,
                    SymBytes::exprs(data),
                    Some(count),
                ));
            }
            expectCall_2Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.expectCall",
                )?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    Some(value),
                    None,
                    None,
                    SymBytes::exprs(data),
                    None,
                ));
            }
            expectCall_3Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.expectCall",
                )?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 3, "symbolic vm.expectCall")?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    Some(value),
                    None,
                    None,
                    SymBytes::exprs(data),
                    Some(count),
                ));
            }
            expectCall_4Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.expectCall",
                )?;
                let gas =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    Some(value),
                    Some(gas),
                    None,
                    SymBytes::exprs(data),
                    None,
                ));
            }
            expectCall_5Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.expectCall",
                )?;
                let gas =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    Some(value),
                    Some(gas),
                    None,
                    SymBytes::exprs(data),
                    Some(count),
                ));
            }
            expectCallMinGas_0Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.expectCall",
                )?;
                let min_gas =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    Some(value),
                    None,
                    Some(min_gas),
                    SymBytes::exprs(data),
                    None,
                ));
            }
            expectCallMinGas_1Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.expectCall",
                )?;
                let min_gas =
                    read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.expectCall",
                )?;
                let count =
                    read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
                return Ok(self.set_expected_call(
                    state,
                    callee,
                    Some(value),
                    None,
                    Some(min_gas),
                    SymBytes::exprs(data),
                    Some(count),
                ));
            }
            expectCreateCall::SELECTOR | expectCreate2Call::SELECTOR => {
                let bytecode = read_abi_dynamic_bytes_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.expectCreate bytecode",
                )?;
                let deployer = read_abi_word_arg(&state.memory, args_offset, 1)?;
                let kind = if selector == expectCreateCall::SELECTOR {
                    CreateKind::Create
                } else {
                    CreateKind::Create2
                };
                return Ok(self.set_expected_create(state, bytecode, deployer, kind));
            }
            clearMockedCallsCall::SELECTOR => {
                state.call_mocks.clear();
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            mockCall_0Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    1,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCall",
                )?;
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCall",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    None,
                    SymBytes::exprs(data),
                    vec![ret],
                    false,
                ));
            }
            mockCall_1Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockCall",
                )?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCall",
                )?;
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCall",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    Some(value),
                    SymBytes::exprs(data),
                    vec![ret],
                    false,
                ));
            }
            mockCall_2Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCall",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    None,
                    SymBytes::exprs(data),
                    vec![ret],
                    false,
                ));
            }
            mockCall_3Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockCall",
                )?;
                let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCall",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    Some(value),
                    SymBytes::exprs(data),
                    vec![ret],
                    false,
                ));
            }
            mockCalls_0Call::SELECTOR | mockCalls_1Call::SELECTOR => {
                let has_value = selector == mockCalls_1Call::SELECTOR;
                let (value, data_idx, ret_idx) = if has_value {
                    let value = read_abi_concrete_word_arg(
                        &state.memory,
                        args_offset,
                        1,
                        "symbolic vm.mockCalls",
                    )?;
                    (Some(value), 2, 3)
                } else {
                    (None, 1, 2)
                };
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    data_idx,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCalls data",
                )?;
                let returns = read_abi_symbolic_dynamic_bytes_array_arg(
                    state,
                    args_offset,
                    ret_idx,
                    self.config.max_dynamic_length as usize,
                    self.config.max_calldata_bytes as usize,
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    value,
                    SymBytes::exprs(data),
                    returns,
                    false,
                ));
            }
            mockCallRevert_0Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    1,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCallRevert",
                )?;
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCallRevert",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    None,
                    SymBytes::exprs(data),
                    vec![ret],
                    true,
                ));
            }
            mockCallRevert_1Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockCallRevert",
                )?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCallRevert",
                )?;
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCallRevert",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    Some(value),
                    SymBytes::exprs(data),
                    vec![ret],
                    true,
                ));
            }
            mockCallRevert_2Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCallRevert",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    None,
                    SymBytes::exprs(data),
                    vec![ret],
                    true,
                ));
            }
            mockCallRevert_3Call::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockCallRevert",
                )?;
                let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
                let ret = read_abi_dynamic_return_data_arg(
                    state,
                    args_offset,
                    3,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCallRevert",
                )?;
                return Ok(self.add_call_mock(
                    state,
                    callee,
                    Some(value),
                    SymBytes::exprs(data),
                    vec![ret],
                    true,
                ));
            }
            mockFunctionCall::SELECTOR => {
                let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let target = read_abi_address_arg(
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockFunction",
                )?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockFunction",
                )?;
                return Ok(self.set_function_mock(state, callee, target, SymBytes::exprs(data)));
            }
            prank_0Call::SELECTOR => {
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.prank.set_next(caller, None);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            prank_1Call::SELECTOR => {
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                let origin = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?;
                state.prank.set_next(caller, Some(origin));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            prank_2Call::SELECTOR => {
                let delegate_call =
                    read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.prank")?;
                if delegate_call {
                    return Err(SymbolicError::Unsupported("symbolic vm.prank delegatecall"));
                }
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.prank.set_next(caller, None);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            prank_3Call::SELECTOR => {
                let delegate_call =
                    read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.prank")?;
                if delegate_call {
                    return Err(SymbolicError::Unsupported("symbolic vm.prank delegatecall"));
                }
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                let origin = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?;
                state.prank.set_next(caller, Some(origin));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            startPrank_0Call::SELECTOR => {
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.prank.set_persistent(caller, None);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            startPrank_1Call::SELECTOR => {
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                let origin = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?;
                state.prank.set_persistent(caller, Some(origin));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            startPrank_2Call::SELECTOR => {
                let delegate_call =
                    read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.startPrank")?;
                if delegate_call {
                    return Err(SymbolicError::Unsupported("symbolic vm.startPrank delegatecall"));
                }
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.prank.set_persistent(caller, None);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            startPrank_3Call::SELECTOR => {
                let delegate_call =
                    read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.startPrank")?;
                if delegate_call {
                    return Err(SymbolicError::Unsupported("symbolic vm.startPrank delegatecall"));
                }
                let caller = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?;
                let origin = read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?;
                state.prank.set_persistent(caller, Some(origin));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            stopPrankCall::SELECTOR => {
                state.prank = SymbolicPrank::default();
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            readCallersCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(state.read_callers_words()));
            }
            addrCall::SELECTOR => {
                let private_key =
                    read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.addr")?;
                let address = private_key_address(private_key)?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(address_word(
                    address,
                ))]));
            }
            sign_1Call::SELECTOR => {
                let private_key =
                    read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.sign")?;
                let digest =
                    read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.sign")?;
                return Ok(CheatcodeOutcome::Continue(sign_hash_words(private_key, digest)?));
            }
            signCompact_1Call::SELECTOR => {
                let private_key = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.signCompact",
                )?;
                let digest = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    1,
                    "symbolic vm.signCompact",
                )?;
                return Ok(CheatcodeOutcome::Continue(sign_compact_hash_words(
                    private_key,
                    digest,
                )?));
            }
            deriveKey_0Call::SELECTOR => {
                let mnemonic =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
                let index =
                    read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
                let private_key = derive_private_key::<English>(
                    &mnemonic,
                    DEFAULT_DERIVATION_PATH_PREFIX,
                    index,
                )?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(private_key)]));
            }
            deriveKey_1Call::SELECTOR => {
                let mnemonic =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
                let path =
                    read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
                let index =
                    read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
                let private_key = derive_private_key::<English>(&mnemonic, &path, index)?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(private_key)]));
            }
            deriveKey_2Call::SELECTOR => {
                let mnemonic =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
                let index =
                    read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
                let language =
                    read_abi_string_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
                let private_key = derive_private_key_with_language(
                    &mnemonic,
                    DEFAULT_DERIVATION_PATH_PREFIX,
                    index,
                    &language,
                )?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(private_key)]));
            }
            deriveKey_3Call::SELECTOR => {
                let mnemonic =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
                let path =
                    read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
                let index =
                    read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
                let language =
                    read_abi_string_arg(&state.memory, args_offset, 3, "symbolic vm.deriveKey")?;
                let private_key =
                    derive_private_key_with_language(&mnemonic, &path, index, &language)?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(private_key)]));
            }
            rememberKeyCall::SELECTOR => {
                let private_key = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.rememberKey",
                )?;
                let address = private_key_address(private_key)?;
                state.wallets.insert(address);
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(address_word(
                    address,
                ))]));
            }
            rememberKeys_0Call::SELECTOR | rememberKeys_1Call::SELECTOR => {
                let mnemonic =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.rememberKeys")?;
                let path =
                    read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.rememberKeys")?;
                let (language, count_index) = if selector == rememberKeys_1Call::SELECTOR {
                    (
                        Some(read_abi_string_arg(
                            &state.memory,
                            args_offset,
                            2,
                            "symbolic vm.rememberKeys",
                        )?),
                        3,
                    )
                } else {
                    (None, 2)
                };
                let count = read_abi_u32_arg(
                    &state.memory,
                    args_offset,
                    count_index,
                    "symbolic vm.rememberKeys",
                )?;
                if count > MAX_REMEMBER_KEYS {
                    return Err(SymbolicError::Unsupported("symbolic vm.rememberKeys count"));
                }
                let mut addresses = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let private_key = if let Some(language) = &language {
                        derive_private_key_with_language(&mnemonic, &path, index, language)?
                    } else {
                        derive_private_key::<English>(&mnemonic, &path, index)?
                    };
                    let address = private_key_address(private_key)?;
                    state.wallets.insert(address);
                    addresses.push(DynSolValue::Address(address));
                }
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                    DynSolValue::Array(addresses),
                )));
            }
            getWalletsCall::SELECTOR => {
                let wallets = DynSolValue::Array(
                    state.wallets.iter().copied().map(DynSolValue::Address).collect(),
                );
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(wallets)));
            }
            storeCall::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let slot = state.memory.load_word(in_offset + 36)?;
                let value = state.memory.load_word(in_offset + 68)?;
                if target == CHEATCODE_ADDRESS
                    && slot == SymExpr::constant(failed_slot())
                    && value == SymExpr::constant(U256::from(1))
                {
                    return Ok(CheatcodeOutcome::Failure);
                }
                state.world.sstore(target, slot, value);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            loadCall::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let slot = state.memory.load_word(in_offset + 36)?;
                let concrete_slot = state.constrained_word(&slot);
                let value = state.world.sload(executor, target, slot, concrete_slot)?;
                return Ok(CheatcodeOutcome::Continue(vec![value]));
            }
            getNonce_0Call::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let nonce = state.world.nonce(executor, target)?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(nonce))]));
            }
            computeCreateAddressCall::SELECTOR => {
                let deployer = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let nonce = read_abi_word_arg(&state.memory, args_offset, 1)?;
                let address = compute_create_address_word(state, deployer, nonce)?;
                return Ok(CheatcodeOutcome::Continue(vec![address]));
            }
            computeCreate2Address_0Call::SELECTOR => {
                let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
                let deployer = read_abi_word_arg(&state.memory, args_offset, 2)?;
                let address = compute_create2_address_word(state, deployer, salt, init_code_hash)?;
                return Ok(CheatcodeOutcome::Continue(vec![address]));
            }
            computeCreate2Address_1Call::SELECTOR => {
                let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
                let address = compute_create2_address_word(
                    state,
                    SymExpr::constant(address_word(DEFAULT_CREATE2_DEPLOYER)),
                    salt,
                    init_code_hash,
                )?;
                return Ok(CheatcodeOutcome::Continue(vec![address]));
            }
            etchCall::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let code = read_abi_symbolic_dynamic_byte_exprs_arg(
                    state,
                    args_offset,
                    1,
                    self.config.max_dynamic_length as usize,
                    "symbolic vm.etch",
                )?;
                state.world.install_code(target, SymCode::from_byte_exprs(code));
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getCodeCall::SELECTOR | getDeployedCodeCall::SELECTOR => {
                let artifact =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.getCode")?;
                let code = artifact_code(&artifact, selector == getDeployedCodeCall::SELECTOR)?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&code)));
            }
            dealCall::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let value = read_abi_word_arg(&state.memory, args_offset, 1)?;
                if value.contains_gasleft() {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                let value = state.constrained_word(&value).map(SymExpr::constant).unwrap_or(value);
                state.world.set_balance_word(target, value);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            setNonceCall::SELECTOR | setNonceUnsafeCall::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let nonce =
                    read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.setNonce")?;
                let Ok(nonce) = u64::try_from(nonce) else {
                    return Err(SymbolicError::Unsupported("symbolic vm.setNonce nonce"));
                };
                if selector == setNonceCall::SELECTOR
                    && nonce < state.world.nonce(executor, target)?
                {
                    return Ok(CheatcodeOutcome::Failure);
                }
                state.world.set_nonce(target, nonce);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            resetNonceCall::SELECTOR => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let nonce = if state.world.extcode(executor, target)?.is_empty() { 0 } else { 1 };
                state.world.set_nonce(target, nonce);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            allowCheatcodesCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            makePersistent_0Call::SELECTOR => {
                let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.persistent_accounts.insert(account);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            makePersistent_1Call::SELECTOR => {
                let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
                state.persistent_accounts.insert(account0);
                state.persistent_accounts.insert(account1);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            makePersistent_2Call::SELECTOR => {
                let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
                let account2 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 2)?;
                state.persistent_accounts.insert(account0);
                state.persistent_accounts.insert(account1);
                state.persistent_accounts.insert(account2);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            makePersistent_3Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Array(Box::new(DynSolType::Address))],
                )?;
                for account in dyn_address_array(&values[0])? {
                    state.persistent_accounts.insert(account);
                }
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            revokePersistent_0Call::SELECTOR => {
                let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.persistent_accounts.remove(&account);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            revokePersistent_1Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Array(Box::new(DynSolType::Address))],
                )?;
                for account in dyn_address_array(&values[0])? {
                    state.persistent_accounts.remove(&account);
                }
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            isPersistentCall::SELECTOR => {
                let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    state.persistent_accounts.contains(&account),
                ))]));
            }
            activeForkCall::SELECTOR => {
                let id = executor.backend().active_fork_id().ok_or(SymbolicError::Unsupported(
                    "symbolic vm.activeFork requires an active forked executor",
                ))?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(id)]));
            }
            selectForkCall::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.selectFork id",
                )?;
                if executor.backend().is_active_fork(id) {
                    return Ok(CheatcodeOutcome::Continue(Vec::new()));
                }
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.selectFork can only select the already active fork",
                ));
            }
            rollFork_0Call::SELECTOR => {
                let block_number = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.rollFork block number",
                )?;
                let current = state
                    .block
                    .number
                    .clone()
                    .into_concrete("symbolic vm.rollFork current block")?;
                if block_number == current {
                    return Ok(CheatcodeOutcome::Continue(Vec::new()));
                }
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
                ));
            }
            rollFork_2Call::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.rollFork id",
                )?;
                let block_number = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    1,
                    "symbolic vm.rollFork block number",
                )?;
                let current = state
                    .block
                    .number
                    .clone()
                    .into_concrete("symbolic vm.rollFork current block")?;
                if executor.backend().is_active_fork(id) && block_number == current {
                    return Ok(CheatcodeOutcome::Continue(Vec::new()));
                }
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
                ));
            }
            createFork_0Call::SELECTOR
            | createFork_1Call::SELECTOR
            | createFork_2Call::SELECTOR
            | createSelectFork_0Call::SELECTOR
            | createSelectFork_1Call::SELECTOR
            | createSelectFork_2Call::SELECTOR
            | rollFork_1Call::SELECTOR
            | rollFork_3Call::SELECTOR => {
                return Err(SymbolicError::Unsupported(
                    "symbolic fork creation and fork block mutation must happen before symbolic execution",
                ));
            }
            snapshotCall::SELECTOR | snapshotStateCall::SELECTOR => {
                let id = state.world.snapshot_state();
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(id)]));
            }
            revertToCall::SELECTOR
            | revertToStateCall::SELECTOR
            | revertToAndDeleteCall::SELECTOR
            | revertToStateAndDeleteCall::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.revertToState snapshot",
                )?;
                let success = state.world.restore_snapshot(id);
                if success
                    && (selector == revertToAndDeleteCall::SELECTOR
                        || selector == revertToStateAndDeleteCall::SELECTOR)
                {
                    state.world.delete_snapshot(id);
                }
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    success,
                ))]));
            }
            deleteSnapshotCall::SELECTOR | deleteStateSnapshotCall::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.deleteStateSnapshot snapshot",
                )?;
                let success = state.world.delete_snapshot(id);
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    success,
                ))]));
            }
            deleteSnapshotsCall::SELECTOR | deleteStateSnapshotsCall::SELECTOR => {
                state.world.delete_snapshots();
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            warpCall::SELECTOR => {
                state.block.timestamp = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            rollCall::SELECTOR => {
                state.block.number = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            setBlockhashCall::SELECTOR => {
                let block_number = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.setBlockhash block number",
                )?;
                let block_hash = state.memory.load_word(in_offset + 36)?;
                state.block.set_block_hash(block_number, block_hash)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            prevrandao_0Call::SELECTOR | prevrandao_1Call::SELECTOR => {
                state.block.difficulty = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            blobhashesCall::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Array(Box::new(DynSolType::FixedBytes(32)))],
                )?;
                state.block.set_blob_hashes(dyn_bytes32_array(&values[0])?);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getBlobhashesCall::SELECTOR => {
                let value = DynSolValue::Array(
                    state
                        .block
                        .blob_hashes
                        .iter()
                        .copied()
                        .map(|hash| DynSolValue::FixedBytes(hash, 32))
                        .collect(),
                );
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
            }
            feeCall::SELECTOR => {
                state.block.basefee = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            blobBaseFeeCall::SELECTOR => {
                state.block.blob_basefee = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getBlobBaseFeeCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(vec![state.block.blob_basefee.clone()]));
            }
            chainIdCall::SELECTOR => {
                state.block.chain_id = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getChainIdCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(vec![state.block.chain_id.clone()]));
            }
            difficultyCall::SELECTOR => {
                state.block.difficulty = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            coinbaseCall::SELECTOR => {
                let coinbase = read_abi_constrained_address_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic vm.coinbase value",
                )?;
                state.block.coinbase = coinbase;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getBlockNumberCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(vec![state.block.number.clone()]));
            }
            txGasPriceCall::SELECTOR => {
                state.gas_price = state.memory.load_word(in_offset + 4)?;
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getBlockTimestampCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(vec![state.block.timestamp.clone()]));
            }
            labelCall::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Address, DynSolType::String],
                )?;
                let account = dyn_address(&values[0])?;
                let label = dyn_string(&values[1])?;
                state.labels.insert(account, label);
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            getLabelCall::SELECTOR => {
                let account =
                    read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.getLabel")?;
                let label = state
                    .labels
                    .get(&account)
                    .cloned()
                    .unwrap_or_else(|| format!("unlabeled:{account}"));
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    label.as_bytes(),
                )));
            }
            expectSafeMemoryCall::SELECTOR => {
                return Err(SymbolicError::Unsupported("symbolic vm.expectSafeMemory not modeled"));
            }
            expectSafeMemoryCallCall::SELECTOR => {
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.expectSafeMemoryCall not modeled",
                ));
            }
            stopExpectSafeMemoryCall::SELECTOR => {
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.stopExpectSafeMemory not modeled",
                ));
            }
            lastCallGasCall::SELECTOR => {
                return Err(SymbolicError::Unsupported("symbolic vm.lastCallGas not modeled"));
            }
            snapshotGasLastCall_0Call::SELECTOR | snapshotGasLastCall_1Call::SELECTOR => {
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.snapshotGasLastCall not modeled",
                ));
            }
            stopSnapshotGas_0Call::SELECTOR
            | stopSnapshotGas_1Call::SELECTOR
            | stopSnapshotGas_2Call::SELECTOR => {
                return Err(SymbolicError::Unsupported("symbolic vm.stopSnapshotGas not modeled"));
            }
            pauseGasMeteringCall::SELECTOR
            | resumeGasMeteringCall::SELECTOR
            | resetGasMeteringCall::SELECTOR
            | breakpoint_0Call::SELECTOR
            | breakpoint_1Call::SELECTOR
            | snapshotValue_0Call::SELECTOR
            | snapshotValue_1Call::SELECTOR
            | startSnapshotGas_0Call::SELECTOR
            | startSnapshotGas_1Call::SELECTOR
            | sleepCall::SELECTOR
            | coolCall::SELECTOR
            | accessListCall::SELECTOR
            | warmSlotCall::SELECTOR
            | coolSlotCall::SELECTOR
            | noAccessListCall::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            setEvmVersionCall::SELECTOR => {
                return Err(SymbolicError::Unsupported("symbolic vm.setEvmVersion not modeled"));
            }
            getEvmVersionCall::SELECTOR => {
                return Err(SymbolicError::Unsupported("symbolic vm.getEvmVersion not modeled"));
            }
            getFoundryVersionCall::SELECTOR => {
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    env!("CARGO_PKG_VERSION").as_bytes(),
                )));
            }
            projectRootCall::SELECTOR => {
                let root = std::env::current_dir()
                    .map_err(|_| SymbolicError::Unsupported("symbolic vm.projectRoot"))?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    root.display().to_string().as_bytes(),
                )));
            }
            unixTimeCall::SELECTOR => {
                let milliseconds = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?
                    .as_millis();
                let value = U256::try_from(milliseconds)
                    .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(value)]));
            }
            isContextCall::SELECTOR => {
                let context = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.isContext",
                )?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    context == U256::ZERO || context == U256::from(1),
                ))]));
            }
            toString_0Call::SELECTOR => {
                let address =
                    read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    format!("{address:?}").as_bytes(),
                )));
            }
            toString_1Call::SELECTOR => {
                let bytes = read_abi_dynamic_bytes_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.toString",
                )?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    format!("0x{}", hex::encode(bytes)).as_bytes(),
                )));
            }
            toString_2Call::SELECTOR => {
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.toString",
                )?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    format!("0x{}", hex::encode(value.to_be_bytes::<32>())).as_bytes(),
                )));
            }
            toString_3Call::SELECTOR => {
                let value =
                    read_abi_bool_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    if value { "true" } else { "false" }.as_bytes(),
                )));
            }
            toString_4Call::SELECTOR => {
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.toString",
                )?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    value.to_string().as_bytes(),
                )));
            }
            toString_5Call::SELECTOR => {
                let value = read_abi_concrete_word_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.toString",
                )?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    I256::from_raw(value).to_string().as_bytes(),
                )));
            }
            parseBytesCall::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes")?;
                let bytes = parse_env_bytes(&value)?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&bytes)));
            }
            parseAddressCall::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseAddress")?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(address_word(
                    parse_env_address(&value)?,
                ))]));
            }
            parseUintCall::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseUint")?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(parse_env_uint(
                    &value,
                )?)]));
            }
            parseIntCall::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseInt")?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(parse_env_int(
                    &value,
                )?)]));
            }
            parseBytes32Call::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes32")?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(parse_env_bytes32(
                    &value,
                )?)]));
            }
            parseBoolCall::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBool")?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    parse_env_bool(&value)?,
                ))]));
            }
            toLowercaseCall::SELECTOR | toUppercaseCall::SELECTOR | trimCall::SELECTOR => {
                let value =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.string")?;
                let output = if selector == toLowercaseCall::SELECTOR {
                    value.to_lowercase()
                } else if selector == toUppercaseCall::SELECTOR {
                    value.to_uppercase()
                } else {
                    value.trim().to_string()
                };
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    output.as_bytes(),
                )));
            }
            replaceCall::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::String, DynSolType::String],
                )?;
                let output = dyn_string(&values[0])?
                    .replace(&dyn_string(&values[1])?, &dyn_string(&values[2])?);
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    output.as_bytes(),
                )));
            }
            splitCall::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::String],
                )?;
                let input = dyn_string(&values[0])?;
                let delimiter = dyn_string(&values[1])?;
                let parts = if delimiter.is_empty() {
                    input.chars().map(|ch| DynSolValue::String(ch.to_string())).collect()
                } else {
                    input
                        .split(&delimiter)
                        .map(|part| DynSolValue::String(part.to_string()))
                        .collect()
                };
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                    DynSolValue::Array(parts),
                )));
            }
            indexOfCall::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::String],
                )?;
                let input = dyn_string(&values[0])?;
                let needle = dyn_string(&values[1])?;
                let index = input.find(&needle).map(U256::from).unwrap_or(U256::MAX);
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(index)]));
            }
            containsCall::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::String],
                )?;
                let contains = dyn_string(&values[0])?.contains(&dyn_string(&values[1])?);
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    contains,
                ))]));
            }
            toBase64_0Call::SELECTOR
            | toBase64_1Call::SELECTOR
            | toBase64URL_0Call::SELECTOR
            | toBase64URL_1Call::SELECTOR => {
                let data = read_abi_dynamic_bytes_arg(
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.toBase64",
                )?;
                let encoded = if selector == toBase64URL_0Call::SELECTOR
                    || selector == toBase64URL_1Call::SELECTOR
                {
                    BASE64_URL_SAFE.encode(data)
                } else {
                    BASE64_STANDARD.encode(data)
                };
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    encoded.as_bytes(),
                )));
            }
            bound_0Call::SELECTOR => {
                return self.handle_bound_uint(state, args_offset);
            }
            bound_1Call::SELECTOR => {
                return self.handle_bound_int(state, args_offset);
            }
            envExistsCall::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envExists")?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    std::env::var_os(name).is_some(),
                ))]));
            }
            envBool_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBool")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(U256::from(
                    parse_env_bool(&value)?,
                ))]));
            }
            envUint_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envUint")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(parse_env_uint(
                    &value,
                )?)]));
            }
            envInt_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envInt")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(parse_env_int(
                    &value,
                )?)]));
            }
            envAddress_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envAddress")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                let address = parse_env_address(&value)?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(address_word(
                    address,
                ))]));
            }
            envBytes32_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes32")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(parse_env_bytes32(
                    &value,
                )?)]));
            }
            envString_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envString")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    value.as_bytes(),
                )));
            }
            envBytes_0Call::SELECTOR => {
                let name =
                    read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes")?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                let bytes = parse_env_bytes(&value)?;
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&bytes)));
            }
            envBool_1Call::SELECTOR
            | envUint_1Call::SELECTOR
            | envInt_1Call::SELECTOR
            | envAddress_1Call::SELECTOR
            | envBytes32_1Call::SELECTOR
            | envString_1Call::SELECTOR
            | envBytes_1Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::String],
                )?;
                let name = dyn_string(&values[0])?;
                let delimiter = dyn_string(&values[1])?;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                let value = if selector == envBool_1Call::SELECTOR {
                    parse_env_array(&value, &delimiter, parse_env_bool_value)?
                } else if selector == envUint_1Call::SELECTOR {
                    parse_env_array(&value, &delimiter, parse_env_uint_value)?
                } else if selector == envInt_1Call::SELECTOR {
                    parse_env_array(&value, &delimiter, parse_env_int_value)?
                } else if selector == envAddress_1Call::SELECTOR {
                    parse_env_array(&value, &delimiter, parse_env_address_value)?
                } else if selector == envBytes32_1Call::SELECTOR {
                    parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
                } else if selector == envString_1Call::SELECTOR {
                    parse_env_array(&value, &delimiter, parse_env_string_value)?
                } else {
                    parse_env_array(&value, &delimiter, parse_env_bytes_value)?
                };
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
            }
            envOr_0Call::SELECTOR => {
                let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
                let value = match std::env::var(name) {
                    Ok(value) => U256::from(parse_env_bool(&value)?),
                    Err(_) => read_abi_concrete_word_arg(
                        &state.memory,
                        args_offset,
                        1,
                        "symbolic vm.envOr",
                    )?,
                };
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(value)]));
            }
            envOr_1Call::SELECTOR
            | envOr_2Call::SELECTOR
            | envOr_3Call::SELECTOR
            | envOr_4Call::SELECTOR => {
                let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
                let default =
                    read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?;
                let value = match std::env::var(name) {
                    Ok(value) if selector == envOr_1Call::SELECTOR => parse_env_uint(&value)?,
                    Ok(value) if selector == envOr_2Call::SELECTOR => parse_env_int(&value)?,
                    Ok(value) if selector == envOr_3Call::SELECTOR => {
                        address_word(parse_env_address(&value)?)
                    }
                    Ok(value) => parse_env_bytes32(&value)?,
                    Err(_) => default,
                };
                return Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(value)]));
            }
            envOr_5Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::String],
                )?;
                let name = dyn_string(&values[0])?;
                let value = std::env::var(name).unwrap_or(dyn_string(&values[1])?);
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    value.as_bytes(),
                )));
            }
            envOr_6Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::String, DynSolType::Bytes],
                )?;
                let name = dyn_string(&values[0])?;
                let value = match std::env::var(name) {
                    Ok(value) => parse_env_bytes(&value)?,
                    Err(_) => dyn_bytes(&values[1])?,
                };
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&value)));
            }
            envOr_7Call::SELECTOR
            | envOr_8Call::SELECTOR
            | envOr_9Call::SELECTOR
            | envOr_10Call::SELECTOR
            | envOr_11Call::SELECTOR
            | envOr_12Call::SELECTOR
            | envOr_13Call::SELECTOR => {
                let element_ty = if selector == envOr_7Call::SELECTOR {
                    DynSolType::Bool
                } else if selector == envOr_8Call::SELECTOR {
                    DynSolType::Uint(256)
                } else if selector == envOr_9Call::SELECTOR {
                    DynSolType::Int(256)
                } else if selector == envOr_10Call::SELECTOR {
                    DynSolType::Address
                } else if selector == envOr_11Call::SELECTOR {
                    DynSolType::FixedBytes(32)
                } else if selector == envOr_12Call::SELECTOR {
                    DynSolType::String
                } else {
                    DynSolType::Bytes
                };
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![
                        DynSolType::String,
                        DynSolType::String,
                        DynSolType::Array(Box::new(element_ty)),
                    ],
                )?;
                let name = dyn_string(&values[0])?;
                let delimiter = dyn_string(&values[1])?;
                let value = match std::env::var(name) {
                    Ok(value) if selector == envOr_7Call::SELECTOR => {
                        parse_env_array(&value, &delimiter, parse_env_bool_value)?
                    }
                    Ok(value) if selector == envOr_8Call::SELECTOR => {
                        parse_env_array(&value, &delimiter, parse_env_uint_value)?
                    }
                    Ok(value) if selector == envOr_9Call::SELECTOR => {
                        parse_env_array(&value, &delimiter, parse_env_int_value)?
                    }
                    Ok(value) if selector == envOr_10Call::SELECTOR => {
                        parse_env_array(&value, &delimiter, parse_env_address_value)?
                    }
                    Ok(value) if selector == envOr_11Call::SELECTOR => {
                        parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
                    }
                    Ok(value) if selector == envOr_12Call::SELECTOR => {
                        parse_env_array(&value, &delimiter, parse_env_string_value)?
                    }
                    Ok(value) => parse_env_array(&value, &delimiter, parse_env_bytes_value)?,
                    Err(_) => values[2].clone(),
                };
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
            }
            ffiCall::SELECTOR => {
                if !state.ffi_enabled {
                    return Err(SymbolicError::Unsupported("symbolic ffi disabled"));
                }
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    vec![DynSolType::Array(Box::new(DynSolType::String))],
                )?;
                let args = dyn_string_array(&values[0])?;
                if args.is_empty() || args[0].is_empty() {
                    return Err(SymbolicError::Unsupported("symbolic ffi empty command"));
                }
                let output = Command::new(&args[0])
                    .args(&args[1..])
                    .output()
                    .map_err(|_| SymbolicError::Unsupported("symbolic ffi command"))?;
                if !output.status.success() {
                    return Err(SymbolicError::Unsupported("symbolic ffi command failed"));
                }
                let stdout = String::from_utf8(output.stdout)
                    .map_err(|_| SymbolicError::Unsupported("symbolic ffi stdout"))?;
                let trimmed = stdout.trim();
                let bytes = hex::decode(trimmed).unwrap_or_else(|_| trimmed.as_bytes().to_vec());
                return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&bytes)));
            }
            assertTrue_0Call::SELECTOR | assertTrue_1Call::SELECTOR => {
                let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.nonzero_bool();
                return self.handle_assertion(state, condition);
            }
            assertFalse_0Call::SELECTOR | assertFalse_1Call::SELECTOR => {
                let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.into_zero_bool();
                return self.handle_assertion(state, condition);
            }
            assertEq_2Call::SELECTOR
            | assertEq_3Call::SELECTOR
            | assertEq_4Call::SELECTOR
            | assertEq_5Call::SELECTOR
            | assertEq_6Call::SELECTOR
            | assertEq_7Call::SELECTOR
            | assertEq_8Call::SELECTOR
            | assertEq_9Call::SELECTOR
            | assertEq_0Call::SELECTOR
            | assertEq_1Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self.handle_assertion(state, SymBoolExpr::eq(left, right));
            }
            assertEq_10Call::SELECTOR | assertEq_11Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    if selector == assertEq_10Call::SELECTOR {
                        vec![DynSolType::String, DynSolType::String]
                    } else {
                        vec![DynSolType::String, DynSolType::String, DynSolType::String]
                    },
                )?;
                return self.handle_assertion(
                    state,
                    SymBoolExpr::constant(dyn_string(&values[0])? == dyn_string(&values[1])?),
                );
            }
            assertEq_12Call::SELECTOR | assertEq_13Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    if selector == assertEq_12Call::SELECTOR {
                        vec![DynSolType::Bytes, DynSolType::Bytes]
                    } else {
                        vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                    },
                )?;
                return self.handle_assertion(
                    state,
                    SymBoolExpr::constant(dyn_bytes(&values[0])? == dyn_bytes(&values[1])?),
                );
            }
            assertEq_14Call::SELECTOR
            | assertEq_15Call::SELECTOR
            | assertEq_16Call::SELECTOR
            | assertEq_17Call::SELECTOR
            | assertEq_18Call::SELECTOR
            | assertEq_19Call::SELECTOR
            | assertEq_20Call::SELECTOR
            | assertEq_21Call::SELECTOR
            | assertEq_22Call::SELECTOR
            | assertEq_23Call::SELECTOR
            | assertEq_24Call::SELECTOR
            | assertEq_25Call::SELECTOR
            | assertEq_26Call::SELECTOR
            | assertEq_27Call::SELECTOR => {
                let element_ty = array_assertion_element_type(selector)?;
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    if selector_has_string_reason(selector) {
                        vec![
                            DynSolType::Array(Box::new(element_ty.clone())),
                            DynSolType::Array(Box::new(element_ty)),
                            DynSolType::String,
                        ]
                    } else {
                        vec![
                            DynSolType::Array(Box::new(element_ty.clone())),
                            DynSolType::Array(Box::new(element_ty)),
                        ]
                    },
                )?;
                return self.handle_assertion(state, SymBoolExpr::constant(values[0] == values[1]));
            }
            assertEqDecimal_0Call::SELECTOR
            | assertEqDecimal_1Call::SELECTOR
            | assertEqDecimal_2Call::SELECTOR
            | assertEqDecimal_3Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self.handle_assertion(state, SymBoolExpr::eq(left, right));
            }
            assertNotEq_2Call::SELECTOR
            | assertNotEq_3Call::SELECTOR
            | assertNotEq_4Call::SELECTOR
            | assertNotEq_5Call::SELECTOR
            | assertNotEq_6Call::SELECTOR
            | assertNotEq_7Call::SELECTOR
            | assertNotEq_8Call::SELECTOR
            | assertNotEq_9Call::SELECTOR
            | assertNotEq_0Call::SELECTOR
            | assertNotEq_1Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self.handle_assertion(state, SymBoolExpr::eq(left, right).not());
            }
            assertNotEq_10Call::SELECTOR | assertNotEq_11Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    if selector == assertNotEq_10Call::SELECTOR {
                        vec![DynSolType::String, DynSolType::String]
                    } else {
                        vec![DynSolType::String, DynSolType::String, DynSolType::String]
                    },
                )?;
                return self.handle_assertion(
                    state,
                    SymBoolExpr::constant(dyn_string(&values[0])? != dyn_string(&values[1])?),
                );
            }
            assertNotEq_12Call::SELECTOR | assertNotEq_13Call::SELECTOR => {
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    if selector == assertNotEq_12Call::SELECTOR {
                        vec![DynSolType::Bytes, DynSolType::Bytes]
                    } else {
                        vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                    },
                )?;
                return self.handle_assertion(
                    state,
                    SymBoolExpr::constant(dyn_bytes(&values[0])? != dyn_bytes(&values[1])?),
                );
            }
            assertNotEq_14Call::SELECTOR
            | assertNotEq_15Call::SELECTOR
            | assertNotEq_16Call::SELECTOR
            | assertNotEq_17Call::SELECTOR
            | assertNotEq_18Call::SELECTOR
            | assertNotEq_19Call::SELECTOR
            | assertNotEq_20Call::SELECTOR
            | assertNotEq_21Call::SELECTOR
            | assertNotEq_22Call::SELECTOR
            | assertNotEq_23Call::SELECTOR
            | assertNotEq_24Call::SELECTOR
            | assertNotEq_25Call::SELECTOR
            | assertNotEq_26Call::SELECTOR
            | assertNotEq_27Call::SELECTOR => {
                let element_ty = array_assertion_element_type(selector)?;
                let values = decode_cheatcode_args(
                    state,
                    in_offset,
                    in_size,
                    if selector_has_string_reason(selector) {
                        vec![
                            DynSolType::Array(Box::new(element_ty.clone())),
                            DynSolType::Array(Box::new(element_ty)),
                            DynSolType::String,
                        ]
                    } else {
                        vec![
                            DynSolType::Array(Box::new(element_ty.clone())),
                            DynSolType::Array(Box::new(element_ty)),
                        ]
                    },
                )?;
                return self.handle_assertion(state, SymBoolExpr::constant(values[0] != values[1]));
            }
            assertLt_0Call::SELECTOR | assertLt_1Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self
                    .handle_assertion(state, SymBoolExpr::cmp(SymBoolExprOp::Ult, left, right));
            }
            assertLe_0Call::SELECTOR | assertLe_1Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self
                    .handle_assertion(state, SymBoolExpr::cmp(SymBoolExprOp::Ule, left, right));
            }
            assertGt_0Call::SELECTOR | assertGt_1Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self
                    .handle_assertion(state, SymBoolExpr::cmp(SymBoolExprOp::Ugt, left, right));
            }
            assertGe_0Call::SELECTOR | assertGe_1Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self
                    .handle_assertion(state, SymBoolExpr::cmp(SymBoolExprOp::Uge, left, right));
            }
            assertLt_2Call::SELECTOR | assertLt_3Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self
                    .handle_assertion(state, SymBoolExpr::cmp(SymBoolExprOp::Slt, left, right));
            }
            assertGt_2Call::SELECTOR | assertGt_3Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self
                    .handle_assertion(state, SymBoolExpr::cmp(SymBoolExprOp::Sgt, left, right));
            }
            assertLe_2Call::SELECTOR | assertLe_3Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self.handle_assertion(
                    state,
                    SymBoolExpr::cmp(SymBoolExprOp::Sgt, left, right).not(),
                );
            }
            assertGe_2Call::SELECTOR | assertGe_3Call::SELECTOR => {
                let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
                return self.handle_assertion(
                    state,
                    SymBoolExpr::cmp(SymBoolExprOp::Slt, left, right).not(),
                );
            }
            randomUint_0Call::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomUint")]));
            }
            randomUint_2Call::SELECTOR => {
                let bits = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic randomUint bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic randomUint bits")?;
                return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(bits)]));
            }
            randomUint_1Call::SELECTOR => {
                let min = state.memory.load_word(in_offset + 4)?;
                let max = state.memory.load_word(in_offset + 36)?;
                let value = state.fresh_word("vmRandomUintRange");
                state.constraints.push(SymBoolExpr::cmp_word_expr(SymBoolExprOp::Uge, &value, min));
                state.constraints.push(SymBoolExpr::cmp_word_expr(SymBoolExprOp::Ule, &value, max));
                return Ok(CheatcodeOutcome::Continue(vec![value]));
            }
            randomInt_0Call::SELECTOR => {
                return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomInt")]));
            }
            randomInt_1Call::SELECTOR => {
                let bits = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic randomInt bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic randomInt bits")?;
                return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_int(bits)]));
            }
            randomAddressCall::SELECTOR => {
                let value = state.fresh_bounded_uint(U256::from(160));
                return Ok(CheatcodeOutcome::Continue(vec![value]));
            }
            randomBoolCall::SELECTOR => {
                let value = state.fresh_bounded_uint(U256::from(1));
                return Ok(CheatcodeOutcome::Continue(vec![value]));
            }
            randomBytesCall::SELECTOR => {
                let len = read_abi_word_arg(&state.memory, args_offset, 0)?;
                let max_limit = self.config.max_dynamic_length as usize;
                let max_len = state
                    .upper_bound_usize(&len)
                    .filter(|len| *len <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &len,
                            max_limit,
                            "symbolic randomBytes length",
                        )
                    })?;
                return Ok(CheatcodeOutcome::ContinueData(abi_bytes_return_with_len(
                    len,
                    state.fresh_bytes(max_len),
                )));
            }
            randomBytes4Call::SELECTOR => {
                let value = state.fresh_bounded_uint(U256::from(32));
                return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 224)]));
            }
            randomBytes8Call::SELECTOR => {
                let value = state.fresh_bounded_uint(U256::from(64));
                return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 192)]));
            }

            _ => {}
        }

        Err(SymbolicError::Unsupported("symbolic Foundry cheatcode"))
    }

    pub(super) fn handle_symbolic_vm_cheatcode(
        &mut self,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        let Some(cheatcode) = SymbolicVmCheatcode::from_selector(selector) else {
            return Err(SymbolicError::Unsupported("symbolic VM compatibility cheatcode"));
        };
        let args_offset = in_offset + 4;

        match cheatcode {
            SymbolicVmCheatcode::CreateUintBits(bits) => {
                let value = if bits == 256 {
                    state.fresh_word("svm")
                } else {
                    state.fresh_bounded_uint(U256::from(bits))
                };
                Ok(SymReturnData::from_words(vec![value]))
            }
            SymbolicVmCheatcode::CreateIntBits(bits) => {
                let value = if bits == 256 {
                    state.fresh_word("svm")
                } else {
                    state.fresh_bounded_int(U256::from(bits))
                };
                Ok(SymReturnData::from_words(vec![value]))
            }
            SymbolicVmCheatcode::CreateBytesFixed(bytes) => {
                let value = if bytes == 32 {
                    state.fresh_word("svm")
                } else {
                    shift_left(state.fresh_bounded_uint(U256::from(bytes * 8)), (32 - bytes) * 8)
                };
                Ok(SymReturnData::from_words(vec![value]))
            }
            SymbolicVmCheatcode::CreateUint => {
                let bits = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic svm.create integer bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic svm.create integer bits")?;
                Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(bits)]))
            }
            SymbolicVmCheatcode::CreateInt => {
                let bits = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic svm.create integer bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic svm.create integer bits")?;
                Ok(SymReturnData::from_words(vec![state.fresh_bounded_int(bits)]))
            }
            SymbolicVmCheatcode::CreateAddress => {
                Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(160))]))
            }
            SymbolicVmCheatcode::CreateBool => {
                Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(1))]))
            }
            SymbolicVmCheatcode::CreateBytes => {
                Ok(abi_bytes_return(state.fresh_bytes(self.config.default_dynamic_length as usize)))
            }
            SymbolicVmCheatcode::CreateBytesSized => {
                let len = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic svm.createBytes length",
                )?;
                let len = usize::try_from(len)
                    .ok()
                    .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                    .ok_or(SymbolicError::Unsupported("symbolic svm.createBytes length"))?;
                Ok(abi_bytes_return(state.fresh_bytes(len)))
            }
            SymbolicVmCheatcode::CreateString => Ok(abi_bytes_return(
                state.fresh_printable_ascii_bytes(self.config.default_dynamic_length as usize),
            )),
            SymbolicVmCheatcode::CreateStringSized => {
                let len = read_abi_constrained_word_arg(
                    state,
                    args_offset,
                    0,
                    "symbolic svm.createString length",
                )?;
                let len = usize::try_from(len)
                    .ok()
                    .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                    .ok_or(SymbolicError::Unsupported("symbolic svm.createString length"))?;
                Ok(abi_bytes_return(state.fresh_printable_ascii_bytes(len)))
            }
            SymbolicVmCheatcode::CreateCalldata => {
                let max = self.config.max_calldata_bytes as usize;
                let len = if max < 4 {
                    max
                } else {
                    (self.config.default_dynamic_length as usize).max(4).min(max)
                };
                Ok(abi_bytes_return(state.fresh_bytes(len)))
            }
            SymbolicVmCheatcode::EnableSymbolicStorage => {
                let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                state.world.enable_arbitrary_storage(target);
                Ok(SymReturnData::default())
            }
            SymbolicVmCheatcode::SnapshotStorage => {
                let _target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
                let id = state.world.snapshot_state();
                Ok(SymReturnData::from_words(vec![SymExpr::constant(id)]))
            }
            SymbolicVmCheatcode::SnapshotState => {
                let id = state.world.snapshot_state();
                Ok(SymReturnData::from_words(vec![SymExpr::constant(id)]))
            }
        }
    }
}
