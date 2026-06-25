use super::*;

impl SymbolicExecutor {
    /// Runs the `handle_assertion` symbolic executor helper.
    pub(super) fn handle_assertion(
        &mut self,
        state: &mut PathState,
        pass: BoolExpr,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let fail = pass.clone().not();
        match fail {
            BoolExpr::Const(true) => return Ok(CheatcodeOutcome::Failure),
            BoolExpr::Const(false) => return Ok(CheatcodeOutcome::Continue(Vec::new())),
            _ => {}
        }

        if bool_contains_gasleft(&pass) {
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

    /// Applies the `set_expected_revert` symbolic executor helper.
    pub(super) fn set_expected_revert(
        &mut self,
        state: &mut PathState,
        data: ExpectedRevertData,
        reverter: Option<SymWord>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_revert =
            Some(ExpectedRevert { data, reverter, remaining: remaining.max(1) });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `set_expected_emit` symbolic executor helper.
    pub(super) fn set_expected_emit(
        &mut self,
        state: &mut PathState,
        checks: ExpectedEmitChecks,
        emitter: Option<SymWord>,
        remaining: u64,
    ) -> CheatcodeOutcome {
        state.expected_emit =
            Some(ExpectedEmit { checks, emitter, remaining: remaining.max(1), template: None });
        CheatcodeOutcome::Continue(Vec::new())
    }

    #[expect(clippy::too_many_arguments)]
    /// Applies the `set_expected_call` symbolic executor helper.
    pub(super) fn set_expected_call(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        value: Option<U256>,
        gas: Option<u64>,
        min_gas: Option<u64>,
        data: Vec<SymWord>,
        count: Option<u64>,
    ) -> CheatcodeOutcome {
        let (gas, min_gas) = adjust_expected_call_gas_for_value(value, gas, min_gas);
        state.expected_calls.push(ExpectedCall {
            callee,
            value,
            gas,
            min_gas,
            data,
            expected: count.unwrap_or(1).max(1),
            observed: 0,
            exact: count.is_some(),
        });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `set_expected_create` symbolic executor helper.
    pub(super) fn set_expected_create(
        &mut self,
        state: &mut PathState,
        bytecode: Vec<u8>,
        deployer: SymWord,
        kind: CreateKind,
    ) -> CheatcodeOutcome {
        state.expected_creates.push(ExpectedCreate { bytecode, deployer, kind });
        CheatcodeOutcome::Continue(Vec::new())
    }

    #[expect(clippy::too_many_arguments)]
    /// Implements the `deploy_code_cheatcode_if_needed` symbolic executor helper.
    pub(super) fn deploy_code_cheatcode_if_needed<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymWord,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let args_offset = in_offset + 4;
        let (artifact, constructor_args) = if selector == selector!("deployCode(string)") {
            let artifact =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deployCode")?;
            (artifact, Vec::new())
        } else if selector == selector!("deployCode(string,bytes)") {
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
    /// Applies the `deploy_code_cheatcode_call` symbolic executor helper.
    pub(super) fn deploy_code_cheatcode_call<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        artifact: String,
        constructor_args: Vec<u8>,
        out_offset: SymWord,
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
        let created_word = SymWord::Concrete(address_word(created));

        let mut failure_world = state.world.clone();
        failure_world.increment_nonce(executor, state.address)?;
        if failure_world.has_code_or_nonce(executor, created)? {
            state.world = failure_world;
            complete_cheatcode_call(
                state,
                out_offset,
                out_size,
                SymReturnData::from_words(vec![SymWord::zero()]),
            )?;
            return Ok(StepOutcome::Continue);
        }

        let mut frame = CallFrame::new(
            created,
            created,
            created,
            state.address,
            SymWord::zero(),
            false,
            SymCalldata::new(Vec::new()),
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
                            SymReturnData::from_words(vec![SymWord::zero()]),
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
                    if !parent.world.destroyed_accounts.contains(&created) {
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
                    let return_data = parent.return_data.clone();
                    parent.memory.copy_call_output_offset(
                        out_offset.clone(),
                        out_size,
                        &return_data,
                    )?;
                    parent.stack.push(SymWord::zero())?;
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

    /// Applies the `observe_expected_create` symbolic executor helper.
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
            let expected = state.expected_creates[idx].clone();
            if expected.kind != kind || expected.bytecode != bytecode {
                continue;
            }

            let condition = address_match_condition(&expected.deployer, deployer);
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

    /// Implements the `branch_accesses_cheatcode_if_needed` symbolic executor helper.
    pub(super) fn branch_accesses_cheatcode_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        in_offset: usize,
        out_offset: SymWord,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        if selector != selector!("accesses(address)") {
            return Ok(None);
        }

        let Some(record) = state.access_record.clone() else {
            return Ok(None);
        };
        let target = read_abi_word_arg(&state.memory, in_offset + 4, 0)?;
        if matches!(target, SymWord::Concrete(_)) {
            return Ok(None);
        }

        let addresses =
            record.reads.keys().chain(record.writes.keys()).copied().collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Ok(None);
        }

        let mut branches = VecDeque::new();
        let mut matched_conditions = Vec::new();
        for address in addresses {
            let condition = address_match_condition(&target, address);
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
            BoolExpr::and(matched_conditions.into_iter().map(BoolExpr::not).collect());
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

    /// Implements the `accesses_return_data_for_target` symbolic executor helper.
    pub(super) fn accesses_return_data_for_target(
        &mut self,
        state: &mut PathState,
        target: SymWord,
    ) -> Result<SymReturnData, SymbolicError> {
        let Some(record) = state.access_record.clone() else {
            return Ok(accesses_return_data(None, Address::ZERO));
        };

        if let SymWord::Concrete(target) = target {
            return Ok(accesses_return_data(Some(&record), word_to_address(target)));
        }

        let addresses =
            record.reads.keys().chain(record.writes.keys()).copied().collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Ok(accesses_return_data(Some(&record), Address::ZERO));
        }

        for address in addresses {
            let condition = address_match_condition(&target, address);
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

    /// Implements the `add_call_mock` symbolic executor helper.
    pub(super) fn add_call_mock(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        value: Option<U256>,
        data: Vec<SymWord>,
        returns: Vec<SymReturnData>,
        reverts: bool,
    ) -> CheatcodeOutcome {
        state.call_mocks.push(CallMock { callee, value, data, returns, reverts, calls: 0 });
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Applies the `set_function_mock` symbolic executor helper.
    pub(super) fn set_function_mock(
        &mut self,
        state: &mut PathState,
        callee: SymWord,
        target: Address,
        data: Vec<SymWord>,
    ) -> CheatcodeOutcome {
        if let Some(mock) =
            state.function_mocks.iter_mut().find(|mock| mock.callee == callee && mock.data == data)
        {
            mock.target = target;
        } else {
            state.function_mocks.push(FunctionMock { callee, target, data });
        }
        CheatcodeOutcome::Continue(Vec::new())
    }

    /// Runs the `handle_foundry_cheatcode` symbolic executor helper.
    pub(super) fn handle_foundry_cheatcode<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
        in_size: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let args_offset = in_offset + 4;
        if selector == selector!("assume(bool)") {
            return self.handle_assume(state, in_offset + 4);
        }
        if selector == selector!("assumeNoRevert()") {
            if state.assume_no_revert_next_call.is_some() {
                return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
            }
            state.assume_no_revert_next_call = Some(AssumeNoRevert::Any);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("assumeNoRevert((address,bool,bytes))") {
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
        if selector == selector!("assumeNoRevert((address,bool,bytes)[])") {
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
        if selector == selector!("skip(bool)") || selector == selector!("skip(bool,string)") {
            return self.handle_skip(state, in_offset + 4);
        }
        if selector == selector!("recordLogs()") {
            state.recorded_logs = Some(Vec::new());
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("record()") {
            state.access_record = Some(AccessRecord::default());
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("stopRecord()") {
            state.access_record = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("accesses(address)") {
            let target = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(CheatcodeOutcome::ContinueData(
                self.accesses_return_data_for_target(state, target)?,
            ));
        }
        if selector == selector!("getRecordedLogs()") {
            let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
            return Ok(CheatcodeOutcome::ContinueData(recorded_logs_return_data(logs)));
        }
        if selector == selector!("getRecordedLogsJson()") {
            let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
            return Ok(CheatcodeOutcome::ContinueData(recorded_logs_json_return_data(logs)?));
        }
        if selector == selector!("expectRevert()") {
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, 1));
        }
        if selector == selector!("expectRevert(bytes4)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                1,
            ));
        }
        if selector == selector!("expectRevert(bytes)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Exact(data), None, 1));
        }
        if selector == selector!("expectRevert(address)") {
            let reverter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, Some(reverter), 1));
        }
        if selector == selector!("expectRevert(bytes4,address)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectRevert(bytes,address)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                0,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectRevert",
            )?;
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Exact(data),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectRevert(uint64)") {
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(state, ExpectedRevertData::Any, None, count));
        }
        if selector == selector!("expectRevert(bytes4,uint64)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes,uint64)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
                ExpectedRevertData::Exact(data),
                None,
                count,
            ));
        }
        if selector == selector!("expectRevert(address,uint64)") {
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
        if selector == selector!("expectRevert(bytes4,address,uint64)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let count =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectRevert")?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectRevert(bytes,address,uint64)") {
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
                ExpectedRevertData::Exact(data),
                Some(reverter),
                count,
            ));
        }
        if selector == selector!("expectPartialRevert(bytes4)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                None,
                1,
            ));
        }
        if selector == selector!("expectPartialRevert(bytes4,address)") {
            let selector = read_abi_bytes4_words_arg(&state.memory, args_offset, 0);
            let reverter = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return Ok(self.set_expected_revert(
                state,
                ExpectedRevertData::Prefix(selector),
                Some(reverter),
                1,
            ));
        }
        if selector == selector!("expectEmit()") {
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                None,
                1,
            ));
        }
        if selector == selector!("expectEmit(address)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                Some(emitter),
                1,
            ));
        }
        if selector == selector!("expectEmit(uint64)") {
            let count = read_abi_u64_arg(&state.memory, args_offset, 0, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                None,
                count,
            ));
        }
        if selector == selector!("expectEmit(address,uint64)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 1, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_non_anonymous(),
                Some(emitter),
                count,
            ));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            return Ok(self.set_expected_emit(state, checks, None, 1));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,address)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,uint64)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(state, checks, None, count));
        }
        if selector == selector!("expectEmit(bool,bool,bool,bool,address,uint64)") {
            let checks = ExpectedEmitChecks::from_non_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 4)?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 5, "symbolic vm.expectEmit")?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), count));
        }
        if selector == selector!("expectEmitAnonymous()") {
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_anonymous(),
                None,
                1,
            ));
        }
        if selector == selector!("expectEmitAnonymous(address)") {
            let emitter = read_abi_word_arg(&state.memory, args_offset, 0)?;
            return Ok(self.set_expected_emit(
                state,
                ExpectedEmitChecks::default_anonymous(),
                Some(emitter),
                1,
            ));
        }
        if selector == selector!("expectEmitAnonymous(bool,bool,bool,bool,bool)") {
            let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
            return Ok(self.set_expected_emit(state, checks, None, 1));
        }
        if selector == selector!("expectEmitAnonymous(bool,bool,bool,bool,bool,address)") {
            let checks = ExpectedEmitChecks::from_anonymous_args(&state.memory, args_offset)?;
            let emitter = read_abi_word_arg(&state.memory, args_offset, 5)?;
            return Ok(self.set_expected_emit(state, checks, Some(emitter), 1));
        }
        if selector == selector!("expectCall(address,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(state, callee, None, None, None, data, None));
        }
        if selector == selector!("expectCall(address,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(state, callee, None, None, None, data, Some(count)));
        }
        if selector == selector!("expectCall(address,uint256,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            return Ok(self.set_expected_call(state, callee, Some(value), None, None, data, None));
        }
        if selector == selector!("expectCall(address,uint256,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 3, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                None,
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCall(address,uint256,uint64,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let gas = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
                data,
                None,
            ));
        }
        if selector == selector!("expectCall(address,uint256,uint64,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let gas = read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                Some(gas),
                None,
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCallMinGas(address,uint256,uint64,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let min_gas =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
                data,
                None,
            ));
        }
        if selector == selector!("expectCallMinGas(address,uint256,uint64,bytes,uint64)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.expectCall",
            )?;
            let min_gas =
                read_abi_u64_arg(&state.memory, args_offset, 2, "symbolic vm.expectCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.expectCall",
            )?;
            let count = read_abi_u64_arg(&state.memory, args_offset, 4, "symbolic vm.expectCall")?;
            return Ok(self.set_expected_call(
                state,
                callee,
                Some(value),
                None,
                Some(min_gas),
                data,
                Some(count),
            ));
        }
        if selector == selector!("expectCreate(bytes,address)")
            || selector == selector!("expectCreate2(bytes,address)")
        {
            let bytecode = read_abi_dynamic_bytes_arg(
                &state.memory,
                args_offset,
                0,
                "symbolic vm.expectCreate bytecode",
            )?;
            let deployer = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let kind = if selector == selector!("expectCreate(bytes,address)") {
                CreateKind::Create
            } else {
                CreateKind::Create2
            };
            return Ok(self.set_expected_create(state, bytecode, deployer, kind));
        }
        if selector == selector!("clearMockedCalls()") {
            state.call_mocks.clear();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("mockCall(address,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,uint256,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.mockCall")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], false));
        }
        if selector == selector!("mockCall(address,uint256,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.mockCall")?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 2);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                3,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCall",
            )?;
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], false));
        }
        if selector == selector!("mockCalls(address,bytes,bytes[])")
            || selector == selector!("mockCalls(address,uint256,bytes,bytes[])")
        {
            let has_value = selector == selector!("mockCalls(address,uint256,bytes,bytes[])");
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
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
            return Ok(self.add_call_mock(state, callee, value, data, returns, false));
        }
        if selector == selector!("mockCallRevert(address,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,uint256,bytes,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let value = read_abi_concrete_word_arg(
                &state.memory,
                args_offset,
                1,
                "symbolic vm.mockCallRevert",
            )?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
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
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,bytes4,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let data = read_abi_bytes4_words_arg(&state.memory, args_offset, 1);
            let ret = read_abi_dynamic_return_data_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockCallRevert",
            )?;
            return Ok(self.add_call_mock(state, callee, None, data, vec![ret], true));
        }
        if selector == selector!("mockCallRevert(address,uint256,bytes4,bytes)") {
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
            return Ok(self.add_call_mock(state, callee, Some(value), data, vec![ret], true));
        }
        if selector == selector!("mockFunction(address,address,bytes)") {
            let callee = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let target =
                read_abi_address_arg(&state.memory, args_offset, 1, "symbolic vm.mockFunction")?;
            let data = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                2,
                self.config.max_calldata_bytes as usize,
                "symbolic vm.mockFunction",
            )?;
            return Ok(self.set_function_mock(state, callee, target, data));
        }
        if selector == selector!("prank(address)") {
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,address)") {
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,bool)") {
            let delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.prank")?;
            if delegate_call {
                return Err(SymbolicError::Unsupported("symbolic vm.prank delegatecall"));
            }
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("prank(address,address,bool)") {
            let delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.prank")?;
            if delegate_call {
                return Err(SymbolicError::Unsupported("symbolic vm.prank delegatecall"));
            }
            state.prank.next_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.next_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address)") {
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,address)") {
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,bool)") {
            let delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 1, "symbolic vm.startPrank")?;
            if delegate_call {
                return Err(SymbolicError::Unsupported("symbolic vm.startPrank delegatecall"));
            }
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin = None;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("startPrank(address,address,bool)") {
            let delegate_call =
                read_abi_bool_arg(&state.memory, args_offset, 2, "symbolic vm.startPrank")?;
            if delegate_call {
                return Err(SymbolicError::Unsupported("symbolic vm.startPrank delegatecall"));
            }
            state.prank.persistent_caller =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 0)?);
            state.prank.persistent_origin =
                Some(read_abi_address_word_or_symbolic_slot_arg(state, args_offset, 1)?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("stopPrank()") {
            state.prank = SymbolicPrank::default();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("readCallers()") {
            return Ok(CheatcodeOutcome::Continue(state.read_callers_words()));
        }
        if selector == selector!("addr(uint256)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.addr")?;
            let address = private_key_address(private_key)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("sign(uint256,bytes32)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.sign")?;
            let digest = read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.sign")?;
            return Ok(CheatcodeOutcome::Continue(sign_hash_words(private_key, digest)?));
        }
        if selector == selector!("signCompact(uint256,bytes32)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.signCompact")?;
            let digest =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.signCompact")?;
            return Ok(CheatcodeOutcome::Continue(sign_compact_hash_words(private_key, digest)?));
        }
        if selector == selector!("deriveKey(string,uint32)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let private_key =
                derive_private_key::<English>(&mnemonic, DEFAULT_DERIVATION_PATH_PREFIX, index)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,string,uint32)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let path = read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key::<English>(&mnemonic, &path, index)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,uint32,string)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let language =
                read_abi_string_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key_with_language(
                &mnemonic,
                DEFAULT_DERIVATION_PATH_PREFIX,
                index,
                &language,
            )?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("deriveKey(string,string,uint32,string)") {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.deriveKey")?;
            let path = read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.deriveKey")?;
            let index = read_abi_u32_arg(&state.memory, args_offset, 2, "symbolic vm.deriveKey")?;
            let language =
                read_abi_string_arg(&state.memory, args_offset, 3, "symbolic vm.deriveKey")?;
            let private_key = derive_private_key_with_language(&mnemonic, &path, index, &language)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(private_key)]));
        }
        if selector == selector!("rememberKey(uint256)") {
            let private_key =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.rememberKey")?;
            let address = private_key_address(private_key)?;
            state.wallets.insert(address);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("rememberKeys(string,string,uint32)")
            || selector == selector!("rememberKeys(string,string,string,uint32)")
        {
            let mnemonic =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.rememberKeys")?;
            let path =
                read_abi_string_arg(&state.memory, args_offset, 1, "symbolic vm.rememberKeys")?;
            let (language, count_index) =
                if selector == selector!("rememberKeys(string,string,string,uint32)") {
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
        if selector == selector!("getWallets()") {
            let wallets = DynSolValue::Array(
                state.wallets.iter().copied().map(DynSolValue::Address).collect(),
            );
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(wallets)));
        }
        if selector == selector!("store(address,bytes32,bytes32)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let slot = state.memory.load_word(in_offset + 36)?;
            let value = state.memory.load_word(in_offset + 68)?;
            if target == CHEATCODE_ADDRESS
                && slot == SymWord::Concrete(failed_slot())
                && value == SymWord::Concrete(U256::from(1))
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            state.world.sstore(target, slot, value);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("load(address,bytes32)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let slot = state.memory.load_word(in_offset + 36)?;
            let concrete_slot = state.constrained_word(&slot);
            let value = state.world.sload(executor, target, slot, concrete_slot)?;
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("getNonce(address)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce = state.world.nonce(executor, target)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(nonce))]));
        }
        if selector == selector!("computeCreateAddress(address,uint256)") {
            let deployer = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let nonce = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let address = compute_create_address_word(state, deployer, nonce)?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("computeCreate2Address(bytes32,bytes32,address)") {
            let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let deployer = read_abi_word_arg(&state.memory, args_offset, 2)?;
            let address = compute_create2_address_word(state, deployer, salt, init_code_hash)?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("computeCreate2Address(bytes32,bytes32)") {
            let salt = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let init_code_hash = read_abi_word_arg(&state.memory, args_offset, 1)?;
            let address = compute_create2_address_word(
                state,
                SymWord::Concrete(address_word(DEFAULT_CREATE2_DEPLOYER)),
                salt,
                init_code_hash,
            )?;
            return Ok(CheatcodeOutcome::Continue(vec![address]));
        }
        if selector == selector!("etch(address,bytes)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let code = read_abi_symbolic_dynamic_bytes_arg(
                state,
                args_offset,
                1,
                self.config.max_dynamic_length as usize,
                "symbolic vm.etch",
            )?;
            state.world.install_code(target, SymCode::from_symbolic_bytes(code));
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getCode(string)")
            || selector == selector!("getDeployedCode(string)")
        {
            let artifact =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.getCode")?;
            let code = artifact_code(&artifact, selector == selector!("getDeployedCode(string)"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(code)));
        }
        if selector == selector!("deal(address,uint256)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let value = read_abi_word_arg(&state.memory, args_offset, 1)?;
            if value.contains_gasleft() {
                return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
            }
            let value = state.constrained_word(&value).map(SymWord::Concrete).unwrap_or(value);
            state.world.set_balance_word(target, value);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setNonce(address,uint64)")
            || selector == selector!("setNonceUnsafe(address,uint64)")
        {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce =
                read_abi_constrained_word_arg(state, args_offset, 1, "symbolic vm.setNonce")?;
            if nonce > U256::from(u64::MAX) {
                return Err(SymbolicError::Unsupported("symbolic vm.setNonce nonce"));
            }
            let nonce = nonce.to::<u64>();
            if selector == selector!("setNonce(address,uint64)")
                && nonce < state.world.nonce(executor, target)?
            {
                return Ok(CheatcodeOutcome::Failure);
            }
            state.world.set_nonce(target, nonce);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("resetNonce(address)") {
            let target = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let nonce = if state.world.extcode(executor, target)?.is_empty() { 0 } else { 1 };
            state.world.set_nonce(target, nonce);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("allowCheatcodes(address)") {
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            state.persistent_accounts.insert(account);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address,address)") {
            let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
            state.persistent_accounts.insert(account0);
            state.persistent_accounts.insert(account1);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address,address,address)") {
            let account0 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            let account1 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 1)?;
            let account2 = read_abi_address_or_symbolic_slot_arg(state, args_offset, 2)?;
            state.persistent_accounts.insert(account0);
            state.persistent_accounts.insert(account1);
            state.persistent_accounts.insert(account2);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("makePersistent(address[])") {
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
        if selector == selector!("revokePersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            state.persistent_accounts.remove(&account);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("revokePersistent(address[])") {
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
        if selector == selector!("isPersistent(address)") {
            let account = read_abi_address_or_symbolic_slot_arg(state, args_offset, 0)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                state.persistent_accounts.contains(&account),
            ))]));
        }
        if selector == selector!("activeFork()") {
            let id = executor.backend().active_fork_id().ok_or(SymbolicError::Unsupported(
                "symbolic vm.activeFork requires an active forked executor",
            ))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("selectFork(uint256)") {
            let id =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.selectFork id")?;
            if executor.backend().is_active_fork(id) {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.selectFork can only select the already active fork",
            ));
        }
        if selector == selector!("rollFork(uint256)") {
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.rollFork block number",
            )?;
            let current =
                state.block.number.clone().into_concrete("symbolic vm.rollFork current block")?;
            if block_number == current {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
            ));
        }
        if selector == selector!("rollFork(uint256,uint256)") {
            let id =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic vm.rollFork id")?;
            let block_number = read_abi_constrained_word_arg(
                state,
                args_offset,
                1,
                "symbolic vm.rollFork block number",
            )?;
            let current =
                state.block.number.clone().into_concrete("symbolic vm.rollFork current block")?;
            if executor.backend().is_active_fork(id) && block_number == current {
                return Ok(CheatcodeOutcome::Continue(Vec::new()));
            }
            return Err(SymbolicError::Unsupported(
                "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
            ));
        }
        if selector == selector!("createFork(string)")
            || selector == selector!("createFork(string,uint256)")
            || selector == selector!("createFork(string,bytes32)")
            || selector == selector!("createSelectFork(string)")
            || selector == selector!("createSelectFork(string,uint256)")
            || selector == selector!("createSelectFork(string,bytes32)")
            || selector == selector!("rollFork(bytes32)")
            || selector == selector!("rollFork(uint256,bytes32)")
        {
            return Err(SymbolicError::Unsupported(
                "symbolic fork creation and fork block mutation must happen before symbolic execution",
            ));
        }
        if selector == selector!("snapshot()") || selector == selector!("snapshotState()") {
            let id = state.world.snapshot_state();
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("revertTo(uint256)")
            || selector == selector!("revertToState(uint256)")
            || selector == selector!("revertToAndDelete(uint256)")
            || selector == selector!("revertToStateAndDelete(uint256)")
        {
            let id = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.revertToState snapshot",
            )?;
            let success = state.world.restore_snapshot(id);
            if success
                && (selector == selector!("revertToAndDelete(uint256)")
                    || selector == selector!("revertToStateAndDelete(uint256)"))
            {
                state.world.delete_snapshot(id);
            }
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(success))]));
        }
        if selector == selector!("deleteSnapshot(uint256)")
            || selector == selector!("deleteStateSnapshot(uint256)")
        {
            let id = read_abi_constrained_word_arg(
                state,
                args_offset,
                0,
                "symbolic vm.deleteStateSnapshot snapshot",
            )?;
            let success = state.world.delete_snapshot(id);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(success))]));
        }
        if selector == selector!("deleteSnapshots()")
            || selector == selector!("deleteStateSnapshots()")
        {
            state.world.delete_snapshots();
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("warp(uint256)") {
            state.block.timestamp = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("roll(uint256)") {
            state.block.number = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setBlockhash(uint256,bytes32)") {
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
        if selector == selector!("prevrandao(bytes32)")
            || selector == selector!("prevrandao(uint256)")
        {
            state.block.difficulty = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("blobhashes(bytes32[])") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::Array(Box::new(DynSolType::FixedBytes(32)))],
            )?;
            state.block.set_blob_hashes(dyn_bytes32_array(&values[0])?);
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlobhashes()") {
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
        if selector == selector!("fee(uint256)") {
            state.block.basefee = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("blobBaseFee(uint256)") {
            state.block.blob_basefee = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlobBaseFee()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.blob_basefee.clone()]));
        }
        if selector == selector!("chainId(uint256)") {
            state.block.chain_id = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getChainId()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.chain_id.clone()]));
        }
        if selector == selector!("difficulty(uint256)") {
            state.block.difficulty = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("coinbase(address)") {
            let coinbase = read_abi_constrained_address_arg(
                state,
                args_offset,
                0,
                "symbolic vm.coinbase value",
            )?;
            state.block.coinbase = coinbase;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlockNumber()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.number.clone()]));
        }
        if selector == selector!("txGasPrice(uint256)") {
            state.gas_price = state.memory.load_word(in_offset + 4)?;
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("getBlockTimestamp()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.block.timestamp.clone()]));
        }
        if selector == selector!("label(address,string)") {
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
        if selector == selector!("getLabel(address)") {
            let account =
                read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.getLabel")?;
            let label = state
                .labels
                .get(&account)
                .cloned()
                .unwrap_or_else(|| format!("unlabeled:{account}"));
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(label.bytes())));
        }
        if selector == selector!("expectSafeMemory(uint64,uint64)") {
            return Err(SymbolicError::Unsupported("symbolic vm.expectSafeMemory not modeled"));
        }
        if selector == selector!("expectSafeMemoryCall(uint64,uint64)") {
            return Err(SymbolicError::Unsupported("symbolic vm.expectSafeMemoryCall not modeled"));
        }
        if selector == selector!("stopExpectSafeMemory()") {
            return Err(SymbolicError::Unsupported("symbolic vm.stopExpectSafeMemory not modeled"));
        }
        if selector == selector!("lastCallGas()") {
            return Err(SymbolicError::Unsupported("symbolic vm.lastCallGas not modeled"));
        }
        if selector == selector!("snapshotGasLastCall(string)")
            || selector == selector!("snapshotGasLastCall(string,string)")
        {
            return Err(SymbolicError::Unsupported("symbolic vm.snapshotGasLastCall not modeled"));
        }
        if selector == selector!("stopSnapshotGas()")
            || selector == selector!("stopSnapshotGas(string)")
            || selector == selector!("stopSnapshotGas(string,string)")
        {
            return Err(SymbolicError::Unsupported("symbolic vm.stopSnapshotGas not modeled"));
        }
        if selector == selector!("pauseGasMetering()")
            || selector == selector!("resumeGasMetering()")
            || selector == selector!("resetGasMetering()")
            || selector == selector!("breakpoint(string)")
            || selector == selector!("breakpoint(string,bool)")
            || selector == selector!("snapshotValue(string,uint256)")
            || selector == selector!("snapshotValue(string,string,uint256)")
            || selector == selector!("startSnapshotGas(string)")
            || selector == selector!("startSnapshotGas(string,string)")
            || selector == selector!("sleep(uint256)")
            || selector == selector!("cool(address)")
            || selector == selector!("accessList((address,bytes32[])[])")
            || selector == selector!("warmSlot(address,bytes32)")
            || selector == selector!("coolSlot(address,bytes32)")
            || selector == selector!("noAccessList()")
        {
            return Ok(CheatcodeOutcome::Continue(Vec::new()));
        }
        if selector == selector!("setEvmVersion(string)") {
            return Err(SymbolicError::Unsupported("symbolic vm.setEvmVersion not modeled"));
        }
        if selector == selector!("getEvmVersion()") {
            return Err(SymbolicError::Unsupported("symbolic vm.getEvmVersion not modeled"));
        }
        if selector == selector!("getFoundryVersion()") {
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                env!("CARGO_PKG_VERSION").bytes(),
            )));
        }
        if selector == selector!("projectRoot()") {
            let root = std::env::current_dir()
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.projectRoot"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                root.display().to_string().bytes(),
            )));
        }
        if selector == selector!("unixTime()") {
            let milliseconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?
                .as_millis();
            let value = U256::try_from(milliseconds)
                .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("isContext(uint8)") {
            let context =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.isContext")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                context == U256::ZERO || context == U256::from(1),
            ))]));
        }
        if selector == selector!("toString(address)") {
            let address =
                read_abi_address_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("{address:?}").bytes(),
            )));
        }
        if selector == selector!("toString(bytes)") {
            let bytes =
                read_abi_dynamic_bytes_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("0x{}", hex::encode(bytes)).bytes(),
            )));
        }
        if selector == selector!("toString(bytes32)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                format!("0x{}", hex::encode(value.to_be_bytes::<32>())).bytes(),
            )));
        }
        if selector == selector!("toString(bool)") {
            let value = read_abi_bool_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                if value { "true" } else { "false" }.bytes(),
            )));
        }
        if selector == selector!("toString(uint256)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                value.to_string().bytes(),
            )));
        }
        if selector == selector!("toString(int256)") {
            let value =
                read_abi_concrete_word_arg(&state.memory, args_offset, 0, "symbolic vm.toString")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                I256::from_raw(value).to_string().bytes(),
            )));
        }
        if selector == selector!("parseBytes(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes")?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(parse_env_bytes(
                &value,
            )?)));
        }
        if selector == selector!("parseAddress(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseAddress")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(
                parse_env_address(&value)?,
            ))]));
        }
        if selector == selector!("parseUint(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseUint")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_uint(
                &value,
            )?)]));
        }
        if selector == selector!("parseInt(string)") {
            let value = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseInt")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_int(&value)?)]));
        }
        if selector == selector!("parseBytes32(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBytes32")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_bytes32(
                &value,
            )?)]));
        }
        if selector == selector!("parseBool(string)") {
            let value =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.parseBool")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                parse_env_bool(&value)?,
            ))]));
        }
        if selector == selector!("toLowercase(string)")
            || selector == selector!("toUppercase(string)")
            || selector == selector!("trim(string)")
        {
            let value = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.string")?;
            let output = if selector == selector!("toLowercase(string)") {
                value.to_lowercase()
            } else if selector == selector!("toUppercase(string)") {
                value.to_uppercase()
            } else {
                value.trim().to_string()
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(output.bytes())));
        }
        if selector == selector!("replace(string,string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String, DynSolType::String],
            )?;
            let output =
                dyn_string(&values[0])?.replace(&dyn_string(&values[1])?, &dyn_string(&values[2])?);
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(output.bytes())));
        }
        if selector == selector!("split(string,string)") {
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
                input.split(&delimiter).map(|part| DynSolValue::String(part.to_string())).collect()
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                DynSolValue::Array(parts),
            )));
        }
        if selector == selector!("indexOf(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let input = dyn_string(&values[0])?;
            let needle = dyn_string(&values[1])?;
            let index = input.find(&needle).map(U256::from).unwrap_or(U256::MAX);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(index)]));
        }
        if selector == selector!("contains(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let contains = dyn_string(&values[0])?.contains(&dyn_string(&values[1])?);
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(contains))]));
        }
        if selector == selector!("toBase64(bytes)")
            || selector == selector!("toBase64(string)")
            || selector == selector!("toBase64URL(bytes)")
            || selector == selector!("toBase64URL(string)")
        {
            let data =
                read_abi_dynamic_bytes_arg(&state.memory, args_offset, 0, "symbolic vm.toBase64")?;
            let encoded = if selector == selector!("toBase64URL(bytes)")
                || selector == selector!("toBase64URL(string)")
            {
                BASE64_URL_SAFE.encode(data)
            } else {
                BASE64_STANDARD.encode(data)
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(encoded.bytes())));
        }
        if selector == selector!("bound(uint256,uint256,uint256)") {
            return self.handle_bound_uint(state, args_offset);
        }
        if selector == selector!("bound(int256,int256,int256)") {
            return self.handle_bound_int(state, args_offset);
        }
        if selector == selector!("envExists(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envExists")?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                std::env::var_os(name).is_some(),
            ))]));
        }
        if selector == selector!("envBool(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBool")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(U256::from(
                parse_env_bool(&value)?,
            ))]));
        }
        if selector == selector!("envUint(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envUint")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_uint(
                &value,
            )?)]));
        }
        if selector == selector!("envInt(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envInt")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_int(&value)?)]));
        }
        if selector == selector!("envAddress(string)") {
            let name =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envAddress")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            let address = parse_env_address(&value)?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(address_word(address))]));
        }
        if selector == selector!("envBytes32(string)") {
            let name =
                read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes32")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(parse_env_bytes32(
                &value,
            )?)]));
        }
        if selector == selector!("envString(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envString")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value.bytes())));
        }
        if selector == selector!("envBytes(string)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envBytes")?;
            let value = std::env::var(name)
                .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(parse_env_bytes(
                &value,
            )?)));
        }
        if selector == selector!("envBool(string,string)")
            || selector == selector!("envUint(string,string)")
            || selector == selector!("envInt(string,string)")
            || selector == selector!("envAddress(string,string)")
            || selector == selector!("envBytes32(string,string)")
            || selector == selector!("envString(string,string)")
            || selector == selector!("envBytes(string,string)")
        {
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
            let value = if selector == selector!("envBool(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_bool_value)?
            } else if selector == selector!("envUint(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_uint_value)?
            } else if selector == selector!("envInt(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_int_value)?
            } else if selector == selector!("envAddress(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_address_value)?
            } else if selector == selector!("envBytes32(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
            } else if selector == selector!("envString(string,string)") {
                parse_env_array(&value, &delimiter, parse_env_string_value)?
            } else {
                parse_env_array(&value, &delimiter, parse_env_bytes_value)?
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("envOr(string,bool)") {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
            let value = match std::env::var(name) {
                Ok(value) => U256::from(parse_env_bool(&value)?),
                Err(_) => {
                    read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?
                }
            };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("envOr(string,uint256)")
            || selector == selector!("envOr(string,int256)")
            || selector == selector!("envOr(string,address)")
            || selector == selector!("envOr(string,bytes32)")
        {
            let name = read_abi_string_arg(&state.memory, args_offset, 0, "symbolic vm.envOr")?;
            let default =
                read_abi_concrete_word_arg(&state.memory, args_offset, 1, "symbolic vm.envOr")?;
            let value = match std::env::var(name) {
                Ok(value) if selector == selector!("envOr(string,uint256)") => {
                    parse_env_uint(&value)?
                }
                Ok(value) if selector == selector!("envOr(string,int256)") => {
                    parse_env_int(&value)?
                }
                Ok(value) if selector == selector!("envOr(string,address)") => {
                    address_word(parse_env_address(&value)?)
                }
                Ok(value) => parse_env_bytes32(&value)?,
                Err(_) => default,
            };
            return Ok(CheatcodeOutcome::Continue(vec![SymWord::Concrete(value)]));
        }
        if selector == selector!("envOr(string,string)") {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                vec![DynSolType::String, DynSolType::String],
            )?;
            let name = dyn_string(&values[0])?;
            let value = std::env::var(name).unwrap_or(dyn_string(&values[1])?);
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value.bytes())));
        }
        if selector == selector!("envOr(string,bytes)") {
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
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(value)));
        }
        if selector == selector!("envOr(string,string,bool[])")
            || selector == selector!("envOr(string,string,uint256[])")
            || selector == selector!("envOr(string,string,int256[])")
            || selector == selector!("envOr(string,string,address[])")
            || selector == selector!("envOr(string,string,bytes32[])")
            || selector == selector!("envOr(string,string,string[])")
            || selector == selector!("envOr(string,string,bytes[])")
        {
            let element_ty = if selector == selector!("envOr(string,string,bool[])") {
                DynSolType::Bool
            } else if selector == selector!("envOr(string,string,uint256[])") {
                DynSolType::Uint(256)
            } else if selector == selector!("envOr(string,string,int256[])") {
                DynSolType::Int(256)
            } else if selector == selector!("envOr(string,string,address[])") {
                DynSolType::Address
            } else if selector == selector!("envOr(string,string,bytes32[])") {
                DynSolType::FixedBytes(32)
            } else if selector == selector!("envOr(string,string,string[])") {
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
                Ok(value) if selector == selector!("envOr(string,string,bool[])") => {
                    parse_env_array(&value, &delimiter, parse_env_bool_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,uint256[])") => {
                    parse_env_array(&value, &delimiter, parse_env_uint_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,int256[])") => {
                    parse_env_array(&value, &delimiter, parse_env_int_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,address[])") => {
                    parse_env_array(&value, &delimiter, parse_env_address_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,bytes32[])") => {
                    parse_env_array(&value, &delimiter, parse_env_bytes32_value)?
                }
                Ok(value) if selector == selector!("envOr(string,string,string[])") => {
                    parse_env_array(&value, &delimiter, parse_env_string_value)?
                }
                Ok(value) => parse_env_array(&value, &delimiter, parse_env_bytes_value)?,
                Err(_) => values[2].clone(),
            };
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(value)));
        }
        if selector == selector!("ffi(string[])") {
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
            return Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(bytes)));
        }
        if selector == selector!("assertTrue(bool)")
            || selector == selector!("assertTrue(bool,string)")
        {
            let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.nonzero_bool();
            return self.handle_assertion(state, condition);
        }
        if selector == selector!("assertFalse(bool)")
            || selector == selector!("assertFalse(bool,string)")
        {
            let condition = read_abi_word_arg(&state.memory, args_offset, 0)?.into_zero_bool();
            return self.handle_assertion(state, condition);
        }
        if selector == selector!("assertEq(uint256,uint256)")
            || selector == selector!("assertEq(uint256,uint256,string)")
            || selector == selector!("assertEq(int256,int256)")
            || selector == selector!("assertEq(int256,int256,string)")
            || selector == selector!("assertEq(address,address)")
            || selector == selector!("assertEq(address,address,string)")
            || selector == selector!("assertEq(bytes32,bytes32)")
            || selector == selector!("assertEq(bytes32,bytes32,string)")
            || selector == selector!("assertEq(bool,bool)")
            || selector == selector!("assertEq(bool,bool,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()));
        }
        if selector == selector!("assertEq(string,string)")
            || selector == selector!("assertEq(string,string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertEq(string,string)") {
                    vec![DynSolType::String, DynSolType::String]
                } else {
                    vec![DynSolType::String, DynSolType::String, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_string(&values[0])? == dyn_string(&values[1])?),
            );
        }
        if selector == selector!("assertEq(bytes,bytes)")
            || selector == selector!("assertEq(bytes,bytes,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertEq(bytes,bytes)") {
                    vec![DynSolType::Bytes, DynSolType::Bytes]
                } else {
                    vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_bytes(&values[0])? == dyn_bytes(&values[1])?),
            );
        }
        if selector == selector!("assertEq(bool[],bool[])")
            || selector == selector!("assertEq(bool[],bool[],string)")
            || selector == selector!("assertEq(uint256[],uint256[])")
            || selector == selector!("assertEq(uint256[],uint256[],string)")
            || selector == selector!("assertEq(int256[],int256[])")
            || selector == selector!("assertEq(int256[],int256[],string)")
            || selector == selector!("assertEq(address[],address[])")
            || selector == selector!("assertEq(address[],address[],string)")
            || selector == selector!("assertEq(bytes32[],bytes32[])")
            || selector == selector!("assertEq(bytes32[],bytes32[],string)")
            || selector == selector!("assertEq(string[],string[])")
            || selector == selector!("assertEq(string[],string[],string)")
            || selector == selector!("assertEq(bytes[],bytes[])")
            || selector == selector!("assertEq(bytes[],bytes[],string)")
        {
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
            return self.handle_assertion(state, BoolExpr::Const(values[0] == values[1]));
        }
        if selector == selector!("assertEqDecimal(uint256,uint256,uint256)")
            || selector == selector!("assertEqDecimal(uint256,uint256,uint256,string)")
            || selector == selector!("assertEqDecimal(int256,int256,uint256)")
            || selector == selector!("assertEqDecimal(int256,int256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()));
        }
        if selector == selector!("assertNotEq(uint256,uint256)")
            || selector == selector!("assertNotEq(uint256,uint256,string)")
            || selector == selector!("assertNotEq(int256,int256)")
            || selector == selector!("assertNotEq(int256,int256,string)")
            || selector == selector!("assertNotEq(address,address)")
            || selector == selector!("assertNotEq(address,address,string)")
            || selector == selector!("assertNotEq(bytes32,bytes32)")
            || selector == selector!("assertNotEq(bytes32,bytes32,string)")
            || selector == selector!("assertNotEq(bool,bool)")
            || selector == selector!("assertNotEq(bool,bool,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self
                .handle_assertion(state, BoolExpr::eq(left.into_expr(), right.into_expr()).not());
        }
        if selector == selector!("assertNotEq(string,string)")
            || selector == selector!("assertNotEq(string,string,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertNotEq(string,string)") {
                    vec![DynSolType::String, DynSolType::String]
                } else {
                    vec![DynSolType::String, DynSolType::String, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_string(&values[0])? != dyn_string(&values[1])?),
            );
        }
        if selector == selector!("assertNotEq(bytes,bytes)")
            || selector == selector!("assertNotEq(bytes,bytes,string)")
        {
            let values = decode_cheatcode_args(
                state,
                in_offset,
                in_size,
                if selector == selector!("assertNotEq(bytes,bytes)") {
                    vec![DynSolType::Bytes, DynSolType::Bytes]
                } else {
                    vec![DynSolType::Bytes, DynSolType::Bytes, DynSolType::String]
                },
            )?;
            return self.handle_assertion(
                state,
                BoolExpr::Const(dyn_bytes(&values[0])? != dyn_bytes(&values[1])?),
            );
        }
        if selector == selector!("assertNotEq(bool[],bool[])")
            || selector == selector!("assertNotEq(bool[],bool[],string)")
            || selector == selector!("assertNotEq(uint256[],uint256[])")
            || selector == selector!("assertNotEq(uint256[],uint256[],string)")
            || selector == selector!("assertNotEq(int256[],int256[])")
            || selector == selector!("assertNotEq(int256[],int256[],string)")
            || selector == selector!("assertNotEq(address[],address[])")
            || selector == selector!("assertNotEq(address[],address[],string)")
            || selector == selector!("assertNotEq(bytes32[],bytes32[])")
            || selector == selector!("assertNotEq(bytes32[],bytes32[],string)")
            || selector == selector!("assertNotEq(string[],string[])")
            || selector == selector!("assertNotEq(string[],string[],string)")
            || selector == selector!("assertNotEq(bytes[],bytes[])")
            || selector == selector!("assertNotEq(bytes[],bytes[],string)")
        {
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
            return self.handle_assertion(state, BoolExpr::Const(values[0] != values[1]));
        }
        if selector == selector!("assertLt(uint256,uint256)")
            || selector == selector!("assertLt(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ult, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLe(uint256,uint256)")
            || selector == selector!("assertLe(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ule, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGt(uint256,uint256)")
            || selector == selector!("assertGt(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Ugt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGe(uint256,uint256)")
            || selector == selector!("assertGe(uint256,uint256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Uge, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLt(int256,int256)")
            || selector == selector!("assertLt(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Slt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertGt(int256,int256)")
            || selector == selector!("assertGt(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Sgt, left.into_expr(), right.into_expr()),
            );
        }
        if selector == selector!("assertLe(int256,int256)")
            || selector == selector!("assertLe(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Sgt, left.into_expr(), right.into_expr()).not(),
            );
        }
        if selector == selector!("assertGe(int256,int256)")
            || selector == selector!("assertGe(int256,int256,string)")
        {
            let left = read_abi_word_arg(&state.memory, args_offset, 0)?;
            let right = read_abi_word_arg(&state.memory, args_offset, 1)?;
            return self.handle_assertion(
                state,
                BoolExpr::cmp(BoolExprOp::Slt, left.into_expr(), right.into_expr()).not(),
            );
        }
        if selector == selector!("randomUint()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomUint")]));
        }
        if selector == selector!("randomUint(uint256)") {
            let bits =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic randomUint bits")?;
            Self::validate_symbolic_integer_bits(bits, "symbolic randomUint bits")?;
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("randomUint(uint256,uint256)") {
            let min = state.memory.load_word(in_offset + 4)?;
            let max = state.memory.load_word(in_offset + 36)?;
            let value = state.fresh_word("vmRandomUintRange");
            state.constraints.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                value.clone().into_expr(),
                min.into_expr(),
            ));
            state.constraints.push(BoolExpr::cmp(
                BoolExprOp::Ule,
                value.clone().into_expr(),
                max.into_expr(),
            ));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomInt()") {
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_word("vmRandomInt")]));
        }
        if selector == selector!("randomInt(uint256)") {
            let bits =
                read_abi_constrained_word_arg(state, args_offset, 0, "symbolic randomInt bits")?;
            Self::validate_symbolic_integer_bits(bits, "symbolic randomInt bits")?;
            return Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_int(bits)]));
        }
        if selector == selector!("randomAddress()") {
            let value = state.fresh_bounded_uint(U256::from(160));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomBool()") {
            let value = state.fresh_bounded_uint(U256::from(1));
            return Ok(CheatcodeOutcome::Continue(vec![value]));
        }
        if selector == selector!("randomBytes(uint256)") {
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
            let bytes = (0..max_len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(CheatcodeOutcome::ContinueData(abi_bytes_return_with_len(len, bytes)));
        }
        if selector == selector!("randomBytes4()") {
            let value = state.fresh_bounded_uint(U256::from(32));
            return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 224)]));
        }
        if selector == selector!("randomBytes8()") {
            let value = state.fresh_bounded_uint(U256::from(64));
            return Ok(CheatcodeOutcome::Continue(vec![shift_left(value, 192)]));
        }

        Err(SymbolicError::Unsupported("symbolic Foundry cheatcode"))
    }

    /// Runs the `handle_symbolic_vm_cheatcode` symbolic executor helper.
    pub(super) fn handle_symbolic_vm_cheatcode(
        &mut self,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        if selector == selector!("createUint256(string)")
            || selector == selector!("createInt256(string)")
            || selector == selector!("createBytes32(string)")
        {
            return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
        }
        for bits in (8..=256).step_by(8) {
            if selector == selector_for(&format!("createUint{bits}(string)")) {
                if bits == 256 {
                    return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
                }
                return Ok(SymReturnData::from_words(vec![
                    state.fresh_bounded_uint(U256::from(bits)),
                ]));
            }
            if selector == selector_for(&format!("createInt{bits}(string)")) {
                if bits == 256 {
                    return Ok(SymReturnData::from_words(vec![state.fresh_word("svm")]));
                }
                return Ok(SymReturnData::from_words(vec![
                    state.fresh_bounded_int(U256::from(bits)),
                ]));
            }
        }
        for bytes in 1..=32 {
            if selector == selector_for(&format!("createBytes{bytes}(string)")) {
                let value = state.fresh_bounded_uint(U256::from(bytes * 8));
                let value = if bytes == 32 { value } else { shift_left(value, (32 - bytes) * 8) };
                return Ok(SymReturnData::from_words(vec![value]));
            }
        }
        if selector == selector!("createUint(uint256,string)") {
            let bits = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.create integer bits",
            )?;
            Self::validate_symbolic_integer_bits(bits, "symbolic svm.create integer bits")?;
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(bits)]));
        }
        if selector == selector!("createInt(uint256,string)") {
            let bits = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.create integer bits",
            )?;
            Self::validate_symbolic_integer_bits(bits, "symbolic svm.create integer bits")?;
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_int(bits)]));
        }
        if selector == selector!("createAddress(string)") {
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(160))]));
        }
        if selector == selector!("createBool(string)") {
            return Ok(SymReturnData::from_words(vec![state.fresh_bounded_uint(U256::from(1))]));
        }
        if selector == selector!("createBytes(string)") {
            let len = self.config.default_dynamic_length as usize;
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createBytes(uint256,string)") {
            let len = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.createBytes length",
            )?;
            let len = u256_to_usize(len)
                .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                .ok_or(SymbolicError::Unsupported("symbolic svm.createBytes length"))?;
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createString(string)") {
            let len = self.config.default_dynamic_length as usize;
            let bytes = (0..len)
                .map(|_| {
                    let byte = state.fresh_bounded_uint(U256::from(8));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Uge,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x20)),
                    ));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Ule,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x7e)),
                    ));
                    byte
                })
                .collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createString(uint256,string)") {
            let len = read_abi_constrained_word_arg(
                state,
                in_offset + 4,
                0,
                "symbolic svm.createString length",
            )?;
            let len = u256_to_usize(len)
                .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                .ok_or(SymbolicError::Unsupported("symbolic svm.createString length"))?;
            let bytes = (0..len)
                .map(|_| {
                    let byte = state.fresh_bounded_uint(U256::from(8));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Uge,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x20)),
                    ));
                    state.constraints.push(BoolExpr::cmp(
                        BoolExprOp::Ule,
                        byte.clone().into_expr(),
                        Expr::Const(U256::from(0x7e)),
                    ));
                    byte
                })
                .collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("createBytes4(string)") {
            return Ok(SymReturnData::from_words(vec![shift_left(
                state.fresh_bounded_uint(U256::from(32)),
                224,
            )]));
        }
        if selector == selector!("createCalldata(string)") {
            let max = self.config.max_calldata_bytes as usize;
            let len = if max < 4 {
                max
            } else {
                (self.config.default_dynamic_length as usize).max(4).min(max)
            };
            let bytes = (0..len).map(|_| state.fresh_bounded_uint(U256::from(8))).collect();
            return Ok(abi_bytes_return(bytes));
        }
        if selector == selector!("enableSymbolicStorage(address)")
            || selector == selector!("setArbitraryStorage(address)")
        {
            let target = read_abi_address_or_symbolic_slot_arg(state, in_offset + 4, 0)?;
            state.world.enable_arbitrary_storage(target);
            return Ok(SymReturnData::default());
        }
        if selector == selector!("snapshotStorage(address)") {
            let _target = read_abi_address_or_symbolic_slot_arg(state, in_offset + 4, 0)?;
            let id = state.world.snapshot_state();
            return Ok(SymReturnData::from_words(vec![SymWord::Concrete(id)]));
        }
        if selector == selector!("snapshotState()") {
            let id = state.world.snapshot_state();
            return Ok(SymReturnData::from_words(vec![SymWord::Concrete(id)]));
        }

        Err(SymbolicError::Unsupported("symbolic VM compatibility cheatcode"))
    }
}
