use super::*;

enum MappingStorageProvenance {
    None,
    Exact(SymbolicMappingProvenance),
    Fork { equality: Vec<SymBoolExpr>, inequality: Vec<SymBoolExpr> },
}

impl SymbolicExecutor {
    fn classify_mapping_match(
        &mut self,
        state: &PathState,
        matches: SymBoolExpr,
        provenance: SymbolicMappingProvenance,
    ) -> Result<Option<MappingStorageProvenance>, SymbolicError> {
        let does_not_match = matches.clone().not(&mut self.cx);
        let (inequality, inequality_is_sat) =
            self.constraints_with_condition(state, does_not_match)?;
        if !inequality_is_sat {
            return Ok(Some(MappingStorageProvenance::Exact(provenance)));
        }
        let (equality, equality_is_sat) = self.constraints_with_condition(state, matches)?;
        Ok(equality_is_sat.then_some(MappingStorageProvenance::Fork { equality, inequality }))
    }

    fn observed_mapping_chain(
        &mut self,
        state: &PathState,
        hash: &SymExpr,
    ) -> Option<(SymExpr, Vec<SymExpr>)> {
        let account = state.storage_address;
        let mut current = hash.clone();
        let mut keys = Vec::new();
        let mut visited = Vec::new();
        loop {
            if visited.contains(&current) {
                return None;
            }
            visited.push(current.clone());
            let Some(bytes) =
                state.mapping_hook_keccak_preimages.get(&(account, current.clone())).cloned()
            else {
                keys.reverse();
                return Some((current, keys));
            };
            if bytes.len() != 64 {
                return None;
            }
            keys.push(SymExpr::from_bytes(&mut self.cx, bytes[..32].iter().cloned()));
            current = SymExpr::from_bytes(&mut self.cx, bytes[32..64].iter().cloned());
        }
    }

    fn mapping_storage_provenance(
        &mut self,
        state: &PathState,
        key: &SymExpr,
    ) -> Result<MappingStorageProvenance, SymbolicError> {
        let account = state.storage_address;
        let observed = |hash: &SymExpr| {
            state.mapping_hook_keccak_preimages.get(&(account, hash.clone())).cloned()
        };
        if let Some(provenance) =
            key.storage_mapping_provenance_observed_with(&mut self.cx, observed)
        {
            return Ok(MappingStorageProvenance::Exact(provenance));
        }
        let mut hashes = state
            .mapping_hook_keccak_preimages
            .keys()
            .filter(|(address, _)| *address == account)
            .map(|(_, hash)| hash.clone())
            .collect::<Vec<_>>();
        let contains_observed_hash =
            hashes.iter().any(|hash| key.visit_bool(|candidate| candidate == hash));
        let key_is_const = key.as_const().is_some();
        let key_contains_keccak = key.contains_keccak();
        let key_is_storage_mapping_key = key.storage_mapping_key(&mut self.cx).is_some();
        let roots = state
            .mapping_storage_store_hooks
            .keys()
            .filter(|(address, _)| *address == account)
            .map(|(_, root)| *root)
            .collect::<Vec<_>>();
        hashes.sort_by_key(|hash| hash != key);
        for hash in hashes {
            if contains_observed_hash && !key.visit_bool(|candidate| candidate == &hash) {
                continue;
            }
            let use_legacy_match = contains_observed_hash || !key_contains_keccak;
            if use_legacy_match
                && let Some(provenance) =
                    hash.storage_mapping_provenance_observed_with(&mut self.cx, |candidate| {
                        state
                            .mapping_hook_keccak_preimages
                            .get(&(account, candidate.clone()))
                            .cloned()
                    })
            {
                if !state.mapping_storage_store_hooks.contains_key(&(account, provenance.root_slot))
                {
                    continue;
                }
                let equality = SymBoolExpr::eq(&mut self.cx, key.clone(), hash.clone());
                if key_is_const {
                    let inequality = equality.not(&mut self.cx);
                    let (_, inequality_is_sat) =
                        self.constraints_with_condition(state, inequality)?;
                    if !inequality_is_sat {
                        return Ok(MappingStorageProvenance::Exact(provenance));
                    }
                    continue;
                }
                if let Some(provenance) =
                    self.classify_mapping_match(state, equality, provenance)?
                {
                    return Ok(provenance);
                }
                continue;
            }
            if (key_is_const || key_contains_keccak) && !key_is_storage_mapping_key {
                continue;
            }
            let Some((root, keys)) = self.observed_mapping_chain(state, &hash) else {
                continue;
            };
            for &root_slot in &roots {
                let slot_matches = if key_is_const || key.visit_bool(|candidate| candidate == &hash)
                {
                    SymBoolExpr::eq(&mut self.cx, key.clone(), hash.clone())
                } else {
                    key.storage_key_eq(&mut self.cx, &hash)
                };
                let matches = if let Some(constrained_root) =
                    state.constrained_word(&mut self.cx, &root)
                {
                    if constrained_root != root_slot {
                        continue;
                    }
                    slot_matches
                } else {
                    let root_slot_expr = SymExpr::constant(&mut self.cx, root_slot);
                    let root_matches = SymBoolExpr::eq(&mut self.cx, root.clone(), root_slot_expr);
                    SymBoolExpr::and(&mut self.cx, vec![slot_matches, root_matches])
                };
                let provenance = SymbolicMappingProvenance { root_slot, keys: keys.clone() };
                if let Some(provenance) = self.classify_mapping_match(state, matches, provenance)? {
                    return Ok(provenance);
                }
            }
        }
        Ok(MappingStorageProvenance::None)
    }

    fn storage_hook_calldata(
        &mut self,
        selector: [u8; 4],
        words: impl IntoIterator<Item = SymExpr>,
    ) -> SymCalldata {
        let selector = SymBytes::concrete(&mut self.cx, selector.to_vec());
        let words = words.into_iter().map(|word| word.into_bytes(&mut self.cx)).collect::<Vec<_>>();
        let bytes = SymBytes::concat(&mut self.cx, std::iter::once(selector).chain(words));
        SymCalldata::from_bytes(&mut self.cx, bytes)
    }

    fn mapping_storage_hook_calldata(
        &mut self,
        selector: [u8; 4],
        [account, computed_slot, root_slot, old_value, new_value]: [SymExpr; 5],
        keys: Vec<SymExpr>,
    ) -> SymCalldata {
        let selector = SymBytes::concrete(&mut self.cx, selector.to_vec());
        let keys_offset = SymExpr::constant(&mut self.cx, U256::from(6 * 32));
        let mut words = vec![account, computed_slot, root_slot, keys_offset, old_value, new_value];
        words.push(SymExpr::constant(&mut self.cx, U256::from(keys.len())));
        words.extend(keys);
        let words = words.into_iter().map(|word| word.into_bytes(&mut self.cx)).collect::<Vec<_>>();
        let bytes = SymBytes::concat(&mut self.cx, std::iter::once(selector).chain(words));
        SymCalldata::from_bytes(&mut self.cx, bytes)
    }

    fn invoke_storage_hook<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        hook: SymbolicStorageHook,
        calldata: SymCalldata,
    ) -> Result<StepOutcome, SymbolicError> {
        let code = state.world.extcode(&mut self.cx, executor, hook.callback_target)?;
        if code.is_empty() {
            return Ok(StepOutcome::Continue);
        }

        let callvalue = SymExpr::zero(&mut self.cx);
        let frame = CallFrame::new(
            &mut self.cx,
            hook.callback_target,
            hook.callback_target,
            CHEATCODE_ADDRESS,
            callvalue,
            false,
            calldata,
        );
        let child = state.storage_hook_child(frame);
        let outcomes = self.execute_external_call(executor, child, &code, completed_paths)?;
        if outcomes.is_empty() {
            return Ok(StepOutcome::AssumeRejected);
        }

        let mut parents = VecDeque::with_capacity(outcomes.len());
        for mut outcome in outcomes {
            let mut parent = state.clone();
            parent.constraints = std::mem::take(&mut outcome.state.constraints);
            parent.next_symbol = outcome.state.next_symbol;
            parent.storage_load_hooks = std::mem::take(&mut outcome.state.storage_load_hooks);
            parent.storage_store_hooks = std::mem::take(&mut outcome.state.storage_store_hooks);
            parent.mapping_storage_store_hooks =
                std::mem::take(&mut outcome.state.mapping_storage_store_hooks);
            parent.mapping_hook_keccak_preimages =
                std::mem::take(&mut outcome.state.mapping_hook_keccak_preimages);
            parent.storage_hook_active = false;

            match outcome.status {
                CallStatus::Success => {
                    parent.world = outcome.state.world;
                    parent.block = outcome.state.block;
                }
                CallStatus::Revert | CallStatus::Failure => {
                    parent.return_data = outcome.state.frame.return_data;
                    parent.pending_storage_hook_revert = true;
                }
            }
            parents.push_back(parent);
        }

        let Some(first) = self.pop_next_path(&mut parents) else {
            return Ok(StepOutcome::AssumeRejected);
        };
        *state = first;
        worklist.extend(parents);
        Ok(if std::mem::take(&mut state.pending_storage_hook_revert) {
            StepOutcome::Revert
        } else {
            StepOutcome::Continue
        })
    }

    fn push_comparison_result(
        &mut self,
        state: &mut PathState,
        op_pc: usize,
        opcode: u8,
        condition: SymBoolExpr,
    ) -> Result<StepOutcome, SymbolicError> {
        if !self.apply_branch_target_constraint(state, op_pc, opcode, &condition)? {
            return Ok(StepOutcome::AssumeRejected);
        }
        let value = SymExpr::bool_word(&mut self.cx, condition);
        state.stack.push(value)?;
        Ok(StepOutcome::Continue)
    }

    fn apply_branch_target_constraint(
        &mut self,
        state: &mut PathState,
        op_pc: usize,
        opcode: u8,
        condition: &SymBoolExpr,
    ) -> Result<bool, SymbolicError> {
        let Some(target) = state.branch_target() else {
            return Ok(true);
        };
        if state.satisfies_branch_target() {
            return Ok(true);
        }
        if !target.matches(state.address, op_pc, opcode) {
            return Ok(true);
        }

        let desired =
            if target.result() { condition.clone().not(&mut self.cx) } else { condition.clone() };
        let mut constraints = state.constraints.clone();
        constraints.push(desired);
        if !self.branch_is_sat_or_defer(state, &constraints)? {
            return Ok(false);
        }
        state.constraints = constraints;
        state.mark_branch_target_reached();
        Ok(true)
    }

    fn guard_fixed_memory_access<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        offset: &SymExpr,
        size: usize,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let memory_limit = executor.evm_env().cfg_env.memory_limit();
        let host_max_offset = (usize::MAX & !31usize).checked_sub(size);
        let constrained_offset = state.constrained_usize_checked(&mut self.cx, offset);
        if constrained_offset.as_ref().is_some_and(|offset| match offset {
            Ok(offset) => host_max_offset.is_none_or(|max| *offset > max),
            Err(_) => true,
        }) {
            state.return_data = SymReturnData::empty(&mut self.cx);
            return Ok(Some(StepOutcome::Revert));
        }

        let expanded_size_bound = state
            .upper_bound_usize(&mut self.cx, offset)
            .and_then(|offset| offset.checked_add(size))
            .and_then(|end| end.checked_add(31))
            .and_then(|end| u64::try_from(end & !31usize).ok());
        if expanded_size_bound.is_some_and(|size| size <= memory_limit) {
            return Ok(None);
        }

        let representable = if let Some(host_max_offset) = host_max_offset {
            let host_max_offset = SymExpr::constant(&mut self.cx, U256::from(host_max_offset));
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, offset.clone(), host_max_offset)
        } else {
            SymBoolExpr::constant(&mut self.cx, false)
        };
        let size = SymExpr::constant(&mut self.cx, U256::from(size));
        let local_size =
            state.memory.size_after_range_expansion_word(&mut self.cx, offset.clone(), size);
        if let Some(local_size) = local_size.as_const() {
            if local_size <= U256::from(memory_limit) {
                return Ok(None);
            }
            state.return_data = SymReturnData::empty(&mut self.cx);
            return Ok(Some(StepOutcome::Revert));
        }
        let memory_limit = SymExpr::constant(&mut self.cx, U256::from(memory_limit));
        let within_limit = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, local_size, memory_limit);
        let valid_access = SymBoolExpr::and(&mut self.cx, vec![representable, within_limit]);
        self.apply_memory_access_guard(state, worklist, valid_access)
    }

    fn apply_memory_access_guard(
        &mut self,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        valid_access: SymBoolExpr,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let (valid_constraints, valid_sat) =
            self.constraints_with_condition(state, valid_access.clone())?;
        let invalid = valid_access.clone().not(&mut self.cx);
        let (invalid_constraints, invalid_sat) = self.constraints_with_condition(state, invalid)?;
        match (valid_sat, invalid_sat) {
            (true, true) => {
                let (valid_seed_models, invalid_seed_models) =
                    state.split_corpus_seed_models(&valid_access);
                let mut valid = state.clone();
                valid.pc = valid.pc.saturating_sub(1);
                valid.depth = valid.depth.saturating_sub(1);
                valid.constraints = valid_constraints;
                valid.set_corpus_seed_models(valid_seed_models);
                worklist.push_back(valid);
                state.constraints = invalid_constraints;
                state.set_corpus_seed_models(invalid_seed_models);
                state.return_data = SymReturnData::empty(&mut self.cx);
                Ok(Some(StepOutcome::Revert))
            }
            (true, false) => {
                state.constraints = valid_constraints;
                Ok(None)
            }
            (false, true) => {
                state.constraints = invalid_constraints;
                state.return_data = SymReturnData::empty(&mut self.cx);
                Ok(Some(StepOutcome::Revert))
            }
            (false, false) => Ok(Some(StepOutcome::AssumeRejected)),
        }
    }

    pub(super) fn guard_memory_range<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        offset: &SymExpr,
        size: &SymExpr,
    ) -> Result<Option<StepOutcome>, SymbolicError> {
        let memory_limit = executor.evm_env().cfg_env.memory_limit();
        if let (Some(offset_value), Some(size_value)) = (offset.as_const(), size.as_const()) {
            let valid = size_value.is_zero()
                || usize::try_from(offset_value)
                    .ok()
                    .zip(usize::try_from(size_value).ok())
                    .and_then(|(offset, size)| offset.checked_add(size))
                    .and_then(|end| end.checked_add(31))
                    .and_then(|end| u64::try_from(end & !31usize).ok())
                    .is_some_and(|end| end <= memory_limit);
            if !valid {
                state.return_data = SymReturnData::empty(&mut self.cx);
                return Ok(Some(StepOutcome::Revert));
            }
            state.memory.expand_range(&mut self.cx, offset.clone(), size.clone());
            return Ok(None);
        }

        let offset_bound = state.upper_bound_usize(&mut self.cx, offset);
        let size_bound = state.upper_bound_usize(&mut self.cx, size);
        if offset_bound
            .zip(size_bound)
            .and_then(|(offset, size)| offset.checked_add(size))
            .and_then(|end| end.checked_add(31))
            .and_then(|end| u64::try_from(end & !31usize).ok())
            .is_some_and(|end| end <= memory_limit)
        {
            state.memory.expand_range(&mut self.cx, offset.clone(), size.clone());
            return Ok(None);
        }

        let zero_size = SymBoolExpr::eq_word_const(&mut self.cx, size, U256::ZERO);
        let host_max = SymExpr::constant(&mut self.cx, U256::from(usize::MAX & !31usize));
        let size_fits =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, size.clone(), host_max.clone());
        let max_offset = SymExpr::binop(&mut self.cx, SymBinOp::Sub, host_max, size.clone());
        let offset_fits = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, offset.clone(), max_offset);

        let local_size = state.memory.size_after_range_expansion_word(
            &mut self.cx,
            offset.clone(),
            size.clone(),
        );
        let memory_limit = SymExpr::constant(&mut self.cx, U256::from(memory_limit));
        let local_fits = SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, local_size, memory_limit);
        let nonzero_valid =
            SymBoolExpr::and(&mut self.cx, vec![size_fits, offset_fits, local_fits]);
        let valid_access = SymBoolExpr::or(&mut self.cx, vec![zero_size, nonzero_valid]);

        let outcome = self.apply_memory_access_guard(state, worklist, valid_access)?;
        if outcome.is_none() {
            state.memory.expand_range(&mut self.cx, offset.clone(), size.clone());
        }
        Ok(outcome)
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn step<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        jumpdests: &JumpTable,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        op: u8,
    ) -> Result<StepOutcome, SymbolicError> {
        state.pc += 1;

        match op {
            opcode::PUSH0 => {
                state.stack.push(SymExpr::zero(&mut self.cx))?;
            }
            opcode::PUSH1..=opcode::PUSH32 => {
                let n = (op - opcode::PUSH1 + 1) as usize;
                let end = state.pc.saturating_add(n);
                if end > code.len() {
                    return Err(SymbolicError::InvalidBytecode("truncated PUSH data"));
                }
                let value = code.push_data_word(&mut self.cx, state.pc, n);
                state.pc = end;
                state.stack.push(value)?;
            }
            opcode::DUP1..=opcode::DUP16 => {
                let n = (op - opcode::DUP1 + 1) as usize;
                let value = state.stack.peek(n - 1)?.clone();
                state.stack.push(value)?;
            }
            opcode::SWAP1..=opcode::SWAP16 => {
                let n = (op - opcode::SWAP1 + 1) as usize;
                state.stack.swap(n)?;
            }
            opcode::STOP => return Ok(StepOutcome::Halt),
            opcode::ADD => {
                state.bin_word(&mut self.cx, SymBinOp::Add)?;
            }
            opcode::SUB => {
                state.bin_word(&mut self.cx, SymBinOp::Sub)?;
            }
            opcode::MUL => {
                state.bin_word(&mut self.cx, SymBinOp::Mul)?;
            }
            opcode::EXP => {
                state.exp_word(&mut self.cx)?;
            }
            opcode::DIV => {
                state.bin_word_div_zero_guard(&mut self.cx, SymBinOp::UDiv)?;
            }
            opcode::SDIV => {
                state.bin_word_div_zero_guard(&mut self.cx, SymBinOp::SDiv)?;
            }
            opcode::MOD => {
                state.bin_word_div_zero_guard(&mut self.cx, SymBinOp::URem)?;
            }
            opcode::SMOD => {
                state.bin_word_div_zero_guard(&mut self.cx, SymBinOp::SRem)?;
            }
            opcode::ADDMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                state.stack.push(SymExpr::ternop(&mut self.cx, SymTernOp::AddMod, a, b, n))?;
            }
            opcode::MULMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                state.stack.push(SymExpr::ternop(&mut self.cx, SymTernOp::MulMod, a, b, n))?;
            }
            opcode::LT => {
                let op_pc = state.pc - 1;
                let condition = state.cmp_word_condition(&mut self.cx, SymCmpOp::Ult)?;
                return self.push_comparison_result(state, op_pc, op, condition);
            }
            opcode::GT => {
                let op_pc = state.pc - 1;
                let condition = state.cmp_word_condition(&mut self.cx, SymCmpOp::Ugt)?;
                return self.push_comparison_result(state, op_pc, op, condition);
            }
            opcode::SLT => {
                let op_pc = state.pc - 1;
                let condition = state.cmp_word_condition(&mut self.cx, SymCmpOp::Slt)?;
                return self.push_comparison_result(state, op_pc, op, condition);
            }
            opcode::SGT => {
                let op_pc = state.pc - 1;
                let condition = state.cmp_word_condition(&mut self.cx, SymCmpOp::Sgt)?;
                return self.push_comparison_result(state, op_pc, op, condition);
            }
            opcode::EQ => {
                let op_pc = state.pc - 1;
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let condition = SymBoolExpr::eq(&mut self.cx, b, a);
                return self.push_comparison_result(state, op_pc, op, condition);
            }
            opcode::ISZERO => {
                let op_pc = state.pc - 1;
                let value = state.stack.pop()?;
                let value = value.into_zero_bool(&mut self.cx);
                return self.push_comparison_result(state, op_pc, op, value);
            }
            opcode::AND => {
                state.bin_word(&mut self.cx, SymBinOp::And)?;
            }
            opcode::OR => {
                state.bin_word(&mut self.cx, SymBinOp::Or)?;
            }
            opcode::XOR => {
                state.bin_word(&mut self.cx, SymBinOp::Xor)?;
            }
            opcode::NOT => {
                let value = state.stack.pop()?;
                state.stack.push(SymExpr::not(&mut self.cx, value))?;
            }
            opcode::SIGNEXTEND => {
                let byte_index = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.stack.push(signextend_word_dynamic(&mut self.cx, byte_index, value))?;
            }
            opcode::BYTE => {
                let index = state.stack.pop()?;
                let word = state.stack.pop()?;
                state.stack.push(byte_word_dynamic(&mut self.cx, index, word))?;
            }
            opcode::SHL => {
                state.shift_word(&mut self.cx, ShiftKind::Shl)?;
            }
            opcode::SHR => {
                state.shift_word(&mut self.cx, ShiftKind::Shr)?;
            }
            opcode::SAR => {
                state.shift_word(&mut self.cx, ShiftKind::Sar)?;
            }
            opcode::KECCAK256 => {
                let offset = state.stack.peek(0)?.clone();
                let size = state.stack.peek(1)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &offset, &size)?
                {
                    return Ok(outcome);
                }
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        let bytes = state.memory.read_byte_exprs_offset(&mut self.cx, offset, size);
                        let hash = keccak_word(&mut self.cx, bytes.clone());
                        let has_mapping_hook = state
                            .mapping_storage_store_hooks
                            .keys()
                            .any(|(address, _)| *address == state.storage_address);
                        if has_mapping_hook && !state.storage_hook_active && size == 64 {
                            state
                                .mapping_hook_keccak_preimages
                                .entry((state.storage_address, hash.clone()))
                                .or_insert_with(|| bytes.into());
                        }
                        state.stack.push(hash)?;
                    }
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let has_mapping_hook = state
                            .mapping_storage_store_hooks
                            .keys()
                            .any(|(address, _)| *address == state.storage_address);
                        if has_mapping_hook && !state.storage_hook_active {
                            let mapping_size = SymExpr::constant(&mut self.cx, U256::from(64));
                            let mapping_size_feasible =
                                SymBoolExpr::eq(&mut self.cx, size.clone(), mapping_size);
                            let (_, mapping_size_feasible) =
                                self.constraints_with_condition(state, mapping_size_feasible)?;
                            if mapping_size_feasible {
                                self.defer_incomplete(
                                    "symbolic KECCAK256 size may conceal mapping provenance",
                                );
                            }
                        }
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic SHA3 size",
                                )
                            })?;
                        let bytes = state.memory.read_byte_exprs_symbolic_size(
                            &mut self.cx,
                            offset,
                            size.clone(),
                            max_size,
                        );
                        state.stack.push(keccak_word_with_len(&mut self.cx, bytes, size))?;
                    }
                }
            }
            opcode::ADDRESS => {
                let address = state.address_word.clone();
                state.stack.push(address)?;
            }
            opcode::CALLER => {
                let caller = state.caller_word.clone();
                state.stack.push(caller)?;
            }
            opcode::ORIGIN => {
                let origin = state.origin_word.clone();
                state.stack.push(origin)?;
            }
            opcode::CALLVALUE => {
                let callvalue = state.callvalue.clone();
                state.stack.push(callvalue)?;
            }
            opcode::BLOCKHASH => {
                let number = state.stack.pop()?;
                let hash = state.block.block_hash_word(&mut self.cx, executor, number)?;
                state.stack.push(hash)?;
            }
            opcode::BALANCE => {
                let target = state.stack.pop()?;
                let balance = state.balance_word(&mut self.cx, executor, target)?;
                state.stack.push(balance)?;
            }
            opcode::SELFBALANCE => {
                let balance = state.balance(&mut self.cx, executor, state.address);
                state.stack.push(balance)?;
            }
            opcode::EXTCODESIZE => {
                let target = state.stack.pop()?;
                let size = state.extcode_size_word(&mut self.cx, executor, target)?;
                state.stack.push(size)?;
            }
            opcode::EXTCODEHASH => {
                let target = state.stack.pop()?;
                let hash = state.extcode_hash_word(&mut self.cx, executor, target)?;
                state.stack.push(hash)?;
            }
            opcode::EXTCODECOPY => {
                let dest = state.stack.peek(1)?.clone();
                let size = state.stack.peek(3)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &dest, &size)?
                {
                    return Ok(outcome);
                }
                let target = state.stack.pop()?;
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        let bytes = state.extcode_bytes_word(
                            &mut self.cx,
                            executor,
                            target,
                            offset,
                            size,
                        )?;
                        state.memory.copy_bytes_offset(&mut self.cx, dest, bytes);
                    }
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic EXTCODECOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let bytes = state.extcode_bytes_word(
                                &mut self.cx,
                                executor,
                                target,
                                offset,
                                max_size,
                            )?;
                            state.memory.copy_bytes_size_offset(&mut self.cx, dest, size, bytes)?;
                        }
                    }
                }
            }
            opcode::CALLDATALOAD => {
                let offset = state.stack.pop()?;
                let value = state.calldata.load_word(&mut self.cx, offset)?;
                state.stack.push(value)?;
            }
            opcode::CALLDATASIZE => {
                let size = state.calldata.size_word();
                state.stack.push(size)?;
            }
            opcode::CALLDATACOPY => {
                let dest = state.stack.peek(0)?.clone();
                let size = state.stack.peek(2)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &dest, &size)?
                {
                    return Ok(outcome);
                }
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        if size != 0 {
                            state.copy_calldata_to_offset(&mut self.cx, dest, offset, size)?;
                        }
                    }
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic CALLDATACOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            state.copy_calldata_symbolic_size(
                                &mut self.cx,
                                dest,
                                offset,
                                size,
                                max_size,
                            )?;
                        }
                    }
                }
            }
            opcode::CODESIZE => {
                let value = SymExpr::constant(&mut self.cx, U256::from(code.len()));
                state.stack.push(value)?;
            }
            opcode::CODECOPY => {
                let dest = state.stack.peek(0)?.clone();
                let size = state.stack.peek(2)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &dest, &size)?
                {
                    return Ok(outcome);
                }
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        let bytes = code.read_bytes_offset(&mut self.cx, offset, size);
                        state.memory.copy_bytes_offset(&mut self.cx, dest, bytes);
                    }
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic CODECOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            let bytes = code.read_bytes_offset(&mut self.cx, offset, max_size);
                            state.memory.copy_bytes_size_offset(&mut self.cx, dest, size, bytes)?;
                        }
                    }
                }
            }
            opcode::RETURNDATASIZE => {
                let size = state.return_data.len_word();
                state.stack.push(size)?;
            }
            opcode::RETURNDATACOPY => {
                let dest = state.stack.peek(0)?.clone();
                let size = state.stack.peek(2)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &dest, &size)?
                {
                    return Ok(outcome);
                }
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        let size_word = SymExpr::constant(&mut self.cx, U256::from(size));
                        if !self.assume_returndata_copy_in_bounds(
                            state,
                            offset.clone(),
                            size_word,
                        )? {
                            return Ok(StepOutcome::Revert);
                        }
                        state.copy_return_data_to_offset(&mut self.cx, dest, offset, size)?;
                    }
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let available = state
                            .constrained_usize(&mut self.cx, &offset)
                            .map(|offset| state.return_data.len().saturating_sub(offset))
                            .unwrap_or(state.return_data.len());
                        let max_limit = available.min(self.config.max_calldata_bytes as usize);
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic RETURNDATACOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            if !self.assume_returndata_copy_in_bounds(
                                state,
                                offset.clone(),
                                size.clone(),
                            )? {
                                return Ok(StepOutcome::Revert);
                            }
                            state.copy_return_data_symbolic_size(
                                &mut self.cx,
                                dest,
                                offset,
                                size,
                                max_size,
                            )?;
                        }
                    }
                }
            }
            opcode::POP => {
                state.stack.pop()?;
            }
            opcode::MLOAD => {
                let offset = state.stack.peek(0)?.clone();
                if let Some(outcome) =
                    self.guard_fixed_memory_access(executor, state, worklist, &offset, 32)?
                {
                    return Ok(outcome);
                }
                let offset = state.stack.pop()?;
                let value = state.memory.load_word_offset(&mut self.cx, offset)?;
                state.stack.push(value)?;
            }
            opcode::MSTORE => {
                let offset = state.stack.peek(0)?.clone();
                if let Some(outcome) =
                    self.guard_fixed_memory_access(executor, state, worklist, &offset, 32)?
                {
                    return Ok(outcome);
                }
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_word_offset(&mut self.cx, offset, value);
            }
            opcode::MSTORE8 => {
                let offset = state.stack.peek(0)?.clone();
                if let Some(outcome) =
                    self.guard_fixed_memory_access(executor, state, worklist, &offset, 1)?
                {
                    return Ok(outcome);
                }
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_byte_offset(&mut self.cx, offset, value);
            }
            opcode::SLOAD => {
                let key = state.stack.pop()?;
                state.record_sload(state.storage_address, key.clone());
                let concrete_key = state.constrained_word(&mut self.cx, &key);
                let value = state.world.sload(
                    &mut self.cx,
                    executor,
                    state.storage_address,
                    key.clone(),
                    concrete_key,
                )?;
                state.stack.push(value.clone())?;
                if !state.storage_hook_active
                    && let Some(hook) =
                        state.storage_load_hooks.get(&state.storage_address).copied()
                {
                    let account =
                        SymExpr::constant(&mut self.cx, address_word(state.storage_address));
                    let calldata =
                        self.storage_hook_calldata(hook.callback_selector, [account, key, value]);
                    return self.invoke_storage_hook(
                        executor,
                        state,
                        worklist,
                        completed_paths,
                        hook,
                        calldata,
                    );
                }
            }
            opcode::SSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.peek(0)?.clone();
                state.stack.peek(1)?;
                let hook = (!state.storage_hook_active)
                    .then(|| state.storage_store_hooks.get(&state.storage_address).copied())
                    .flatten();
                let mapping = if state.storage_hook_active {
                    MappingStorageProvenance::None
                } else {
                    self.mapping_storage_provenance(state, &key)?
                };
                let mapping = match mapping {
                    MappingStorageProvenance::None => None,
                    MappingStorageProvenance::Exact(provenance) => state
                        .mapping_storage_store_hooks
                        .get(&(state.storage_address, provenance.root_slot))
                        .copied()
                        .map(|hook| (hook, provenance)),
                    MappingStorageProvenance::Fork { equality, inequality } => {
                        let store_pc = state.pc - 1;
                        let mut equality_state = state.clone();
                        equality_state.pc = store_pc;
                        equality_state.constraints = equality;
                        let mut inequality_state = state.clone();
                        inequality_state.pc = store_pc;
                        inequality_state.constraints = inequality;
                        worklist.push_back(equality_state);
                        worklist.push_back(inequality_state);
                        return Ok(StepOutcome::Forked);
                    }
                };
                state.stack.pop()?;
                let value = state.stack.pop()?;
                state.record_sstore(state.storage_address, key.clone());
                let old_value = if hook.is_some() || mapping.is_some() {
                    let concrete_key = state.constrained_word(&mut self.cx, &key);
                    Some(state.world.sload(
                        &mut self.cx,
                        executor,
                        state.storage_address,
                        key.clone(),
                        concrete_key,
                    )?)
                } else {
                    None
                };
                state.world.sstore(state.storage_address, key.clone(), value.clone());
                if let Some(hook) = hook {
                    let account =
                        SymExpr::constant(&mut self.cx, address_word(state.storage_address));
                    let calldata = self.storage_hook_calldata(
                        hook.callback_selector,
                        [
                            account,
                            key,
                            old_value.expect("old value loaded for storage hook"),
                            value,
                        ],
                    );
                    return self.invoke_storage_hook(
                        executor,
                        state,
                        worklist,
                        completed_paths,
                        hook,
                        calldata,
                    );
                } else if let Some((hook, provenance)) = mapping {
                    let account =
                        SymExpr::constant(&mut self.cx, address_word(state.storage_address));
                    let root = SymExpr::constant(&mut self.cx, provenance.root_slot);
                    let calldata = self.mapping_storage_hook_calldata(
                        hook.callback_selector,
                        [
                            account,
                            key,
                            root,
                            old_value.expect("old value loaded for mapping storage hook"),
                            value,
                        ],
                        provenance.keys,
                    );
                    return self.invoke_storage_hook(
                        executor,
                        state,
                        worklist,
                        completed_paths,
                        hook,
                        calldata,
                    );
                }
            }
            opcode::TLOAD => {
                let key = state.stack.pop()?;
                let value = state.world.tload(&mut self.cx, state.storage_address, key);
                state.stack.push(value)?;
            }
            opcode::TSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.world.tstore(state.storage_address, key, value);
            }
            opcode::JUMP => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(
                    &mut self.cx,
                    dest,
                    "symbolic JUMP destination",
                )?;
                ensure_jumpdest(dest, jumpdests)?;
                if !self.take_loop_jump(state, state.pc, dest) {
                    return Ok(StepOutcome::AssumeRejected);
                }
                state.pc = dest;
            }
            opcode::JUMPI => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(
                    &mut self.cx,
                    dest,
                    "symbolic JUMPI destination",
                )?;
                ensure_jumpdest(dest, jumpdests)?;
                let cond = state.stack.pop()?;
                match cond.truth() {
                    Some(true) => {
                        if !self.take_loop_jump(state, state.pc, dest) {
                            return Ok(StepOutcome::AssumeRejected);
                        }
                        state.pc = dest;
                    }
                    Some(false) => {}
                    None => {
                        let op_pc = state.pc.saturating_sub(1);
                        let _branch_span = trace_span!("jumpi_branch", pc = op_pc, dest).entered();
                        let true_cond = cond.nonzero_bool(&mut self.cx);
                        let false_cond = true_cond.clone().not(&mut self.cx);
                        let fallthrough = state.pc;
                        let (true_seed_models, false_seed_models) =
                            state.split_corpus_seed_models(&true_cond);
                        let mut true_state = state.clone();
                        true_state.constraints.push(true_cond);
                        true_state.set_corpus_seed_models(true_seed_models);
                        true_state.pc = dest;
                        let mut false_state = state.clone();
                        false_state.constraints.push(false_cond);
                        false_state.set_corpus_seed_models(false_seed_models);
                        false_state.pc = fallthrough;

                        let true_pending = self.take_loop_jump(&mut true_state, fallthrough, dest);
                        if true_pending {
                            true_state.defer_feasibility_check();
                        }
                        false_state.defer_feasibility_check();
                        trace!(true_pending, false_pending = true, "JUMPI symbolic branch");
                        if true_pending {
                            let true_seed_count = true_state.corpus_seed_model_count();
                            let false_seed_count = false_state.corpus_seed_model_count();
                            match (
                                false_seed_count.cmp(&true_seed_count),
                                self.config.exploration_order,
                            ) {
                                (std::cmp::Ordering::Greater, SymbolicExplorationOrder::Bfs)
                                | (std::cmp::Ordering::Less, SymbolicExplorationOrder::Dfs) => {
                                    worklist.push_back(false_state);
                                    worklist.push_back(true_state);
                                }
                                (std::cmp::Ordering::Greater, SymbolicExplorationOrder::Dfs)
                                | (std::cmp::Ordering::Less, SymbolicExplorationOrder::Bfs)
                                | (std::cmp::Ordering::Equal, _) => {
                                    worklist.push_back(true_state);
                                    worklist.push_back(false_state);
                                }
                            }
                        } else {
                            worklist.push_back(false_state);
                        }
                        return Ok(StepOutcome::Forked);
                    }
                }
            }
            opcode::PC => {
                let pc = state.pc - 1;
                let pc = SymExpr::constant(&mut self.cx, U256::from(pc));
                state.stack.push(pc)?;
            }
            opcode::MSIZE => {
                let size = state.memory.size_word(&mut self.cx);
                state.stack.push(size)?;
            }
            opcode::GAS => {
                let gas = state.fresh_gasleft(&mut self.cx);
                state.stack.push(gas)?;
            }
            opcode::JUMPDEST => {}
            opcode::MCOPY => {
                let dest = state.stack.peek(0)?.clone();
                let src = state.stack.peek(1)?.clone();
                let size = state.stack.peek(2)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &dest, &size)?
                {
                    return Ok(outcome);
                }
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &src, &size)?
                {
                    return Ok(outcome);
                }
                let dest = state.stack.pop()?;
                let src = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        state.memory.copy_memory_to_offset(&mut self.cx, dest, src, size)?;
                    }
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic MCOPY size",
                                )
                            })?;
                        if max_size != 0 {
                            state.memory.copy_memory_symbolic_size(
                                &mut self.cx,
                                dest,
                                src,
                                size,
                                max_size,
                            )?;
                        }
                    }
                }
            }
            opcode::RETURN | opcode::REVERT => {
                let offset = state.stack.peek(0)?.clone();
                let size = state.stack.peek(1)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &offset, &size)?
                {
                    return Ok(outcome);
                }
                return self.return_or_revert(state, op == opcode::REVERT);
            }
            opcode::INVALID => return Ok(StepOutcome::Failure),
            opcode::CALL => {
                return self.call(executor, state, worklist, completed_paths, CallKind::Call);
            }
            opcode::CALLCODE => {
                return self.call(executor, state, worklist, completed_paths, CallKind::CallCode);
            }
            opcode::DELEGATECALL => {
                return self.call(
                    executor,
                    state,
                    worklist,
                    completed_paths,
                    CallKind::DelegateCall,
                );
            }
            opcode::STATICCALL => {
                return self.call(executor, state, worklist, completed_paths, CallKind::StaticCall);
            }
            opcode::CREATE => {
                return self.create(executor, state, worklist, completed_paths, CreateKind::Create);
            }
            opcode::CREATE2 => {
                return self.create(
                    executor,
                    state,
                    worklist,
                    completed_paths,
                    CreateKind::Create2,
                );
            }
            opcode::SELFDESTRUCT => {
                if state.is_static {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    return Ok(StepOutcome::Revert);
                }
                let spec_id: SpecId = executor.spec_id().into();
                let (beneficiary_word, beneficiary) =
                    state.pop_address_word_or_symbolic_slot(&mut self.cx)?;
                if spec_id < SpecId::CANCUN
                    || state.world.was_created_in_current_transaction(state.address)
                {
                    state.world.selfdestruct_legacy(
                        &mut self.cx,
                        executor,
                        state.address,
                        beneficiary,
                    )?;
                } else {
                    if state.constrained_word(&mut self.cx, &beneficiary_word).is_none() {
                        return Err(SymbolicError::Unsupported(
                            "symbolic SELFDESTRUCT beneficiary",
                        ));
                    }
                    state.world.selfdestruct_cancun_existing(
                        &mut self.cx,
                        executor,
                        state.address,
                        beneficiary,
                    );
                }
                state.return_data = SymReturnData::empty(&mut self.cx);
                return Ok(StepOutcome::Halt);
            }
            opcode::CHAINID => {
                let value = state.block.chain_id.clone();
                state.stack.push(value)?;
            }
            opcode::BASEFEE => {
                let value = state.block.basefee.clone();
                state.stack.push(value)?;
            }
            opcode::GASPRICE => {
                let gas_price = state.gas_price.clone();
                state.stack.push(gas_price)?;
            }
            opcode::BLOBHASH => {
                let index = state.stack.pop()?;
                let index = state.expect_constrained_usize(
                    &mut self.cx,
                    index,
                    "symbolic BLOBHASH index",
                )?;
                let hash = state.block.blob_hash(index);
                let hash = SymExpr::constant(&mut self.cx, U256::from_be_slice(hash.as_slice()));
                state.stack.push(hash)?;
            }
            opcode::COINBASE => {
                let coinbase = state.block.coinbase;
                let coinbase = SymExpr::constant(&mut self.cx, address_word(coinbase));
                state.stack.push(coinbase)?;
            }
            opcode::TIMESTAMP => {
                let value = state.block.timestamp.clone();
                state.stack.push(value)?;
            }
            opcode::NUMBER => {
                let value = state.block.number.clone();
                state.stack.push(value)?;
            }
            opcode::DIFFICULTY => {
                let value = state.block.difficulty.clone();
                state.stack.push(value)?;
            }
            opcode::GASLIMIT => {
                let value = state.block.gaslimit.clone();
                state.stack.push(value)?;
            }
            opcode::BLOBBASEFEE => {
                let value = state.block.blob_basefee.clone();
                state.stack.push(value)?;
            }
            opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
                if state.is_static {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    return Ok(StepOutcome::Revert);
                }
                let topics = (op - opcode::LOG0) as usize;
                let offset = state.stack.peek(0)?.clone();
                let size = state.stack.peek(1)?.clone();
                if let Some(outcome) =
                    self.guard_memory_range(executor, state, worklist, &offset, &size)?
                {
                    return Ok(outcome);
                }
                let offset = state.stack.pop()?;
                if offset.contains_gasleft() {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                let size = state.stack.pop()?;
                if size.contains_gasleft() {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                let (data_len, data) = match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => (
                        SymExpr::constant(&mut self.cx, U256::from(size)),
                        state.memory.read_bytes_offset(&mut self.cx, offset, size),
                    ),
                    Some(Err(_)) => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&mut self.cx, &size)
                            .filter(|size| *size <= max_limit)
                            .map(Ok)
                            .unwrap_or_else(|| {
                                self.solver_upper_bound_usize(
                                    state,
                                    &size,
                                    max_limit,
                                    "symbolic LOG size",
                                )
                            })?;
                        let data = state.memory.read_bytes_symbolic_size(
                            &mut self.cx,
                            offset,
                            size.clone(),
                            max_size,
                        );
                        (size, data)
                    }
                };
                let mut log_topics = Vec::with_capacity(topics);
                for _ in 0..topics {
                    let topic = state.stack.pop()?;
                    if topic.contains_gasleft() {
                        return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                    }
                    log_topics.push(topic);
                }
                return self.handle_log(
                    state,
                    SymbolicLog::new(log_topics, data_len, data, state.address),
                );
            }
            _ => return Err(SymbolicError::UnsupportedOpcode(op)),
        };

        Ok(StepOutcome::Continue)
    }

    pub(super) fn assume_returndata_copy_in_bounds(
        &mut self,
        state: &mut PathState,
        offset: SymExpr,
        size: SymExpr,
    ) -> Result<bool, SymbolicError> {
        if offset.contains_gasleft() || size.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let end = SymExpr::binop(&mut self.cx, SymBinOp::Add, offset, size);
        let in_bounds =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, end, state.return_data.len_expr());
        match in_bounds.as_const() {
            Some(value) => Ok(value),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(in_bounds);
                if self.is_sat_with_state(state, &constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    pub(super) fn return_or_revert(
        &mut self,
        state: &mut PathState,
        is_revert: bool,
    ) -> Result<StepOutcome, SymbolicError> {
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        match state.constrained_usize_checked(&mut self.cx, &size) {
            Some(Ok(size)) => {
                state.return_data = state.memory.return_data(&mut self.cx, offset.clone(), size)?;
                if is_revert {
                    Ok(self.classify_revert(state, offset, size))
                } else {
                    Ok(StepOutcome::Halt)
                }
            }
            Some(Err(_)) => Ok(StepOutcome::Revert),
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&mut self.cx, &size)
                    .filter(|size| *size <= max_limit)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        self.solver_upper_bound_usize(
                            state,
                            &size,
                            max_limit,
                            if is_revert { "symbolic REVERT size" } else { "symbolic RETURN size" },
                        )
                    })?;
                state.return_data =
                    state.memory.return_data_symbolic_size(&mut self.cx, offset, size, max_size)?;
                Ok(if is_revert { StepOutcome::Revert } else { StepOutcome::Halt })
            }
        }
    }

    pub(super) fn classify_revert(
        &mut self,
        state: &PathState,
        offset: SymExpr,
        size: usize,
    ) -> StepOutcome {
        if state.call_depth == 0
            && let Some(offset) = offset.as_const()
            && let Ok(offset) = usize::try_from(offset)
            && let Ok(data) = state.memory.read_concrete(&mut self.cx, offset, size)
            && is_assertion_revert(&data)
        {
            StepOutcome::Failure
        } else {
            StepOutcome::Revert
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_state(executor: &mut SymbolicExecutor) -> PathState {
        let calldata =
            SymbolicCalldata::selector_only(&mut executor.cx, &Function::parse("empty()").unwrap())
                .unwrap();
        PathState::new(&mut executor.cx, Address::ZERO, Address::ZERO, U256::ZERO, calldata, false)
    }

    #[test]
    fn branch_target_constraint_is_one_shot_after_target_reached() {
        let mut executor = SymbolicExecutor::new(SymbolicConfig::default());
        let mut state = empty_state(&mut executor);
        state.set_branch_target(Some(SymbolicBranchTarget::new(
            Address::ZERO,
            0,
            opcode::EQ,
            false,
        )));
        state.mark_branch_target_reached();

        let condition = SymBoolExpr::constant(&mut executor.cx, false);
        let accepted =
            executor.apply_branch_target_constraint(&mut state, 0, opcode::EQ, &condition).unwrap();

        assert!(accepted);
        assert!(state.constraints.is_empty());
        assert!(state.satisfies_branch_target());
    }
}
