use foundry_cheatcodes_spec::Vm::*;
use foundry_common::version::SEMVER_VERSION;
use foundry_evm::{
    core::{backend::GLOBAL_FAIL_SLOT, decode::ASSERTION_FAILED_PREFIX},
    inspectors::cheatcodes::current_execution_context,
};

use super::*;

impl SymbolicExecutor {
    pub(super) fn handle_assertion(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        pass: SymBoolExpr,
        selector: [u8; 4],
        args_offset: Option<usize>,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let fail = pass.clone().not(&mut self.cx);
        match fail.as_const() {
            Some(true) => {
                return Ok(match self.fail_assertion(executor, state, selector, args_offset) {
                    Some(revert) => CheatcodeOutcome::Revert(revert),
                    None => CheatcodeOutcome::Continue(Vec::new()),
                });
            }
            Some(false) => return Ok(CheatcodeOutcome::Continue(Vec::new())),
            None => {}
        }

        let mut fail_constraints = state.constraints.clone();
        fail_constraints.push(fail);
        if self.is_sat_with_state(state, &fail_constraints)? {
            // A caller can catch the failure, so the passing branch continues too. Its
            // feasibility is checked when it is popped from the worklist.
            let mut passing = state.clone();
            passing.constraints.push(pass);
            passing.defer_feasibility_check();
            state.constraints = fail_constraints;
            let revert = self.fail_assertion(executor, state, selector, args_offset);
            return Ok(CheatcodeOutcome::Fork { passing: Box::new(passing), revert });
        }

        state.constraints.push(pass);
        Ok(CheatcodeOutcome::Continue(Vec::new()))
    }

    /// Like Forge, a failed assertion reverts the cheatcode call with the returned data, or sets
    /// the global fail slot when `assertions_revert` is off. The test checks that slot when it
    /// ends.
    fn fail_assertion(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        selector: [u8; 4],
        args_offset: Option<usize>,
    ) -> Option<SymReturnData> {
        state.assertion_failed = true;
        if executor.assertions_revert {
            let message = match selector {
                assertTrue_0Call::SELECTOR | assertFalse_0Call::SELECTOR => {
                    Some(ASSERTION_FAILED_PREFIX.to_owned())
                }
                assertTrue_1Call::SELECTOR | assertFalse_1Call::SELECTOR => {
                    args_offset.and_then(|offset| {
                        read_abi_string_arg(
                            &mut self.cx,
                            &state.memory,
                            offset,
                            1,
                            "symbolic assertion message",
                        )
                        .ok()
                    })
                }
                _ => None,
            };
            let message = if let Some(message) = message {
                message
            } else {
                self.defer_incomplete("symbolic assertion revert payload");
                ASSERTION_FAILED_PREFIX.to_owned()
            };
            return Some(error_string_return_data(&mut self.cx, &message));
        }
        let failed_slot = SymExpr::constant(&mut self.cx, GLOBAL_FAIL_SLOT);
        let one = SymExpr::one(&mut self.cx);
        state.world.sstore(CHEATCODE_ADDRESS, failed_slot, one);
        None
    }

    /// Returns whether the path can end with the global fail slot set, keeping that case.
    pub(super) fn global_fail_slot_set(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
    ) -> Result<bool, SymbolicError> {
        let failed = state.world.global_fail_slot(&mut self.cx, executor)?;
        let failed = failed.nonzero_bool(&mut self.cx);
        let (constraints, sat) = self.constraints_with_condition(state, failed)?;
        if sat {
            state.constraints = constraints;
        }
        Ok(sat)
    }

    pub(super) fn handle_full_word_array_assertion(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        selector: [u8; 4],
        input_offset: &SymExpr,
        input_size: &SymExpr,
        maximum_input_size: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        const HEAD_SIZE: usize = 4 + 2 * 32;
        if maximum_input_size < HEAD_SIZE {
            return Err(SymbolicError::Unsupported("short symbolic array assertion CALL"));
        }

        let minimum_offset = state.lower_bound_usize(input_offset);
        let maximum_offset = state.upper_bound_usize(&mut self.cx, input_offset);
        let mut required_size = HEAD_SIZE;
        let mut layouts = [(0usize, 0usize); 2];

        for (index, layout) in layouts.iter_mut().enumerate() {
            let head_offset = 4 + index * 32;
            let offset = state.memory.load_word_offset_with_bounds(
                &mut self.cx,
                input_offset,
                head_offset,
                minimum_offset,
                maximum_offset,
            );
            let offset = self
                .constrained_word_with_solver(state, &offset)?
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or(SymbolicError::Unsupported("symbolic array assertion offset"))?;

            let length_offset = 4usize
                .checked_add(offset)
                .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?;
            let elements_offset = length_offset
                .checked_add(32)
                .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?;
            if elements_offset > maximum_input_size {
                return Err(SymbolicError::Unsupported("short symbolic array assertion CALL"));
            }
            let length = state.memory.load_word_offset_with_bounds(
                &mut self.cx,
                input_offset,
                length_offset,
                minimum_offset,
                maximum_offset,
            );
            let length = self
                .constrained_word_with_solver(state, &length)?
                .and_then(|length| usize::try_from(length).ok())
                .ok_or(SymbolicError::Unsupported("symbolic array assertion length"))?;
            let byte_length = length
                .checked_mul(32)
                .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?;
            let end = elements_offset
                .checked_add(byte_length)
                .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?;
            if end > maximum_input_size {
                return Err(SymbolicError::Unsupported("short symbolic array assertion CALL"));
            }
            required_size = required_size.max(end);
            *layout = (elements_offset, length);
        }

        if !self.proves_expr_at_least(state, input_size, required_size)? {
            return Err(SymbolicError::Unsupported("symbolic array assertion CALL input size"));
        }

        let [(left_offset, left_len), (right_offset, right_len)] = layouts;
        let mut condition = if left_len == right_len {
            let mut equal = Vec::with_capacity(left_len);
            for element in 0..left_len {
                let offset = element
                    .checked_mul(32)
                    .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?;
                let left = state.memory.load_word_offset_with_bounds(
                    &mut self.cx,
                    input_offset,
                    left_offset
                        .checked_add(offset)
                        .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?,
                    minimum_offset,
                    maximum_offset,
                );
                let right = state.memory.load_word_offset_with_bounds(
                    &mut self.cx,
                    input_offset,
                    right_offset
                        .checked_add(offset)
                        .ok_or(SymbolicError::Unsupported("symbolic array assertion ABI decode"))?,
                    minimum_offset,
                    maximum_offset,
                );
                equal.push(SymBoolExpr::eq(&mut self.cx, left, right));
            }
            SymBoolExpr::and(&mut self.cx, equal)
        } else {
            SymBoolExpr::constant(&mut self.cx, false)
        };
        if matches!(
            selector,
            assertNotEq_16Call::SELECTOR
                | assertNotEq_18Call::SELECTOR
                | assertNotEq_22Call::SELECTOR
        ) {
            condition = condition.not(&mut self.cx);
        }
        self.handle_assertion(executor, state, condition, selector, None)
    }

    pub(super) fn set_expected_revert(
        &mut self,
        state: &mut PathState,
        data: ExpectedRevertData,
        reverter: Option<SymExpr>,
        count: u64,
    ) -> CheatcodeOutcome {
        if state.expected_revert.is_some() {
            return CheatcodeOutcome::Revert(error_string_return_data(
                &mut self.cx,
                "you must call another function prior to expecting a second revert",
            ));
        }
        state.expected_revert = Some(ExpectedRevert::new(data, reverter, count));
        state.first_failed_callee = None;
        CheatcodeOutcome::Continue(Vec::new())
    }

    fn expect_emit_from_args(
        &mut self,
        state: &mut PathState,
        args_offset: usize,
        checks: ExpectedEmitChecks,
        anonymous: bool,
        emitter_arg: Option<usize>,
        count_arg: Option<usize>,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        let emitter = emitter_arg
            .map(|index| read_abi_word_arg(&mut self.cx, &state.memory, args_offset, index))
            .transpose()?;
        let remaining = count_arg
            .map(|index| {
                read_abi_u64_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    index,
                    "symbolic vm.expectEmit",
                )
            })
            .transpose()?
            .unwrap_or(1);
        // Forge queues expectations; replacing a pending one would silently drop it.
        if state.expected_emit.is_some() {
            return Err(SymbolicError::Unsupported(
                "symbolic vm.expectEmit with a pending expectation",
            ));
        }
        state.expected_emit =
            Some(ExpectedEmit::new(checks, emitter, remaining, anonymous, state.call_depth));
        Ok(CheatcodeOutcome::Continue(Vec::new()))
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
        let expected = ExpectedCall::new(callee, value, gas, min_gas, data, count);
        match register_expected_call(&mut state.expected_calls, &mut self.cx, expected) {
            Ok(()) => CheatcodeOutcome::Continue(Vec::new()),
            Err(message) => {
                CheatcodeOutcome::Revert(error_string_return_data(&mut self.cx, message))
            }
        }
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
    pub(super) fn deploy_code_cheatcode_if_needed(
        &mut self,
        executor: &ExecutorView<'_>,
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
            let artifact = read_abi_string_arg(
                &mut self.cx,
                &state.memory,
                args_offset,
                0,
                "symbolic vm.deployCode",
            )?;
            (artifact, Vec::new())
        } else if selector == deployCode_1Call::SELECTOR {
            let artifact = read_abi_string_arg(
                &mut self.cx,
                &state.memory,
                args_offset,
                0,
                "symbolic vm.deployCode",
            )?;
            let args = read_abi_dynamic_bytes_arg(
                &mut self.cx,
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
    pub(super) fn deploy_code_cheatcode_call(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        artifact: String,
        constructor_args: Vec<u8>,
        out_offset: SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<StepOutcome, SymbolicError> {
        if state.is_static {
            state.first_failed_callee.get_or_insert(CHEATCODE_ADDRESS);
            state.first_reverted_callee.get_or_insert(CHEATCODE_ADDRESS);
            state.return_data = SymReturnData::empty(&mut self.cx);
            return Ok(StepOutcome::Revert);
        }

        // Like Forge, the CREATE applies and consumes a pending prank.
        let inherited_prank = state.callers();
        let (creator, creator_word, pranked_origin) = state.prank_for_next_call();
        if creator != state.address
            && state.constrained_word(&mut self.cx, &creator_word).map(word_to_address)
                != Some(creator)
        {
            return Err(SymbolicError::Unsupported("symbolic vm.deployCode prank"));
        }
        self.stateless_retry_safe = false;
        let mut initcode = artifact_code(&artifact, false)?;
        initcode.extend_from_slice(&constructor_args);
        let initcode = SymCode::concrete(&mut self.cx, initcode);

        let nonce = state.world.nonce(executor, creator)?;
        // EIP-2681: the create fails at a `u64::MAX` creator nonce, so the cheatcode reverts.
        if nonce == u64::MAX {
            let return_data = SymReturnData::empty(&mut self.cx);
            revert_cheatcode_call(&mut self.cx, state, out_offset, out_size, return_data)?;
            return Ok(StepOutcome::Continue);
        }
        let created = creator.create(nonce);
        let created_word = SymExpr::constant(&mut self.cx, address_word(created));

        let mut failure_world = state.world.clone();
        failure_world.increment_nonce(executor, creator)?;
        if failure_world.has_code_or_nonce(&mut self.cx, executor, created)? {
            state.world = failure_world;
            let zero = SymExpr::zero(&mut self.cx);
            let return_data = SymReturnData::from_words(&mut self.cx, vec![zero]);
            complete_cheatcode_call(&mut self.cx, state, out_offset, out_size, return_data)?;
            return Ok(StepOutcome::Continue);
        }

        let zero = SymExpr::zero(&mut self.cx);
        let calldata = SymBytes::empty(&mut self.cx);
        let calldata = SymCalldata::from_bytes(&mut self.cx, calldata);
        let mut frame =
            CallFrame::new(&mut self.cx, created, created, creator, zero, false, calldata);
        frame.address_word = created_word.clone();
        frame.caller_word = creator_word;
        frame.is_constructor = true;
        // Isolation runs a `deployCode` from the test frame as its own transaction.
        let isolated = state.starts_isolated_transaction(executor);
        let mut child = state.child(frame);
        child.prank.inherited = inherited_prank;
        // Forge runs the constructor in a nested EVM, so its call depth starts over even
        // without isolation.
        child.call_depth_base = child.call_depth;
        child.world = failure_world.clone();
        if isolated {
            child.world.clear_transient_storage();
        }
        if let Some((origin, origin_word)) = pranked_origin {
            child.origin = origin;
            child.origin_word = origin_word;
        }
        child.world.mark_current_transaction_created(created);
        child.persistent_writes.push(PersistentWrite::Created(created));
        child.world.set_nonce(created, 1);

        let outcomes = self.execute_external_call(executor, child, &initcode, completed_paths)?;
        if outcomes.is_empty() {
            return Ok(StepOutcome::AssumeRejected);
        }

        let mut parents = VecDeque::with_capacity(outcomes.len());
        for outcome in outcomes {
            if !matches!(outcome.status, CallStatus::Success)
                && state.expected_revert.as_ref().is_some_and(ExpectedRevert::repeats_with_reverter)
            {
                return Err(SymbolicError::Unsupported(
                    "symbolic vm.expectRevert reverter count on CREATE",
                ));
            }
            // Forge records a reverting CREATE, and otherwise the failed cheatcode call.
            let failed_callee = match outcome.status {
                CallStatus::Success => None,
                CallStatus::Revert => Some(created),
                CallStatus::ExceptionalHalt | CallStatus::Failure => Some(CHEATCODE_ADDRESS),
            };
            // Like Forge, a cheatcode CREATE leaves `vm.assumeNoRevert` for the next call.
            match self.join_call_outcome(
                executor,
                state,
                &failure_world,
                outcome,
                failed_callee,
                None,
            )? {
                JoinedCallOutcome::Failure(parent)
                | JoinedCallOutcome::Success { parent, unmet_expected_revert: Some(_), .. } => {
                    *state = parent;
                    return Ok(StepOutcome::Failure);
                }
                JoinedCallOutcome::ExceptionalHalt(mut parent) => {
                    parent.return_data = SymReturnData::empty(&mut self.cx);
                    parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), out_size)?;
                    parent.stack.push(SymExpr::zero(&mut self.cx))?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::ExpectedRevert(mut parent) => {
                    let zero = SymExpr::zero(&mut self.cx);
                    let return_data = SymReturnData::from_words(&mut self.cx, vec![zero]);
                    complete_cheatcode_call(
                        &mut self.cx,
                        &mut parent,
                        out_offset.clone(),
                        out_size,
                        return_data,
                    )?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::Success { mut parent, child, .. } => {
                    parent.world = child.world;
                    if isolated {
                        parent.world.restore_transient_storage(&failure_world);
                    }
                    self.observe_expected_create(
                        &mut parent,
                        creator,
                        CreateKind::Create,
                        &child.frame.return_data,
                    )?;
                    if !parent.world.is_destroyed(created) {
                        parent
                            .world
                            .install_code(created, child.frame.return_data.to_code(&mut self.cx)?);
                    }
                    let return_data =
                        SymReturnData::from_words(&mut self.cx, vec![created_word.clone()]);
                    complete_cheatcode_call(
                        &mut self.cx,
                        &mut parent,
                        out_offset.clone(),
                        out_size,
                        return_data,
                    )?;
                    parents.push_back(parent);
                }
                JoinedCallOutcome::Revert { mut parent, child } => {
                    parent.return_data = child.frame.return_data;
                    parent.copy_call_output_offset(&mut self.cx, out_offset.clone(), out_size)?;
                    parent.stack.push(SymExpr::zero(&mut self.cx))?;
                    parents.push_back(parent);
                }
            }
        }

        Ok(self.resume_parent_paths(state, worklist, parents))
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
        // An unrelated deployment may have symbolic immutable bytes in its runtime code.
        let conditions = state
            .expected_creates
            .iter()
            .filter_map(|expected| expected.deployer_match_condition(&mut self.cx, deployer, kind))
            .collect::<Vec<_>>();
        let condition = SymBoolExpr::or(&mut self.cx, conditions);
        let (_, match_sat) = self.constraints_with_condition(state, condition)?;
        if !match_sat {
            return Ok(());
        }
        let bytecode = runtime.read_concrete(&mut self.cx, "symbolic expected create bytecode")?;
        let mut mismatch_constraints = None;
        for idx in 0..state.expected_creates.len() {
            let Some(condition) = state.expected_creates[idx].match_condition(
                &mut self.cx,
                deployer,
                kind,
                &bytecode,
            ) else {
                continue;
            };
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let mismatch_condition = condition.not(&mut self.cx);
            let (candidate_mismatch_constraints, mismatch_sat) =
                self.constraints_with_condition(state, mismatch_condition)?;

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

    /// Returns the address words the CALL on top of the stack must decide before it runs, each
    /// with the known accounts to fork it over.
    ///
    /// `vm.etch` and the nonce cheatcodes through a free address fork over every known account,
    /// since they need concrete code and nonces, and `vm.store` over the known precompiles,
    /// where it reverts. `vm.deal` and the rest of a `vm.store` write each account under a
    /// guard instead. A word written that way is decided only once a call targets it or a
    /// cheatcode other than these writes and their reads takes it.
    pub(super) fn free_address_write_targets(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &PathState,
        kind: CallKind,
    ) -> Result<Vec<(SymExpr, Vec<Address>)>, SymbolicError> {
        let Ok(target) = state.stack.peek(1) else { return Ok(Vec::new()) };
        let Some(to) = target.as_const().map(word_to_address) else {
            let target = target.clone().address_operand(&mut self.cx);
            return Ok(self.guarded_address_word(state, target).into_iter().collect());
        };
        let in_offset_idx = match kind {
            CallKind::Call => 3,
            CallKind::StaticCall => 2,
            CallKind::CallCode | CallKind::DelegateCall => return Ok(Vec::new()),
        };
        if to != CHEATCODE_ADDRESS && to != SYMBOLIC_VM_COMPAT_ADDRESS {
            return Ok(Vec::new());
        }
        let (Ok(in_offset), Ok(in_size)) =
            (state.stack.peek(in_offset_idx), state.stack.peek(in_offset_idx + 1))
        else {
            return Ok(Vec::new());
        };
        let in_size = in_size.clone();
        let Some(in_offset) = state
            .constrained_word(&mut self.cx, in_offset)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            return Ok(Vec::new());
        };
        let Some(selector) = state
            .memory
            .read_concrete(&mut self.cx, in_offset, 4)
            .ok()
            .and_then(|selector| <[u8; 4]>::try_from(selector.as_slice()).ok())
        else {
            return Ok(Vec::new());
        };
        let args_offset = in_offset + 4;
        let mut words = Vec::new();
        // The first argument to scan for guarded words.
        let mut first_scanned = 0;
        if to == CHEATCODE_ADDRESS {
            let store = selector == storeCall::SELECTOR;
            if store
                || [etchCall::SELECTOR, setNonceCall::SELECTOR, setNonceUnsafeCall::SELECTOR]
                    .contains(&selector)
            {
                // The target forks here, or `vm.store` writes it under guards; the other
                // arguments may still be guarded words.
                first_scanned = 1;
                let word = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                if state.constrained_word(&mut self.cx, &word).is_none() {
                    let mut candidates = state.world.concrete_alias_candidates(executor);
                    if store {
                        candidates
                            .retain(|address| is_precompile_for_spec(*address, executor.spec_id));
                    }
                    if !candidates.is_empty() {
                        words.push((word, candidates));
                    }
                }
            }
            if [dealCall::SELECTOR, loadCall::SELECTOR, getNonce_0Call::SELECTOR]
                .contains(&selector)
            {
                return Ok(Vec::new());
            }
        }
        if !state.world.has_guarded_addresses() {
            return Ok(words);
        }
        // A symbolic input size scans up to its bound; extra words only add forks.
        let in_size = match state.constrained_usize_checked(&mut self.cx, &in_size) {
            Some(Ok(size)) => size,
            Some(Err(_)) => return Ok(words),
            None => self.solver_upper_bound_usize(
                state,
                &in_size,
                self.config.max_calldata_bytes as usize,
                "symbolic CALL input size",
            )?,
        };
        for index in first_scanned..in_size.saturating_sub(4) / 32 {
            let word = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, index)?;
            words.extend(self.guarded_address_word(state, word));
        }
        Ok(words)
    }

    /// Returns a word a guarded cheatcode write went through, with the accounts it may still
    /// denote, unless the path constraints already fix it.
    fn guarded_address_word(
        &mut self,
        state: &PathState,
        word: SymExpr,
    ) -> Option<(SymExpr, Vec<Address>)> {
        let candidates = state.world.guarded_candidates(&word)?;
        state.constrained_word(&mut self.cx, &word).is_none().then_some((word, candidates))
    }

    /// Forks an address word over `candidates`, concrete accounts the run knows, so a cheatcode
    /// write through it can land on one of them, or so a guarded word is decided. Accounts the
    /// run has not seen yet stay outside.
    pub(super) fn branch_free_address_write_if_needed(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        pre_call_state: &PathState,
        call_pc: usize,
        target: &SymExpr,
        candidates: &[Address],
    ) -> Result<bool, SymbolicError> {
        if candidates.len() > MAX_FREE_ADDRESS_ALIAS_CANDIDATES {
            return Err(SymbolicError::Unsupported(
                "symbolic cheatcode address with too many alias candidates",
            ));
        }
        // Only the low 160 bits name an account, also for a word passed as a plain integer.
        let address = target.clone().address_operand(&mut self.cx);
        for &candidate in candidates {
            let eq = SymBoolExpr::eq_word_const(&mut self.cx, &address, address_word(candidate));
            let (eq_constraints, eq_sat) = self.constraints_with_condition(state, eq.clone())?;
            let neq = eq.not(&mut self.cx);
            let (neq_constraints, neq_sat) = self.constraints_with_condition(state, neq)?;
            match (eq_sat, neq_sat) {
                (true, true) => {
                    for constraints in [eq_constraints, neq_constraints] {
                        let mut branch = pre_call_state.clone();
                        branch.pc = call_pc;
                        branch.constraints = constraints;
                        worklist.push_back(branch);
                    }
                    return Ok(true);
                }
                (true, false) => {
                    state.constraints = eq_constraints;
                    return Ok(false);
                }
                (false, true) => state.constraints = neq_constraints,
                (false, false) => return Ok(false),
            }
        }
        // The path rules out every candidate, so a guarded word may stand for its
        // representative account again.
        state.world.exclude_guarded_candidates(target, candidates);
        Ok(false)
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
        let target = read_abi_word_arg(&mut self.cx, &state.memory, in_offset + 4, 0)?;
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
            let condition = target.address_match_condition(&mut self.cx, address);
            matched_conditions.push(condition.clone());
            if let Some(constraints) = self.constraints_for_condition(state, condition)? {
                let mut branch = state.clone();
                branch.constraints = constraints;
                let return_data = accesses_return_data(&mut self.cx, Some(&record), address);
                complete_cheatcode_call(
                    &mut self.cx,
                    &mut branch,
                    out_offset.clone(),
                    out_size,
                    return_data,
                )?;
                branches.push_back(branch);
            }
        }

        let unmatched_conditions =
            matched_conditions.into_iter().map(|condition| condition.not(&mut self.cx)).collect();
        let unmatched_condition = SymBoolExpr::and(&mut self.cx, unmatched_conditions);
        if let Some(constraints) = self.constraints_for_condition(state, unmatched_condition)? {
            let mut branch = state.clone();
            branch.constraints = constraints;
            let return_data = accesses_return_data(&mut self.cx, Some(&record), Address::ZERO);
            complete_cheatcode_call(&mut self.cx, &mut branch, out_offset, out_size, return_data)?;
            branches.push_back(branch);
        }

        let Some(first_branch) = self.pop_next_path(&mut branches) else {
            return Ok(Some(StepOutcome::AssumeRejected));
        };
        *state = first_branch;
        worklist.extend(branches);
        Ok(Some(StepOutcome::Continue))
    }

    pub(super) fn accesses_return_data_for_target(
        &mut self,
        state: &mut PathState,
        target: SymExpr,
    ) -> Result<SymReturnData, SymbolicError> {
        let Some(record) = state.access_record.clone() else {
            return Ok(accesses_return_data(&mut self.cx, None, Address::ZERO));
        };

        if let Some(target) = target.as_const() {
            return Ok(accesses_return_data(&mut self.cx, Some(&record), word_to_address(target)));
        }

        let addresses = record.addresses();
        if addresses.is_empty() {
            return Ok(accesses_return_data(&mut self.cx, Some(&record), Address::ZERO));
        }

        for address in addresses {
            let condition = target.address_match_condition(&mut self.cx, address);
            let (match_constraints, match_sat) =
                self.constraints_with_condition(state, condition.clone())?;
            let mismatch_condition = condition.not(&mut self.cx);
            let (_, mismatch_sat) = self.constraints_with_condition(state, mismatch_condition)?;

            match (match_sat, mismatch_sat) {
                (true, false) => {
                    state.constraints = match_constraints;
                    return Ok(accesses_return_data(&mut self.cx, Some(&record), address));
                }
                (true, true) => {
                    return Err(SymbolicError::Unsupported("symbolic vm.accesses address"));
                }
                (false, _) => {}
            }
        }

        Ok(accesses_return_data(&mut self.cx, Some(&record), Address::ZERO))
    }

    /// Reverts a cheatcode call whose arguments Forge rejects, reading its input from `input`
    /// or, without one, the caller's memory. A check on symbolic arguments forks the path: the
    /// reverting branch completes here and the valid branch continues into the cheatcode.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn branch_cheatcode_argument_check_if_needed(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        selector: [u8; 4],
        input: Option<&SymMemory>,
        in_offset: usize,
        out_offset: &SymExpr,
        out_size: &BoundedCopySize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let memory = input.unwrap_or(&state.memory);
        let args_offset = in_offset + 4;
        let (invalid, message) = if selector == randomUint_1Call::SELECTOR {
            let min = read_abi_word_arg(&mut self.cx, memory, args_offset, 0)?;
            let max = read_abi_word_arg(&mut self.cx, memory, args_offset, 1)?;
            (
                SymBoolExpr::cmp_word_expr(&mut self.cx, SymCmpOp::Ugt, &min, max),
                "vm.randomUint: min must be less than or equal to max",
            )
        } else {
            let u64_max = U256::from(u64::MAX);
            let (index, max, message) = match selector {
                chainIdCall::SELECTOR => {
                    (0, u64_max, "vm.chainId: chain ID must be less than 2^64")
                }
                feeCall::SELECTOR => (0, u64_max, "vm.fee: base fee must be less than 2^64"),
                txGasPriceCall::SELECTOR => {
                    (0, u64_max, "vm.txGasPrice: gas price must be less than 2^64")
                }
                // Like Forge, the hard fork check comes first, in the cheatcode.
                blobBaseFeeCall::SELECTOR if executor.spec_id < SpecId::CANCUN => return Ok(None),
                blobBaseFeeCall::SELECTOR => (
                    0,
                    U256::from(u128::MAX),
                    "vm.blobBaseFee: blob base fee must be less than 2^128",
                ),
                setBlockhashCall::SELECTOR => {
                    (0, u64_max, "vm.setBlockhash: blockNumber must be less than 2^64")
                }
                computeCreateAddressCall::SELECTOR => {
                    (1, u64_max, "vm.computeCreateAddress: nonce must be less than 2^64")
                }
                _ => return Ok(None),
            };
            let word = read_abi_word_arg(&mut self.cx, memory, args_offset, index)?;
            (SymBoolExpr::cmp_word_const(&mut self.cx, SymCmpOp::Ugt, &word, max), message)
        };

        let reverted = self.fork_cheatcode_revert(
            state,
            worklist,
            invalid,
            out_offset,
            out_size,
            |this, _| error_string_return_data(&mut this.cx, message),
        )?;
        Ok(reverted.then_some(StepOutcome::Continue))
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn add_call_mock(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        callee: SymExpr,
        value: Option<U256>,
        data: SymBytes,
        returns: Vec<SymReturnData>,
        reverts: bool,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        // Like Forge, a codeless callee gets the one-byte code `0x00`, so Solidity's extcodesize
        // check before a high-level call passes.
        match state.constrained_word(&mut self.cx, &callee).map(word_to_address) {
            Some(address) => {
                if state.world.extcode(&mut self.cx, executor, address)?.is_empty() {
                    let code = SymCode::concrete(&mut self.cx, vec![0]);
                    state.world.install_code(address, code);
                }
            }
            // A symbolic callee may be codeless, and that check would prune a path Forge runs.
            None => self.defer_incomplete("symbolic vm.mockCall callee"),
        }
        // Replace identical definitions in place to preserve mock precedence.
        if let Some(existing) = state.call_mocks.iter_mut().find(|mock| {
            mock.callee == callee
                && mock.value() == value
                && mock.data.same_bytes(&mut self.cx, &data)
        }) {
            *existing = CallMock::new(callee, value, data, returns, reverts);
        } else {
            state.call_mocks.push(CallMock::new(callee, value, data, returns, reverts));
        }
        Ok(CheatcodeOutcome::Continue(Vec::new()))
    }

    pub(super) fn set_function_mock(
        &mut self,
        state: &mut PathState,
        callee: SymExpr,
        target: Address,
        data: SymBytes,
    ) -> CheatcodeOutcome {
        if let Some(mock) = state
            .function_mocks
            .iter_mut()
            .find(|mock| mock.matches_definition(&mut self.cx, &callee, &data))
        {
            mock.set_target(target);
        } else {
            state.function_mocks.push(FunctionMock::new(callee, target, data));
        }
        CheatcodeOutcome::Continue(Vec::new())
    }

    pub(super) fn handle_foundry_cheatcode(
        &mut self,
        executor: &ExecutorView<'_>,
        state: &mut PathState,
        selector: [u8; 4],
        in_offset: &SymExpr,
        input_size: &SymExpr,
        in_size: usize,
    ) -> Result<CheatcodeOutcome, SymbolicError> {
        if is_full_word_array_assertion(selector) {
            return self.handle_full_word_array_assertion(
                executor, state, selector, in_offset, input_size, in_size,
            );
        }
        let in_offset = in_offset.as_usize_or("symbolic cheatcode CALL input offset")?;
        let args_offset = in_offset + 4;
        match selector {
            assumeCall::SELECTOR => self.handle_assume(state, in_offset + 4),
            assumeNoRevert_0Call::SELECTOR
            | assumeNoRevert_1Call::SELECTOR
            | assumeNoRevert_2Call::SELECTOR => {
                if state.assume_no_revert_next_call.is_some() {
                    return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert overlap"));
                }
                let filters = if selector == assumeNoRevert_0Call::SELECTOR {
                    None
                } else {
                    let single = selector == assumeNoRevert_1Call::SELECTOR;
                    let mut values =
                        decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                    let value = values
                        .pop()
                        .ok_or(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"))?;
                    Some(if single {
                        vec![dyn_potential_revert(&mut self.cx, &value)?]
                    } else {
                        dyn_potential_reverts(&mut self.cx, &value)?
                    })
                };
                state.assume_no_revert_next_call =
                    Some(AssumeNoRevert { filters, reverted_by: None });
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            recordLogsCall::SELECTOR => {
                state.recorded_logs = Some(Vec::new());
                state.recorded_logs_generation += 1;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            recordCall::SELECTOR => {
                state.access_record = Some(AccessRecord::default());
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            stopRecordCall::SELECTOR => {
                if let Some(record) = &mut state.access_record {
                    record.stop();
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            accessesCall::SELECTOR => {
                let target = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                Ok(CheatcodeOutcome::ContinueData(
                    self.accesses_return_data_for_target(state, target)?,
                ))
            }
            registerSloadHookCall::SELECTOR | registerSstoreHookCall::SELECTOR => {
                let target = read_abi_address_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic storage hook target",
                )?;
                let callback: [u8; 4] = state
                    .memory
                    .read_concrete(&mut self.cx, args_offset + 32, 4)?
                    .try_into()
                    .map_err(|_| SymbolicError::Unsupported("symbolic storage hook callback"))?;
                let hook = SymbolicStorageHook {
                    callback_target: state.address,
                    callback_selector: callback,
                };
                if selector == registerSloadHookCall::SELECTOR {
                    state.storage_load_hooks.insert(target, hook);
                } else {
                    if state
                        .mapping_storage_store_hooks
                        .keys()
                        .any(|(address, _)| *address == target)
                    {
                        return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                            &mut self.cx,
                            "cannot register raw SSTORE hook: mapping SSTORE hooks already exist for target",
                        )));
                    }
                    state.storage_store_hooks.insert(target, hook);
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            registerMappingSstoreHookCall::SELECTOR => {
                let target = read_abi_address_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic mapping storage hook target",
                )?;
                let root = read_abi_concrete_word_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic mapping storage hook root",
                )?;
                let callback: [u8; 4] = state
                    .memory
                    .read_concrete(&mut self.cx, args_offset + 64, 4)?
                    .try_into()
                    .map_err(|_| {
                        SymbolicError::Unsupported("symbolic mapping storage hook callback")
                    })?;
                let hook = SymbolicStorageHook {
                    callback_target: state.address,
                    callback_selector: callback,
                };
                if state.storage_store_hooks.contains_key(&target) {
                    return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                        &mut self.cx,
                        "cannot register mapping SSTORE hook: raw SSTORE hook already exists for target",
                    )));
                }
                state.mapping_hook_keccak_preimages.retain(|(account, _), _| *account != target);
                state.mapping_storage_store_hooks.insert((target, root), hook);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getRecordedLogsCall::SELECTOR => {
                state.recorded_logs_generation += 1;
                let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
                Ok(CheatcodeOutcome::ContinueData(recorded_logs_return_data(&mut self.cx, logs)))
            }
            getRecordedLogsJsonCall::SELECTOR => {
                state.recorded_logs_generation += 1;
                let logs = state.recorded_logs.replace(Vec::new()).unwrap_or_default();
                Ok(CheatcodeOutcome::ContinueData(recorded_logs_json_return_data(
                    &mut self.cx,
                    logs,
                )?))
            }
            expectRevert_0Call::SELECTOR
            | expectRevert_1Call::SELECTOR
            | expectRevert_2Call::SELECTOR
            | expectRevert_3Call::SELECTOR
            | expectRevert_4Call::SELECTOR
            | expectRevert_5Call::SELECTOR
            | expectRevert_6Call::SELECTOR
            | expectRevert_7Call::SELECTOR
            | expectRevert_8Call::SELECTOR
            | expectRevert_9Call::SELECTOR
            | expectRevert_10Call::SELECTOR
            | expectRevert_11Call::SELECTOR
            | expectPartialRevert_0Call::SELECTOR
            | expectPartialRevert_1Call::SELECTOR => {
                let mut data = ExpectedRevertData::Any;
                let mut reverter = None;
                let mut count = 1;
                // Only `expectPartialRevert` matches the selector alone.
                let partial = matches!(
                    selector,
                    expectPartialRevert_0Call::SELECTOR | expectPartialRevert_1Call::SELECTOR
                );
                for (index, param) in vm_params(selector).into_iter().enumerate() {
                    match param {
                        DynSolType::FixedBytes(4) => {
                            let bytes = read_abi_bytes4_words_arg(
                                &mut self.cx,
                                &state.memory,
                                args_offset,
                                index,
                            );
                            let bytes = SymBytes::exprs(&mut self.cx, bytes);
                            data = if partial {
                                ExpectedRevertData::Prefix(bytes)
                            } else {
                                ExpectedRevertData::Exact(bytes)
                            };
                        }
                        DynSolType::Bytes => {
                            let bytes = read_abi_symbolic_dynamic_byte_exprs_arg(
                                &mut self.cx,
                                state,
                                args_offset,
                                index,
                                self.config.max_calldata_bytes as usize,
                                "symbolic vm.expectRevert",
                            )?;
                            data = ExpectedRevertData::Exact(SymBytes::exprs(&mut self.cx, bytes));
                        }
                        DynSolType::Address => {
                            reverter = Some(read_abi_word_arg(
                                &mut self.cx,
                                &state.memory,
                                args_offset,
                                index,
                            )?);
                        }
                        _ => {
                            count = read_abi_u64_arg(
                                &mut self.cx,
                                &state.memory,
                                args_offset,
                                index,
                                "symbolic vm.expectRevert",
                            )?;
                        }
                    }
                }
                // Forge reads a zero count as "expect no revert", which is not modeled.
                if count == 0 {
                    return Err(SymbolicError::Unsupported("symbolic vm.expectRevert count 0"));
                }
                Ok(self.set_expected_revert(state, data, reverter, count))
            }
            expectEmitAnonymous_0Call::SELECTOR
            | expectEmitAnonymous_1Call::SELECTOR
            | expectEmitAnonymous_2Call::SELECTOR
            | expectEmitAnonymous_3Call::SELECTOR
            | expectEmit_0Call::SELECTOR
            | expectEmit_1Call::SELECTOR
            | expectEmit_2Call::SELECTOR
            | expectEmit_3Call::SELECTOR
            | expectEmit_4Call::SELECTOR
            | expectEmit_5Call::SELECTOR
            | expectEmit_6Call::SELECTOR
            | expectEmit_7Call::SELECTOR => {
                let params = vm_params(selector);
                let checks = match params.iter().filter(|param| **param == DynSolType::Bool).count()
                {
                    0 => ExpectedEmitChecks::default(),
                    4 => ExpectedEmitChecks::from_non_anonymous_args(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                    )?,
                    _ => ExpectedEmitChecks::from_anonymous_args(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                    )?,
                };
                let emitter = params.iter().position(|param| *param == DynSolType::Address);
                let count = params.iter().position(|param| *param == DynSolType::Uint(64));
                let anonymous = matches!(
                    selector,
                    expectEmitAnonymous_0Call::SELECTOR
                        | expectEmitAnonymous_1Call::SELECTOR
                        | expectEmitAnonymous_2Call::SELECTOR
                        | expectEmitAnonymous_3Call::SELECTOR
                );
                self.expect_emit_from_args(state, args_offset, checks, anonymous, emitter, count)
            }
            expectCall_0Call::SELECTOR
            | expectCall_1Call::SELECTOR
            | expectCall_2Call::SELECTOR
            | expectCall_3Call::SELECTOR
            | expectCall_4Call::SELECTOR
            | expectCall_5Call::SELECTOR
            | expectCallMinGas_0Call::SELECTOR
            | expectCallMinGas_1Call::SELECTOR => {
                let callee = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let mut value = None;
                let mut gas = None;
                let mut data = None;
                let mut count = None;
                for (index, param) in vm_params(selector).into_iter().enumerate().skip(1) {
                    match param {
                        DynSolType::Uint(256) => {
                            value = Some(read_abi_concrete_word_arg(
                                &mut self.cx,
                                &state.memory,
                                args_offset,
                                index,
                                "symbolic vm.expectCall",
                            )?);
                        }
                        DynSolType::Bytes => {
                            data = Some(read_abi_symbolic_dynamic_byte_exprs_arg(
                                &mut self.cx,
                                state,
                                args_offset,
                                index,
                                self.config.max_calldata_bytes as usize,
                                "symbolic vm.expectCall",
                            )?);
                        }
                        _ => {
                            let arg = read_abi_u64_arg(
                                &mut self.cx,
                                &state.memory,
                                args_offset,
                                index,
                                "symbolic vm.expectCall",
                            )?;
                            if data.is_some() { count = Some(arg) } else { gas = Some(arg) }
                        }
                    }
                }
                let data = SymBytes::exprs(&mut self.cx, data.unwrap_or_default());
                let (gas, min_gas) = if matches!(
                    selector,
                    expectCallMinGas_0Call::SELECTOR | expectCallMinGas_1Call::SELECTOR
                ) {
                    (None, gas)
                } else {
                    (gas, None)
                };
                Ok(self.set_expected_call(state, callee, value, gas, min_gas, data, count))
            }
            expectCreateCall::SELECTOR | expectCreate2Call::SELECTOR => {
                let bytecode = read_abi_dynamic_bytes_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.expectCreate bytecode",
                )?;
                let deployer = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let kind = if selector == expectCreateCall::SELECTOR {
                    CreateKind::Create
                } else {
                    CreateKind::Create2
                };
                Ok(self.set_expected_create(state, bytecode, deployer, kind))
            }
            clearMockedCallsCall::SELECTOR => {
                state.call_mocks.clear();
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            mockCall_0Call::SELECTOR
            | mockCall_1Call::SELECTOR
            | mockCall_2Call::SELECTOR
            | mockCall_3Call::SELECTOR
            | mockCallRevert_0Call::SELECTOR
            | mockCallRevert_1Call::SELECTOR
            | mockCallRevert_2Call::SELECTOR
            | mockCallRevert_3Call::SELECTOR => {
                let revert = matches!(
                    selector,
                    mockCallRevert_0Call::SELECTOR
                        | mockCallRevert_1Call::SELECTOR
                        | mockCallRevert_2Call::SELECTOR
                        | mockCallRevert_3Call::SELECTOR
                );
                let message =
                    if revert { "symbolic vm.mockCallRevert" } else { "symbolic vm.mockCall" };
                let params = vm_params(selector);
                let callee = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let value = (params[1] == DynSolType::Uint(256))
                    .then(|| {
                        read_abi_concrete_word_arg(
                            &mut self.cx,
                            &state.memory,
                            args_offset,
                            1,
                            message,
                        )
                    })
                    .transpose()?;
                let data_index = params.len() - 2;
                let data = if params[data_index] == DynSolType::FixedBytes(4) {
                    read_abi_bytes4_words_arg(&mut self.cx, &state.memory, args_offset, data_index)
                } else {
                    read_abi_symbolic_dynamic_byte_exprs_arg(
                        &mut self.cx,
                        state,
                        args_offset,
                        data_index,
                        self.config.max_calldata_bytes as usize,
                        message,
                    )?
                };
                let ret = read_abi_dynamic_return_data_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    data_index + 1,
                    self.config.max_calldata_bytes as usize,
                    message,
                )?;
                let data = SymBytes::exprs(&mut self.cx, data);
                self.add_call_mock(executor, state, callee, value, data, vec![ret], revert)
            }
            mockCalls_0Call::SELECTOR | mockCalls_1Call::SELECTOR => {
                let has_value = selector == mockCalls_1Call::SELECTOR;
                let (value, data_idx, ret_idx) = if has_value {
                    let value = read_abi_concrete_word_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        1,
                        "symbolic vm.mockCalls",
                    )?;
                    (Some(value), 2, 3)
                } else {
                    (None, 1, 2)
                };
                let callee = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    data_idx,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockCalls data",
                )?;
                let returns = read_abi_symbolic_dynamic_bytes_array_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    ret_idx,
                    self.config.max_dynamic_length as usize,
                    self.config.max_calldata_bytes as usize,
                )?;
                let data = SymBytes::exprs(&mut self.cx, data);
                self.add_call_mock(executor, state, callee, value, data, returns, false)
            }
            mockFunctionCall::SELECTOR => {
                let callee = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let target = read_abi_address_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.mockFunction",
                )?;
                let data = read_abi_symbolic_dynamic_byte_exprs_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    2,
                    self.config.max_calldata_bytes as usize,
                    "symbolic vm.mockFunction",
                )?;
                let data = SymBytes::exprs(&mut self.cx, data);
                Ok(self.set_function_mock(state, callee, target, data))
            }
            prank_0Call::SELECTOR
            | prank_1Call::SELECTOR
            | prank_2Call::SELECTOR
            | prank_3Call::SELECTOR
            | startPrank_0Call::SELECTOR
            | startPrank_1Call::SELECTOR
            | startPrank_2Call::SELECTOR
            | startPrank_3Call::SELECTOR => {
                let persistent = matches!(
                    selector,
                    startPrank_0Call::SELECTOR
                        | startPrank_1Call::SELECTOR
                        | startPrank_2Call::SELECTOR
                        | startPrank_3Call::SELECTOR
                );
                let (message, delegate_message) = if persistent {
                    ("symbolic vm.startPrank", "symbolic vm.startPrank delegatecall")
                } else {
                    ("symbolic vm.prank", "symbolic vm.prank delegatecall")
                };
                let params = vm_params(selector);
                if params.last() == Some(&DynSolType::Bool)
                    && read_abi_bool_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        params.len() - 1,
                        message,
                    )?
                {
                    return Err(SymbolicError::Unsupported(delegate_message));
                }
                let caller = read_abi_address_word_or_symbolic_slot_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                )?;
                let origin = (params.get(1) == Some(&DynSolType::Address))
                    .then(|| {
                        read_abi_address_word_or_symbolic_slot_arg(
                            &mut self.cx,
                            state,
                            args_offset,
                            1,
                        )
                    })
                    .transpose()?;
                // Like Forge, root frames share one prank slot, so a prank parked for another root
                // frame also blocks an overwrite, and a new prank ends it.
                let parked = state.root_pranks.values().filter(|_| state.call_depth == 0);
                if let Some(error) = std::iter::once(&state.prank)
                    .chain(parked)
                    .find_map(|prank| prank.overwrite_error(persistent))
                {
                    let name = if persistent { "startPrank" } else { "prank" };
                    return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                        &mut self.cx,
                        &format!("vm.{name}: {error}"),
                    )));
                }
                if state.call_depth == 0 {
                    state.root_pranks.clear();
                }
                if persistent {
                    state.prank.set_persistent(caller, origin);
                } else {
                    state.prank.set_next(caller, origin);
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            stopPrankCall::SELECTOR => {
                // A stopped prank also ends a prank parked for another root frame.
                if state.call_depth == 0 {
                    state.root_pranks.clear();
                }
                state.prank.stop();
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            readCallersCall::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(state.read_callers_words(&mut self.cx)))
            }
            addrCall::SELECTOR => {
                let private_key = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.addr",
                )?;
                let address = private_key_address(private_key)?;
                let address = SymExpr::constant(&mut self.cx, address_word(address));
                Ok(CheatcodeOutcome::Continue(vec![address]))
            }
            sign_1Call::SELECTOR | signCompact_1Call::SELECTOR => {
                let compact = selector == signCompact_1Call::SELECTOR;
                let message = if compact { "symbolic vm.signCompact" } else { "symbolic vm.sign" };
                let private_key =
                    read_abi_constrained_word_arg(&mut self.cx, state, args_offset, 0, message)?;
                let digest =
                    read_abi_constrained_word_arg(&mut self.cx, state, args_offset, 1, message)?;
                Ok(CheatcodeOutcome::Continue(if compact {
                    sign_compact_hash_words(&mut self.cx, private_key, digest)?
                } else {
                    sign_hash_words(&mut self.cx, private_key, digest)?
                }))
            }
            deriveKey_0Call::SELECTOR
            | deriveKey_1Call::SELECTOR
            | deriveKey_2Call::SELECTOR
            | deriveKey_3Call::SELECTOR => {
                let params = vm_params(selector);
                let mnemonic = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.deriveKey",
                )?;
                let index_arg = if params[1] == DynSolType::String { 2 } else { 1 };
                let path = if index_arg == 2 {
                    read_abi_string_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        1,
                        "symbolic vm.deriveKey",
                    )?
                } else {
                    DEFAULT_DERIVATION_PATH_PREFIX.to_string()
                };
                let index = read_abi_u32_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    index_arg,
                    "symbolic vm.deriveKey",
                )?;
                let private_key = if params.len() > index_arg + 1 {
                    let language = read_abi_string_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        index_arg + 1,
                        "symbolic vm.deriveKey",
                    )?;
                    derive_private_key_with_language(&mnemonic, &path, index, &language)?
                } else {
                    derive_private_key::<English>(&mnemonic, &path, index)?
                };
                let private_key = SymExpr::constant(&mut self.cx, private_key);
                Ok(CheatcodeOutcome::Continue(vec![private_key]))
            }
            rememberKeyCall::SELECTOR => {
                let private_key = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.rememberKey",
                )?;
                let address = private_key_address(private_key)?;
                state.wallets.insert(address);
                let address = SymExpr::constant(&mut self.cx, address_word(address));
                Ok(CheatcodeOutcome::Continue(vec![address]))
            }
            rememberKeys_0Call::SELECTOR | rememberKeys_1Call::SELECTOR => {
                let mnemonic = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.rememberKeys",
                )?;
                let path = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.rememberKeys",
                )?;
                let (language, count_index) = if selector == rememberKeys_1Call::SELECTOR {
                    (
                        Some(read_abi_string_arg(
                            &mut self.cx,
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
                    &mut self.cx,
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
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                    &mut self.cx,
                    DynSolValue::Array(addresses),
                )))
            }
            getWalletsCall::SELECTOR => {
                let wallets = DynSolValue::Array(
                    state.wallets.iter().copied().map(DynSolValue::Address).collect(),
                );
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(&mut self.cx, wallets)))
            }
            storeCall::SELECTOR => {
                let (target, word) = read_abi_address_word_or_symbolic_slot_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                )?;
                if is_precompile_for_spec(target, executor.spec_id) {
                    return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                        &mut self.cx,
                        &format!("vm.store: cannot use precompile {target} as an argument"),
                    )));
                }
                let slot = state.memory.load_word(&mut self.cx, in_offset + 36)?;
                let value = state.memory.load_word(&mut self.cx, in_offset + 68)?;
                // The call forked off the precompiles this write may name, where it reverts.
                for (address, condition) in
                    state.guard_free_address_write(&mut self.cx, executor, &word)
                {
                    if !is_precompile_for_spec(address, executor.spec_id) {
                        state.world.guarded_sstore(address, slot.clone(), value.clone(), condition);
                    }
                }
                state.world.sstore(target, slot, value);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            loadCall::SELECTOR => {
                let target = read_abi_address_word_arg(&mut self.cx, state, args_offset, 0)?;
                let slot = state.memory.load_word(&mut self.cx, in_offset + 36)?;
                let concrete_slot = state.constrained_word(&mut self.cx, &slot);
                let value =
                    state.world.sload_word(&mut self.cx, executor, target, slot, concrete_slot)?;
                Ok(CheatcodeOutcome::Continue(vec![value]))
            }
            getNonce_0Call::SELECTOR => {
                let target = read_abi_address_word_arg(&mut self.cx, state, args_offset, 0)?;
                let nonce = state.world.nonce_word(&mut self.cx, executor, target)?;
                Ok(CheatcodeOutcome::Continue(vec![nonce]))
            }
            computeCreateAddressCall::SELECTOR => {
                let deployer = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let nonce = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let address = compute_create_address_word(&mut self.cx, state, deployer, nonce)?;
                Ok(CheatcodeOutcome::Continue(vec![address]))
            }
            computeCreate2Address_0Call::SELECTOR | computeCreate2Address_1Call::SELECTOR => {
                let salt = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let init_code_hash =
                    read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let deployer = if selector == computeCreate2Address_0Call::SELECTOR {
                    read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 2)?
                } else {
                    SymExpr::constant(&mut self.cx, address_word(DEFAULT_CREATE2_DEPLOYER))
                };
                let address = compute_create2_address_word(
                    &mut self.cx,
                    state,
                    deployer,
                    salt,
                    init_code_hash,
                )?;
                Ok(CheatcodeOutcome::Continue(vec![address]))
            }
            etchCall::SELECTOR => {
                let target =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                if is_precompile_for_spec(target, executor.spec_id) {
                    return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                        &mut self.cx,
                        &format!("vm.etch: cannot use precompile {target} as an argument"),
                    )));
                }
                // Constant code of any length is read whole; only symbolic code is bounded.
                let code = if let Ok(code) = read_abi_dynamic_bytes_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.etch",
                ) {
                    let code = SymBytes::concrete(&mut self.cx, code);
                    SymCode::from_bytes(&mut self.cx, code)
                } else {
                    let code = read_abi_symbolic_dynamic_byte_exprs_arg(
                        &mut self.cx,
                        state,
                        args_offset,
                        1,
                        self.config.max_dynamic_length as usize,
                        "symbolic vm.etch",
                    )?;
                    SymCode::from_byte_exprs(&mut self.cx, code)
                };
                let delegation_prefix = SymBytes::concrete(&mut self.cx, vec![0xef, 0x01]);
                let delegation = code
                    .read_bytes(&mut self.cx, 0, delegation_prefix.len())
                    .prefix_condition(&mut self.cx, &delegation_prefix);
                if let Some(delegation) = delegation {
                    let valid = if code.len() == EIP7702_BYTECODE_LEN {
                        let delegation = SymBytes::concrete(&mut self.cx, vec![0xef, 0x01, 0x00]);
                        code.read_bytes(&mut self.cx, 0, delegation.len())
                            .prefix_condition(&mut self.cx, &delegation)
                            .unwrap_or_else(|| SymBoolExpr::constant(&mut self.cx, false))
                    } else {
                        SymBoolExpr::constant(&mut self.cx, false)
                    };
                    let invalid = valid.not(&mut self.cx);
                    let malformed = SymBoolExpr::and(&mut self.cx, vec![delegation, invalid]);
                    let (_, malformed_sat) = self.constraints_with_condition(state, malformed)?;
                    if malformed_sat {
                        return Err(SymbolicError::Unsupported("symbolic vm.etch EIP-7702 code"));
                    }
                }
                state.world.install_code(target, code);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getCodeCall::SELECTOR | getDeployedCodeCall::SELECTOR => {
                let artifact = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.getCode",
                )?;
                self.stateless_retry_safe = false;
                let code = artifact_code(&artifact, selector == getDeployedCodeCall::SELECTOR)?;
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&mut self.cx, &code)))
            }
            dealCall::SELECTOR => {
                let (target, word) = read_abi_address_word_or_symbolic_slot_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                )?;
                let value = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                if !self.symbolic_gas() && value.contains_gasleft() {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                let value = state
                    .constrained_word(&mut self.cx, &value)
                    .map(|value| SymExpr::constant(&mut self.cx, value))
                    .unwrap_or(value);
                for (address, condition) in
                    state.guard_free_address_write(&mut self.cx, executor, &word)
                {
                    let previous =
                        state.world.balance_word_for_address(&mut self.cx, executor, address);
                    let balance = SymExpr::ite(&mut self.cx, condition, value.clone(), previous);
                    state.set_persistent_balance(&mut self.cx, executor, address, balance);
                }
                state.set_persistent_balance(&mut self.cx, executor, target, value);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            setNonceCall::SELECTOR | setNonceUnsafeCall::SELECTOR => {
                let target =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                let nonce = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    1,
                    "symbolic vm.setNonce",
                )?;
                let Ok(nonce) = u64::try_from(nonce) else {
                    return Err(SymbolicError::Unsupported("symbolic vm.setNonce nonce"));
                };
                if selector == setNonceCall::SELECTOR
                    && nonce < state.world.nonce(executor, target)?
                {
                    return Ok(CheatcodeOutcome::Failure);
                }
                state.set_persistent_nonce(executor, target, nonce)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            resetNonceCall::SELECTOR => {
                let target =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                let code = state.world.extcode(&mut self.cx, executor, target)?;
                // Like the concrete cheatcode, EIP-7702 delegated EOAs reset to 0.
                let delegated = if !code.is_legacy() && code.len() == EIP7702_BYTECODE_LEN {
                    let prefix = SymBytes::concrete(&mut self.cx, vec![0xef, 0x01, 0x00]);
                    code.read_bytes(&mut self.cx, 0, prefix.len())
                        .prefix_condition(&mut self.cx, &prefix)
                } else {
                    None
                };
                let is_eoa = if let Some(delegated) = delegated {
                    let (_, delegated_sat) =
                        self.constraints_with_condition(state, delegated.clone())?;
                    let not_delegated = delegated.not(&mut self.cx);
                    let (_, not_delegated_sat) =
                        self.constraints_with_condition(state, not_delegated)?;
                    if delegated_sat && not_delegated_sat {
                        return Err(SymbolicError::Unsupported(
                            "symbolic vm.resetNonce code prefix",
                        ));
                    }
                    delegated_sat
                } else {
                    code.is_empty()
                };
                let nonce = if is_eoa { 0 } else { 1 };
                state.set_persistent_nonce(executor, target, nonce)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            allowCheatcodesCall::SELECTOR => Ok(CheatcodeOutcome::Continue(Vec::new())),
            makePersistent_0Call::SELECTOR
            | makePersistent_1Call::SELECTOR
            | makePersistent_2Call::SELECTOR => {
                for index in 0..vm_params(selector).len() {
                    let account = read_abi_address_or_symbolic_slot_arg(
                        &mut self.cx,
                        state,
                        args_offset,
                        index,
                    )?;
                    state.persistent_accounts.insert(account);
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            makePersistent_3Call::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                for account in dyn_address_array(&values[0])? {
                    state.persistent_accounts.insert(account);
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            revokePersistent_0Call::SELECTOR => {
                let account =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                state.persistent_accounts.remove(&account);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            revokePersistent_1Call::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                for account in dyn_address_array(&values[0])? {
                    state.persistent_accounts.remove(&account);
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            isPersistentCall::SELECTOR => {
                let account =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                let exists = SymExpr::constant(
                    &mut self.cx,
                    U256::from(state.persistent_accounts.contains(&account)),
                );
                Ok(CheatcodeOutcome::Continue(vec![exists]))
            }
            activeForkCall::SELECTOR => {
                let id = executor.active_fork_id.ok_or(SymbolicError::Unsupported(
                    "symbolic vm.activeFork requires an active forked executor",
                ))?;
                let id = SymExpr::constant(&mut self.cx, id);
                Ok(CheatcodeOutcome::Continue(vec![id]))
            }
            selectForkCall::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.selectFork id",
                )?;
                if executor.active_fork_id == Some(id) {
                    return Ok(CheatcodeOutcome::Continue(Vec::new()));
                }
                Err(SymbolicError::Unsupported(
                    "symbolic vm.selectFork can only select the already active fork",
                ))
            }
            rollFork_0Call::SELECTOR | rollFork_2Call::SELECTOR => {
                let with_id = selector == rollFork_2Call::SELECTOR;
                let active_fork = !with_id
                    || executor.active_fork_id
                        == Some(read_abi_constrained_word_arg(
                            &mut self.cx,
                            state,
                            args_offset,
                            0,
                            "symbolic vm.rollFork id",
                        )?);
                let block_number = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    usize::from(with_id),
                    "symbolic vm.rollFork block number",
                )?;
                let current =
                    state.block.number.as_const_or("symbolic vm.rollFork current block")?;
                if active_fork && block_number == current {
                    return Ok(CheatcodeOutcome::Continue(Vec::new()));
                }
                Err(SymbolicError::Unsupported(
                    "symbolic vm.rollFork cannot change the active fork block during symbolic execution",
                ))
            }
            createFork_0Call::SELECTOR
            | createFork_1Call::SELECTOR
            | createFork_2Call::SELECTOR
            | createSelectFork_0Call::SELECTOR
            | createSelectFork_1Call::SELECTOR
            | createSelectFork_2Call::SELECTOR
            | rollFork_1Call::SELECTOR
            | rollFork_3Call::SELECTOR => Err(SymbolicError::Unsupported(
                "symbolic fork creation and fork block mutation must happen before symbolic execution",
            )),
            snapshotCall::SELECTOR | snapshotStateCall::SELECTOR => {
                let id = state.world.snapshot_state(&state.block, &state.gas_price);
                let id = SymExpr::constant(&mut self.cx, id);
                Ok(CheatcodeOutcome::Continue(vec![id]))
            }
            revertToCall::SELECTOR
            | revertToStateCall::SELECTOR
            | revertToAndDeleteCall::SELECTOR
            | revertToStateAndDeleteCall::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.revertToState snapshot",
                )?;
                let restored = state.world.restore_snapshot(id);
                let success = restored.is_some();
                if let Some((block, gas_price)) = restored {
                    state.block = block;
                    state.gas_price = gas_price;
                    state.persistent_writes.push(PersistentWrite::Restore);
                }
                if success
                    && (selector == revertToAndDeleteCall::SELECTOR
                        || selector == revertToStateAndDeleteCall::SELECTOR)
                {
                    state.world.delete_snapshot(id);
                }
                let success = SymExpr::constant(&mut self.cx, U256::from(success));
                Ok(CheatcodeOutcome::Continue(vec![success]))
            }
            deleteSnapshotCall::SELECTOR | deleteStateSnapshotCall::SELECTOR => {
                let id = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.deleteStateSnapshot snapshot",
                )?;
                let success = state.world.delete_snapshot(id);
                let success = SymExpr::constant(&mut self.cx, U256::from(success));
                Ok(CheatcodeOutcome::Continue(vec![success]))
            }
            deleteSnapshotsCall::SELECTOR | deleteStateSnapshotsCall::SELECTOR => {
                state.world.delete_snapshots();
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            warpCall::SELECTOR => {
                state.block.timestamp = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            rollCall::SELECTOR => {
                state.block.number = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            rollSlotCall::SELECTOR | getSlotNumberCall::SELECTOR
                if executor.spec_id < SpecId::AMSTERDAM =>
            {
                let message = if selector == rollSlotCall::SELECTOR {
                    "vm.rollSlot: `rollSlot` is not supported before the Amsterdam hard fork; \
                     see EIP-7843: https://eips.ethereum.org/EIPS/eip-7843"
                } else {
                    "vm.getSlotNumber: `getSlotNumber` is not supported before the Amsterdam hard \
                     fork; see EIP-7843: https://eips.ethereum.org/EIPS/eip-7843"
                };
                Ok(CheatcodeOutcome::Revert(error_string_return_data(&mut self.cx, message)))
            }
            rollSlotCall::SELECTOR => {
                // The concrete cheatcode decodes a `uint64`, which drops the high bits.
                let slot = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                let mask = SymExpr::constant(&mut self.cx, U256::from(u64::MAX));
                state.block.slot_num = SymExpr::binop(&mut self.cx, SymBinOp::And, slot, mask);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            setBlockhashCall::SELECTOR => {
                let block_number = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.setBlockhash block number",
                )?;
                let block_hash = state.memory.load_word(&mut self.cx, in_offset + 36)?;
                if !state.block.set_block_hash(block_number, block_hash)? {
                    return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                        &mut self.cx,
                        "vm.setBlockhash: block number must be less than or equal to the current block number",
                    )));
                }
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            prevrandao_0Call::SELECTOR | prevrandao_1Call::SELECTOR => {
                state.block.difficulty = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            blobhashesCall::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                state.block.blob_hashes = dyn_bytes32_array(&values[0])?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
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
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(&mut self.cx, value)))
            }
            feeCall::SELECTOR => {
                state.block.basefee = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            blobBaseFeeCall::SELECTOR if executor.spec_id < SpecId::CANCUN => {
                Ok(CheatcodeOutcome::Revert(error_string_return_data(
                    &mut self.cx,
                    "vm.blobBaseFee: `blobBaseFee` is not supported before the Cancun hard fork; \
                     see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844",
                )))
            }
            blobBaseFeeCall::SELECTOR => {
                state.block.blob_basefee = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getBlobBaseFeeCall::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.block.blob_basefee.clone()]))
            }
            chainIdCall::SELECTOR => {
                state.block.chain_id = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getChainIdCall::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.block.chain_id.clone()]))
            }
            difficultyCall::SELECTOR if executor.spec_id >= SpecId::MERGE => {
                Ok(CheatcodeOutcome::Revert(error_string_return_data(
                    &mut self.cx,
                    "vm.difficulty: `difficulty` is not supported after the Paris hard fork, use \
                     `prevrandao` instead; see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399",
                )))
            }
            difficultyCall::SELECTOR => {
                state.block.difficulty = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            coinbaseCall::SELECTOR => {
                let coinbase = read_abi_constrained_address_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic vm.coinbase value",
                )?;
                state.block.coinbase = coinbase;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getBlockNumberCall::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.block.number.clone()]))
            }
            getSlotNumberCall::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.block.slot_num.clone()]))
            }
            txGasPriceCall::SELECTOR => {
                state.gas_price = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getBlockTimestampCall::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.block.timestamp.clone()]))
            }
            labelCall::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let account = dyn_address(&values[0])?;
                let label = dyn_string(&values[1])?;
                state.labels.insert(account, label);
                Ok(CheatcodeOutcome::Continue(Vec::new()))
            }
            getLabelCall::SELECTOR => {
                let account = read_abi_address_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.getLabel",
                )?;
                let label = state
                    .labels
                    .get(&account)
                    .cloned()
                    .unwrap_or_else(|| format!("unlabeled:{account}"));
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    label.as_bytes(),
                )))
            }
            expectSafeMemoryCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.expectSafeMemory not modeled"))
            }
            expectSafeMemoryCallCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.expectSafeMemoryCall not modeled"))
            }
            stopExpectSafeMemoryCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.stopExpectSafeMemory not modeled"))
            }
            lastCallGasCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.lastCallGas not modeled"))
            }
            lastFrameGasCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.lastFrameGas not modeled"))
            }
            snapshotGasLastCall_0Call::SELECTOR | snapshotGasLastCall_1Call::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.snapshotGasLastCall not modeled"))
            }
            snapshotGasLastFrame_0Call::SELECTOR | snapshotGasLastFrame_1Call::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.snapshotGasLastFrame not modeled"))
            }
            stopSnapshotGas_0Call::SELECTOR
            | stopSnapshotGas_1Call::SELECTOR
            | stopSnapshotGas_2Call::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.stopSnapshotGas not modeled"))
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
            | noAccessListCall::SELECTOR => Ok(CheatcodeOutcome::Continue(Vec::new())),
            setEvmVersionCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.setEvmVersion not modeled"))
            }
            getEvmVersionCall::SELECTOR => {
                Err(SymbolicError::Unsupported("symbolic vm.getEvmVersion not modeled"))
            }
            getFoundryVersionCall::SELECTOR => Ok(CheatcodeOutcome::ContinueData(
                abi_concrete_bytes_return(&mut self.cx, SEMVER_VERSION.as_bytes()),
            )),
            projectRootCall::SELECTOR => {
                self.stateless_retry_safe = false;
                let root = std::env::current_dir()
                    .map_err(|_| SymbolicError::Unsupported("symbolic vm.projectRoot"))?;
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    root.display().to_string().as_bytes(),
                )))
            }
            unixTimeCall::SELECTOR => {
                self.stateless_retry_safe = false;
                let milliseconds = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?
                    .as_millis();
                let value = U256::try_from(milliseconds)
                    .map_err(|_| SymbolicError::Unsupported("symbolic vm.unixTime"))?;
                let value = SymExpr::constant(&mut self.cx, value);
                Ok(CheatcodeOutcome::Continue(vec![value]))
            }
            isIsolateModeCall::SELECTOR => {
                let isolate = SymExpr::constant(&mut self.cx, U256::from(executor.isolate));
                Ok(CheatcodeOutcome::Continue(vec![isolate]))
            }
            isContextCall::SELECTOR => {
                let context = read_abi_concrete_word_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.isContext",
                )?;
                let context = u8::try_from(context)
                    .ok()
                    .and_then(|context| ForgeContext::try_from(context).ok())
                    .ok_or(SymbolicError::Unsupported("symbolic vm.isContext invalid context"))?;
                Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(
                    &mut self.cx,
                    U256::from(current_execution_context() == Some(context)),
                )]))
            }
            toString_0Call::SELECTOR
            | toString_1Call::SELECTOR
            | toString_2Call::SELECTOR
            | toString_3Call::SELECTOR
            | toString_4Call::SELECTOR
            | toString_5Call::SELECTOR => {
                let output = match selector {
                    toString_0Call::SELECTOR => read_abi_address_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        0,
                        "symbolic vm.toString",
                    )?
                    .to_string(),
                    toString_1Call::SELECTOR => hex::encode_prefixed(read_abi_dynamic_bytes_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        0,
                        "symbolic vm.toString",
                    )?),
                    toString_3Call::SELECTOR => read_abi_bool_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        0,
                        "symbolic vm.toString",
                    )?
                    .to_string(),
                    _ => {
                        let value = read_abi_concrete_word_arg(
                            &mut self.cx,
                            &state.memory,
                            args_offset,
                            0,
                            "symbolic vm.toString",
                        )?;
                        match selector {
                            toString_2Call::SELECTOR => {
                                hex::encode_prefixed(value.to_be_bytes::<32>())
                            }
                            toString_4Call::SELECTOR => value.to_string(),
                            _ => I256::from_raw(value).to_string(),
                        }
                    }
                };
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    output.as_bytes(),
                )))
            }
            envBool_0Call::SELECTOR
            | envUint_0Call::SELECTOR
            | envInt_0Call::SELECTOR
            | envAddress_0Call::SELECTOR
            | envBytes32_0Call::SELECTOR
            | envString_0Call::SELECTOR
            | envBytes_0Call::SELECTOR
            | parseBoolCall::SELECTOR
            | parseUintCall::SELECTOR
            | parseIntCall::SELECTOR
            | parseAddressCall::SELECTOR
            | parseBytes32Call::SELECTOR
            | parseBytesCall::SELECTOR => {
                let (ty, message, from_env) = match selector {
                    envBool_0Call::SELECTOR => (DynSolType::Bool, "symbolic vm.envBool", true),
                    envUint_0Call::SELECTOR => (DynSolType::Uint(256), "symbolic vm.envUint", true),
                    envInt_0Call::SELECTOR => (DynSolType::Int(256), "symbolic vm.envInt", true),
                    envAddress_0Call::SELECTOR => {
                        (DynSolType::Address, "symbolic vm.envAddress", true)
                    }
                    envBytes32_0Call::SELECTOR => {
                        (DynSolType::FixedBytes(32), "symbolic vm.envBytes32", true)
                    }
                    envString_0Call::SELECTOR => {
                        (DynSolType::String, "symbolic vm.envString", true)
                    }
                    envBytes_0Call::SELECTOR => (DynSolType::Bytes, "symbolic vm.envBytes", true),
                    parseBoolCall::SELECTOR => (DynSolType::Bool, "symbolic vm.parseBool", false),
                    parseUintCall::SELECTOR => {
                        (DynSolType::Uint(256), "symbolic vm.parseUint", false)
                    }
                    parseIntCall::SELECTOR => (DynSolType::Int(256), "symbolic vm.parseInt", false),
                    parseAddressCall::SELECTOR => {
                        (DynSolType::Address, "symbolic vm.parseAddress", false)
                    }
                    parseBytes32Call::SELECTOR => {
                        (DynSolType::FixedBytes(32), "symbolic vm.parseBytes32", false)
                    }
                    _ => (DynSolType::Bytes, "symbolic vm.parseBytes", false),
                };
                let mut value =
                    read_abi_string_arg(&mut self.cx, &state.memory, args_offset, 0, message)?;
                if from_env {
                    self.stateless_retry_safe = false;
                    value = std::env::var(value)
                        .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                }
                let value = if from_env {
                    parse_env_value(&value, &ty)?
                } else {
                    // Forge's parser accepts the same inputs and reports the same error.
                    match parse_value(&value, &ty) {
                        Ok(value) => value,
                        Err(error) => {
                            let name = message.trim_start_matches("symbolic ");
                            return Ok(CheatcodeOutcome::Revert(error_string_return_data(
                                &mut self.cx,
                                &format!("{name}: {error}"),
                            )));
                        }
                    }
                };
                Ok(match value.as_word() {
                    Some(word) => CheatcodeOutcome::Continue(vec![SymExpr::constant(
                        &mut self.cx,
                        word.into(),
                    )]),
                    None => CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                        &mut self.cx,
                        &value.abi_encode_packed(),
                    )),
                })
            }
            toLowercaseCall::SELECTOR | toUppercaseCall::SELECTOR | trimCall::SELECTOR => {
                let value = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.string",
                )?;
                let output = if selector == toLowercaseCall::SELECTOR {
                    value.to_lowercase()
                } else if selector == toUppercaseCall::SELECTOR {
                    value.to_uppercase()
                } else {
                    value.trim().to_string()
                };
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    output.as_bytes(),
                )))
            }
            replaceCall::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let output = dyn_string(&values[0])?
                    .replace(&dyn_string(&values[1])?, &dyn_string(&values[2])?);
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    output.as_bytes(),
                )))
            }
            splitCall::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let input = dyn_string(&values[0])?;
                let delimiter = dyn_string(&values[1])?;
                let parts = input
                    .split(&delimiter)
                    .map(|part| DynSolValue::String(part.to_string()))
                    .collect();
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(
                    &mut self.cx,
                    DynSolValue::Array(parts),
                )))
            }
            indexOfCall::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let input = dyn_string(&values[0])?;
                let needle = dyn_string(&values[1])?;
                let index = input.find(&needle).map(U256::from).unwrap_or(U256::MAX);
                Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(&mut self.cx, index)]))
            }
            containsCall::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let contains = dyn_string(&values[0])?.contains(&dyn_string(&values[1])?);
                Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(
                    &mut self.cx,
                    U256::from(contains),
                )]))
            }
            toBase64_0Call::SELECTOR
            | toBase64_1Call::SELECTOR
            | toBase64URL_0Call::SELECTOR
            | toBase64URL_1Call::SELECTOR => {
                let data = read_abi_dynamic_bytes_arg(
                    &mut self.cx,
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
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    encoded.as_bytes(),
                )))
            }
            envExistsCall::SELECTOR => {
                let name = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.envExists",
                )?;
                self.stateless_retry_safe = false;
                Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(
                    &mut self.cx,
                    U256::from(std::env::var_os(name).is_some()),
                )]))
            }
            envBool_1Call::SELECTOR
            | envUint_1Call::SELECTOR
            | envInt_1Call::SELECTOR
            | envAddress_1Call::SELECTOR
            | envBytes32_1Call::SELECTOR
            | envString_1Call::SELECTOR
            | envBytes_1Call::SELECTOR => {
                let name = VmCalls::name_by_selector(selector).unwrap_or_default();
                let element_ty = DynSolType::parse(&name["env".len()..].to_ascii_lowercase())
                    .map_err(|_| SymbolicError::Unsupported("symbolic env type"))?;
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let name = dyn_string(&values[0])?;
                let delimiter = dyn_string(&values[1])?;
                self.stateless_retry_safe = false;
                let value = std::env::var(name)
                    .map_err(|_| SymbolicError::Unsupported("symbolic env var missing"))?;
                let value = parse_env_array(&value, &delimiter, &element_ty)?;
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(&mut self.cx, value)))
            }
            envOr_0Call::SELECTOR => {
                let name = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.envOr",
                )?;
                self.stateless_retry_safe = false;
                // Forge treats a set-but-empty variable as missing for the typed overloads.
                let value = match std::env::var(name).ok().filter(|value| !value.is_empty()) {
                    Some(value) => U256::from(parse_env_bool(&value)?),
                    None => read_abi_concrete_word_arg(
                        &mut self.cx,
                        &state.memory,
                        args_offset,
                        1,
                        "symbolic vm.envOr",
                    )?,
                };
                Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(&mut self.cx, value)]))
            }
            envOr_1Call::SELECTOR
            | envOr_2Call::SELECTOR
            | envOr_3Call::SELECTOR
            | envOr_4Call::SELECTOR => {
                let name = read_abi_string_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    0,
                    "symbolic vm.envOr",
                )?;
                let default = read_abi_concrete_word_arg(
                    &mut self.cx,
                    &state.memory,
                    args_offset,
                    1,
                    "symbolic vm.envOr",
                )?;
                self.stateless_retry_safe = false;
                let value = match std::env::var(name).ok().filter(|value| !value.is_empty()) {
                    Some(value) if selector == envOr_1Call::SELECTOR => parse_env_uint(&value)?,
                    Some(value) if selector == envOr_2Call::SELECTOR => parse_env_int(&value)?,
                    Some(value) if selector == envOr_3Call::SELECTOR => {
                        address_word(parse_env_address(&value)?)
                    }
                    Some(value) => parse_env_bytes32(&value)?,
                    None => default,
                };
                Ok(CheatcodeOutcome::Continue(vec![SymExpr::constant(&mut self.cx, value)]))
            }
            envOr_5Call::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let name = dyn_string(&values[0])?;
                self.stateless_retry_safe = false;
                let value = std::env::var(name).unwrap_or(dyn_string(&values[1])?);
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(
                    &mut self.cx,
                    value.as_bytes(),
                )))
            }
            envOr_6Call::SELECTOR => {
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let name = dyn_string(&values[0])?;
                self.stateless_retry_safe = false;
                let value = match std::env::var(name).ok().filter(|value| !value.is_empty()) {
                    Some(value) => parse_env_bytes(&value)?,
                    None => dyn_bytes(&values[1])?,
                };
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&mut self.cx, &value)))
            }
            envOr_7Call::SELECTOR
            | envOr_8Call::SELECTOR
            | envOr_9Call::SELECTOR
            | envOr_10Call::SELECTOR
            | envOr_11Call::SELECTOR
            | envOr_12Call::SELECTOR
            | envOr_13Call::SELECTOR => {
                let params = vm_params(selector);
                let Some(DynSolType::Array(element_ty)) = params.last().cloned() else {
                    return Err(SymbolicError::Unsupported("symbolic env type"));
                };
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let name = dyn_string(&values[0])?;
                let delimiter = dyn_string(&values[1])?;
                self.stateless_retry_safe = false;
                let value = match std::env::var(name) {
                    Ok(value) => parse_env_array(&value, &delimiter, &element_ty)?,
                    Err(_) => values[2].clone(),
                };
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_value_return(&mut self.cx, value)))
            }
            ffiCall::SELECTOR => {
                if !state.ffi_enabled {
                    return Err(SymbolicError::Unsupported("symbolic ffi disabled"));
                }
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let args = dyn_string_array(&values[0])?;
                if args.is_empty() || args[0].is_empty() {
                    return Err(SymbolicError::Unsupported("symbolic ffi empty command"));
                }
                self.stateless_retry_safe = false;
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
                Ok(CheatcodeOutcome::ContinueData(abi_concrete_bytes_return(&mut self.cx, &bytes)))
            }
            assertTrue_0Call::SELECTOR | assertTrue_1Call::SELECTOR => {
                let word = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let condition = word.nonzero_bool(&mut self.cx);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertFalse_0Call::SELECTOR | assertFalse_1Call::SELECTOR => {
                let word = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let condition = word.into_zero_bool(&mut self.cx);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
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
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::eq(&mut self.cx, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertEqDecimal_0Call::SELECTOR
            | assertEqDecimal_1Call::SELECTOR
            | assertEqDecimal_2Call::SELECTOR
            | assertEqDecimal_3Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::eq(&mut self.cx, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
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
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::eq(&mut self.cx, left, right);
                let condition = condition.not(&mut self.cx);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertEq_10Call::SELECTOR
            | assertEq_11Call::SELECTOR
            | assertEq_12Call::SELECTOR
            | assertEq_13Call::SELECTOR
            | assertEq_14Call::SELECTOR
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
            | assertEq_27Call::SELECTOR
            | assertNotEq_10Call::SELECTOR
            | assertNotEq_11Call::SELECTOR
            | assertNotEq_12Call::SELECTOR
            | assertNotEq_13Call::SELECTOR
            | assertNotEq_14Call::SELECTOR
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
                let values =
                    decode_cheatcode_args(&mut self.cx, state, selector, in_offset, in_size)?;
                let expect_equal = VmCalls::name_by_selector(selector) == Some("assertEq");
                let condition =
                    SymBoolExpr::constant(&mut self.cx, (values[0] == values[1]) == expect_equal);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertLt_0Call::SELECTOR | assertLt_1Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ult, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertLe_0Call::SELECTOR | assertLe_1Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertGt_0Call::SELECTOR | assertGt_1Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ugt, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertGe_0Call::SELECTOR | assertGe_1Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Uge, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertLt_2Call::SELECTOR | assertLt_3Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Slt, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertGt_2Call::SELECTOR | assertGt_3Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Sgt, left, right);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertLe_2Call::SELECTOR | assertLe_3Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Sgt, left, right);
                let condition = condition.not(&mut self.cx);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            assertGe_2Call::SELECTOR | assertGe_3Call::SELECTOR => {
                let left = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let right = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 1)?;
                let condition = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Slt, left, right);
                let condition = condition.not(&mut self.cx);
                self.handle_assertion(executor, state, condition, selector, Some(args_offset))
            }
            randomUint_0Call::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.fresh_word(&mut self.cx, "vmRandomUint")]))
            }
            randomUint_2Call::SELECTOR => {
                let bits = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic randomUint bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic randomUint bits")?;
                Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_uint(&mut self.cx, bits)]))
            }
            randomUint_1Call::SELECTOR => {
                let min = state.memory.load_word(&mut self.cx, in_offset + 4)?;
                let max = state.memory.load_word(&mut self.cx, in_offset + 36)?;
                let value = state.fresh_word(&mut self.cx, "vmRandomUintRange");
                state.constraints.push(SymBoolExpr::cmp_word_expr(
                    &mut self.cx,
                    SymCmpOp::Uge,
                    &value,
                    min,
                ));
                state.constraints.push(SymBoolExpr::cmp_word_expr(
                    &mut self.cx,
                    SymCmpOp::Ule,
                    &value,
                    max,
                ));
                Ok(CheatcodeOutcome::Continue(vec![value]))
            }
            randomInt_0Call::SELECTOR => {
                Ok(CheatcodeOutcome::Continue(vec![state.fresh_word(&mut self.cx, "vmRandomInt")]))
            }
            randomInt_1Call::SELECTOR => {
                let bits = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic randomInt bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic randomInt bits")?;
                Ok(CheatcodeOutcome::Continue(vec![state.fresh_bounded_int(&mut self.cx, bits)]))
            }
            randomAddressCall::SELECTOR => {
                let value = state.fresh_bounded_uint(&mut self.cx, U256::from(160));
                Ok(CheatcodeOutcome::Continue(vec![value]))
            }
            randomBoolCall::SELECTOR => {
                let value = state.fresh_bounded_uint(&mut self.cx, U256::ONE);
                Ok(CheatcodeOutcome::Continue(vec![value]))
            }
            randomBytesCall::SELECTOR => {
                let len = read_abi_word_arg(&mut self.cx, &state.memory, args_offset, 0)?;
                let max_limit = self.config.max_dynamic_length as usize;
                let max_len = self.solver_upper_bound_usize(
                    state,
                    &len,
                    max_limit,
                    "symbolic randomBytes length",
                )?;
                let bytes = state.fresh_bytes(&mut self.cx, max_len);
                Ok(CheatcodeOutcome::ContinueData(abi_bytes_return_with_len(
                    &mut self.cx,
                    len,
                    bytes,
                )))
            }
            randomBytes4Call::SELECTOR => {
                let value = state.fresh_bounded_uint(&mut self.cx, U256::from(32));
                Ok(CheatcodeOutcome::Continue(vec![shift_left(&mut self.cx, value, 224)]))
            }
            randomBytes8Call::SELECTOR => {
                let value = state.fresh_bounded_uint(&mut self.cx, U256::from(64));
                Ok(CheatcodeOutcome::Continue(vec![shift_left(&mut self.cx, value, 192)]))
            }

            _ => Err(SymbolicError::Unsupported("symbolic Foundry cheatcode")),
        }
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
                    state.fresh_word(&mut self.cx, "svm")
                } else {
                    state.fresh_bounded_uint(&mut self.cx, U256::from(bits))
                };
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateIntBits(bits) => {
                let value = if bits == 256 {
                    state.fresh_word(&mut self.cx, "svm")
                } else {
                    state.fresh_bounded_int(&mut self.cx, U256::from(bits))
                };
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateBytesFixed(bytes) => {
                let value = if bytes == 32 {
                    state.fresh_word(&mut self.cx, "svm")
                } else {
                    let value = state.fresh_bounded_uint(&mut self.cx, U256::from(bytes * 8));
                    shift_left(&mut self.cx, value, (32 - bytes) * 8)
                };
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateUint => {
                let bits = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic svm.create integer bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic svm.create integer bits")?;
                let value = state.fresh_bounded_uint(&mut self.cx, bits);
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateInt => {
                let bits = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic svm.create integer bits",
                )?;
                Self::validate_symbolic_integer_bits(bits, "symbolic svm.create integer bits")?;
                let value = state.fresh_bounded_int(&mut self.cx, bits);
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateAddress => {
                let value = state.fresh_bounded_uint(&mut self.cx, U256::from(160));
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateBool => {
                let value = state.fresh_bounded_uint(&mut self.cx, U256::ONE);
                Ok(SymReturnData::from_words(&mut self.cx, vec![value]))
            }
            SymbolicVmCheatcode::CreateBytes => {
                let bytes =
                    state.fresh_bytes(&mut self.cx, self.config.default_dynamic_length as usize);
                Ok(abi_bytes_return(&mut self.cx, bytes))
            }
            SymbolicVmCheatcode::CreateBytesSized => {
                let len = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic svm.createBytes length",
                )?;
                let len = usize::try_from(len)
                    .ok()
                    .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                    .ok_or(SymbolicError::Unsupported("symbolic svm.createBytes length"))?;
                let bytes = state.fresh_bytes(&mut self.cx, len);
                Ok(abi_bytes_return(&mut self.cx, bytes))
            }
            SymbolicVmCheatcode::CreateString => {
                let bytes = state.fresh_printable_ascii_bytes(
                    &mut self.cx,
                    self.config.default_dynamic_length as usize,
                );
                Ok(abi_bytes_return(&mut self.cx, bytes))
            }
            SymbolicVmCheatcode::CreateStringSized => {
                let len = read_abi_constrained_word_arg(
                    &mut self.cx,
                    state,
                    args_offset,
                    0,
                    "symbolic svm.createString length",
                )?;
                let len = usize::try_from(len)
                    .ok()
                    .filter(|len| *len <= self.config.max_calldata_bytes as usize)
                    .ok_or(SymbolicError::Unsupported("symbolic svm.createString length"))?;
                let bytes = state.fresh_printable_ascii_bytes(&mut self.cx, len);
                Ok(abi_bytes_return(&mut self.cx, bytes))
            }
            SymbolicVmCheatcode::CreateCalldata => {
                let max = self.config.max_calldata_bytes as usize;
                let len = if max < 4 {
                    max
                } else {
                    (self.config.default_dynamic_length as usize).max(4).min(max)
                };
                let bytes = state.fresh_bytes(&mut self.cx, len);
                Ok(abi_bytes_return(&mut self.cx, bytes))
            }
            SymbolicVmCheatcode::EnableSymbolicStorage => {
                let target =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                state.world.enable_arbitrary_storage(target, false);
                Ok(SymReturnData::empty(&mut self.cx))
            }
            SymbolicVmCheatcode::SnapshotStorage => {
                let _target =
                    read_abi_address_or_symbolic_slot_arg(&mut self.cx, state, args_offset, 0)?;
                let id = state.world.snapshot_state(&state.block, &state.gas_price);
                let id = SymExpr::constant(&mut self.cx, id);
                Ok(SymReturnData::from_words(&mut self.cx, vec![id]))
            }
            SymbolicVmCheatcode::SnapshotState => {
                let id = state.world.snapshot_state(&state.block, &state.gas_price);
                let id = SymExpr::constant(&mut self.cx, id);
                Ok(SymReturnData::from_words(&mut self.cx, vec![id]))
            }
        }
    }

    /// Forks off the paths where `invalid` holds, on which the cheatcode call reverts with
    /// `revert_data`. Returns whether every path reverts, in which case `state` is the reverted
    /// path; otherwise `state` continues where `invalid` does not hold.
    pub(super) fn fork_cheatcode_revert(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        invalid: SymBoolExpr,
        out_offset: &SymExpr,
        out_size: &BoundedCopySize,
        revert_data: impl FnOnce(&mut Self, &PathState) -> SymReturnData,
    ) -> Result<bool, SymbolicError> {
        let (invalid_constraints, invalid_sat) =
            self.constraints_with_condition(state, invalid.clone())?;
        if !invalid_sat {
            return Ok(false);
        }
        let valid = invalid.not(&mut self.cx);
        if let Some(valid_constraints) = self.constraints_for_condition(state, valid)? {
            let mut revert_state = state.clone();
            revert_state.constraints = invalid_constraints;
            let return_data = revert_data(self, &revert_state);
            revert_cheatcode_call(
                &mut self.cx,
                &mut revert_state,
                out_offset.clone(),
                out_size,
                return_data,
            )?;
            worklist.push_back(revert_state);
            state.constraints = valid_constraints;
            return Ok(false);
        }
        let return_data = revert_data(self, state);
        revert_cheatcode_call(&mut self.cx, state, out_offset.clone(), out_size, return_data)?;
        Ok(true)
    }
}

/// Bounds the concrete accounts a cheatcode write through a free symbolic address forks over.
const MAX_FREE_ADDRESS_ALIAS_CANDIDATES: usize = 32;
