use super::*;

impl SymbolicExecutor {
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
                state.bin_word(&mut self.cx, SymExprBinOp::Add)?;
            }
            opcode::SUB => {
                state.bin_word(&mut self.cx, SymExprBinOp::Sub)?;
            }
            opcode::MUL => {
                state.bin_word(&mut self.cx, SymExprBinOp::Mul)?;
            }
            opcode::EXP => {
                state.exp_word(&mut self.cx)?;
            }
            opcode::DIV => {
                state.bin_word_div_zero_guard(&mut self.cx, SymExprBinOp::UDiv)?;
            }
            opcode::SDIV => {
                state.bin_word_div_zero_guard(&mut self.cx, SymExprBinOp::SDiv)?;
            }
            opcode::MOD => {
                state.bin_word_div_zero_guard(&mut self.cx, SymExprBinOp::URem)?;
            }
            opcode::SMOD => {
                state.bin_word_div_zero_guard(&mut self.cx, SymExprBinOp::SRem)?;
            }
            opcode::ADDMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                state.stack.push(SymExpr::ternop(&mut self.cx, SymExprTernOp::AddMod, a, b, n))?;
            }
            opcode::MULMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                state.stack.push(SymExpr::ternop(&mut self.cx, SymExprTernOp::MulMod, a, b, n))?;
            }
            opcode::LT => {
                state.cmp_word(&mut self.cx, SymCmpOp::Ult)?;
            }
            opcode::GT => {
                state.cmp_word(&mut self.cx, SymCmpOp::Ugt)?;
            }
            opcode::SLT => {
                state.cmp_word(&mut self.cx, SymCmpOp::Slt)?;
            }
            opcode::SGT => {
                state.cmp_word(&mut self.cx, SymCmpOp::Sgt)?;
            }
            opcode::EQ => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let condition = SymBoolExpr::eq(&mut self.cx, b, a);
                let value = SymExpr::bool_word(&mut self.cx, condition);
                state.stack.push(value)?;
            }
            opcode::ISZERO => {
                let value = state.stack.pop()?;
                let value = value.into_zero_bool(&mut self.cx);
                state.stack.push(SymExpr::bool_word(&mut self.cx, value))?;
            }
            opcode::AND => {
                state.bin_word(&mut self.cx, SymExprBinOp::And)?;
            }
            opcode::OR => {
                state.bin_word(&mut self.cx, SymExprBinOp::Or)?;
            }
            opcode::XOR => {
                state.bin_word(&mut self.cx, SymExprBinOp::Xor)?;
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
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize_checked(&mut self.cx, &size) {
                    Some(Ok(size)) => {
                        let bytes = state.memory.read_byte_exprs_offset(&mut self.cx, offset, size);
                        state.stack.push(keccak_word(&mut self.cx, bytes))?;
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
                let offset = state.stack.pop()?;
                let value = state.memory.load_word_offset(&mut self.cx, offset)?;
                state.stack.push(value)?;
            }
            opcode::MSTORE => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_word_offset(&mut self.cx, offset, value);
            }
            opcode::MSTORE8 => {
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
                    key,
                    concrete_key,
                )?;
                state.stack.push(value)?;
            }
            opcode::SSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::empty(&mut self.cx);
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.record_sstore(state.storage_address, key.clone());
                state.world.sstore(state.storage_address, key, value);
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
                        if cond.contains_gasleft() {
                            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                        }
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

                        let true_feasible = self.take_loop_jump(&mut true_state, fallthrough, dest)
                            && self.branch_is_sat_or_defer(&true_state.constraints)?;
                        let false_feasible =
                            self.branch_is_sat_or_defer(&false_state.constraints)?;
                        trace!(true_feasible, false_feasible, "JUMPI symbolic branch");
                        match (true_feasible, false_feasible) {
                            (true, true) => {
                                let true_seed_count = true_state.corpus_seed_model_count();
                                let false_seed_count = false_state.corpus_seed_model_count();
                                match (
                                    false_seed_count.cmp(&true_seed_count),
                                    self.config.exploration_order,
                                ) {
                                    (
                                        std::cmp::Ordering::Greater,
                                        SymbolicExplorationOrder::Bfs,
                                    )
                                    | (std::cmp::Ordering::Less, SymbolicExplorationOrder::Dfs) => {
                                        worklist.push_back(false_state);
                                        worklist.push_back(true_state);
                                    }
                                    (
                                        std::cmp::Ordering::Greater,
                                        SymbolicExplorationOrder::Dfs,
                                    )
                                    | (std::cmp::Ordering::Less, SymbolicExplorationOrder::Bfs)
                                    | (std::cmp::Ordering::Equal, _) => {
                                        worklist.push_back(true_state);
                                        worklist.push_back(false_state);
                                    }
                                }
                            }
                            (true, false) => worklist.push_back(true_state),
                            (false, true) => worklist.push_back(false_state),
                            (false, false) => {}
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
            opcode::RETURN => return self.return_or_revert(state, false),
            opcode::REVERT => return self.return_or_revert(state, true),
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
        let end = SymExpr::binop(&mut self.cx, SymExprBinOp::Add, offset, size);
        let in_bounds =
            SymBoolExpr::cmp(&mut self.cx, SymCmpOp::Ule, end, state.return_data.len_expr());
        match in_bounds.as_const() {
            Some(value) => Ok(value),
            None => {
                let mut constraints = state.constraints.clone();
                constraints.push(in_bounds);
                if self.solver.is_sat(&mut self.cx, &constraints)? {
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
