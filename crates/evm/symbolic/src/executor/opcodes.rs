use super::*;

impl SymbolicExecutor {
    #[expect(clippy::too_many_arguments)]
    /// Runs the `step` symbolic executor helper.
    pub(super) fn step<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        jumpdests: &HashSet<usize>,
        state: &mut PathState,
        worklist: &mut VecDeque<PathState>,
        completed_paths: &mut usize,
        op: u8,
    ) -> Result<StepOutcome, SymbolicError> {
        state.pc += 1;

        if op == opcode::PUSH0 {
            state.stack.push(SymWord::zero())?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            let n = (op - opcode::PUSH1 + 1) as usize;
            let end = state.pc.saturating_add(n);
            if end > code.len() {
                return Err(SymbolicError::InvalidBytecode("truncated PUSH data"));
            }
            let bytes = std::iter::repeat_with(SymWord::zero)
                .take(32 - n)
                .chain(code.read_bytes(state.pc, n))
                .collect::<Vec<_>>();
            state.pc = end;
            state.stack.push(word_from_bytes(bytes))?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::DUP1..=opcode::DUP16).contains(&op) {
            let n = (op - opcode::DUP1 + 1) as usize;
            let value = state.stack.peek(n - 1)?.clone();
            state.stack.push(value)?;
            return Ok(StepOutcome::Continue);
        }
        if (opcode::SWAP1..=opcode::SWAP16).contains(&op) {
            let n = (op - opcode::SWAP1 + 1) as usize;
            state.stack.swap(n)?;
            return Ok(StepOutcome::Continue);
        }

        match op {
            opcode::STOP => Ok(StepOutcome::Halt),
            opcode::ADD => state.bin_word(|a, b| a.wrapping_add(b), ExprOp::Add),
            opcode::SUB => state.bin_word(|a, b| a.wrapping_sub(b), ExprOp::Sub),
            opcode::MUL => state.bin_word(|a, b| a.wrapping_mul(b), ExprOp::Mul),
            opcode::EXP => state.exp_word(),
            opcode::DIV => state.bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a / b },
                ExprOp::UDiv,
            ),
            opcode::SDIV => state.bin_word_div_zero_guard(sdiv, ExprOp::SDiv),
            opcode::MOD => state.bin_word_div_zero_guard(
                |a, b| if b.is_zero() { U256::ZERO } else { a % b },
                ExprOp::URem,
            ),
            opcode::SMOD => state.bin_word_div_zero_guard(smod, ExprOp::SRem),
            opcode::ADDMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                match (a, b, n) {
                    (SymWord::Concrete(a), SymWord::Concrete(b), SymWord::Concrete(n)) => {
                        state.stack.push(SymWord::Concrete(addmod_word(a, b, n)))?;
                    }
                    (a, b, n) => {
                        state.stack.push(SymWord::from_expr(Expr::addmod(
                            a.into_expr(),
                            b.into_expr(),
                            n.into_expr(),
                        )))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::MULMOD => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                let n = state.stack.pop()?;
                match (a, b, n) {
                    (SymWord::Concrete(a), SymWord::Concrete(b), SymWord::Concrete(n)) => {
                        state.stack.push(SymWord::Concrete(mulmod_word(a, b, n)))?;
                    }
                    (a, b, n) => {
                        state.stack.push(SymWord::from_expr(Expr::mulmod(
                            a.into_expr(),
                            b.into_expr(),
                            n.into_expr(),
                        )))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::LT => state.cmp_word(|a, b| a < b, BoolExprOp::Ult),
            opcode::GT => state.cmp_word(|a, b| a > b, BoolExprOp::Ugt),
            opcode::SLT => state.cmp_word(slt, BoolExprOp::Slt),
            opcode::SGT => state.cmp_word(|a, b| slt(b, a), BoolExprOp::Sgt),
            opcode::EQ => {
                let a = state.stack.pop()?;
                let b = state.stack.pop()?;
                state.stack.push(SymWord::from_bool(BoolExpr::eq(b.into_expr(), a.into_expr())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::ISZERO => {
                let value = state.stack.pop()?;
                state.stack.push(SymWord::from_bool(value.into_zero_bool()))?;
                Ok(StepOutcome::Continue)
            }
            opcode::AND => state.bin_word(|a, b| a & b, ExprOp::And),
            opcode::OR => state.bin_word(|a, b| a | b, ExprOp::Or),
            opcode::XOR => state.bin_word(|a, b| a ^ b, ExprOp::Xor),
            opcode::NOT => {
                let value = state.stack.pop()?;
                state.stack.push(match value {
                    SymWord::Concrete(value) => SymWord::Concrete(!value),
                    value => SymWord::from_expr(Expr::not(value.into_expr())),
                })?;
                Ok(StepOutcome::Continue)
            }
            opcode::SIGNEXTEND => {
                let byte_index = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.stack.push(signextend_word_dynamic(byte_index, value))?;
                Ok(StepOutcome::Continue)
            }
            opcode::BYTE => {
                let index = state.stack.pop()?;
                let word = state.stack.pop()?;
                state.stack.push(byte_word_dynamic(index, word))?;
                Ok(StepOutcome::Continue)
            }
            opcode::SHL => state.shift_word(ShiftKind::Shl),
            opcode::SHR => state.shift_word(ShiftKind::Shr),
            opcode::SAR => state.shift_word(ShiftKind::Sar),
            opcode::KECCAK256 => {
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        let bytes = state.memory.read_bytes_offset(offset, size);
                        state.stack.push(keccak_word(bytes))?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
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
                        let bytes =
                            state.memory.read_bytes_symbolic_size(offset, size.clone(), max_size);
                        state.stack.push(keccak_word_with_len(bytes, size))?;
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::ADDRESS => {
                let address = state.address_word.clone();
                state.stack.push(address)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLER => {
                let caller = state.caller_word.clone();
                state.stack.push(caller)?;
                Ok(StepOutcome::Continue)
            }
            opcode::ORIGIN => {
                let origin = state.origin_word.clone();
                state.stack.push(origin)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLVALUE => {
                let callvalue = state.callvalue.clone();
                state.stack.push(callvalue)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOCKHASH => {
                let number = state.stack.pop()?;
                let hash = state.block.block_hash_word(executor, number)?;
                state.stack.push(hash)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BALANCE => {
                let target = state.stack.pop()?;
                let balance = state.balance_word(executor, target)?;
                state.stack.push(balance)?;
                Ok(StepOutcome::Continue)
            }
            opcode::SELFBALANCE => {
                let balance = state.balance(executor, state.address);
                state.stack.push(balance)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODESIZE => {
                let target = state.stack.pop()?;
                let size = state.extcode_size_word(executor, target)?;
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODEHASH => {
                let target = state.stack.pop()?;
                let hash = state.extcode_hash_word(executor, target)?;
                state.stack.push(hash)?;
                Ok(StepOutcome::Continue)
            }
            opcode::EXTCODECOPY => {
                let target = state.stack.pop()?;
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        let bytes = state.extcode_bytes_word(executor, target, offset, size)?;
                        state.memory.copy_symbolic_offset(dest, bytes);
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
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
                            let bytes =
                                state.extcode_bytes_word(executor, target, offset, max_size)?;
                            state.memory.copy_symbolic_size_offset(dest, size, bytes)?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATALOAD => {
                let offset = state.stack.pop()?;
                let value = state.calldata.load_word(offset)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATASIZE => {
                let size = state.calldata.size_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::CALLDATACOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        if size != 0 {
                            let calldata = state.calldata.clone();
                            state.memory.copy_calldata_to_offset(dest, offset, size, &calldata)?;
                        }
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
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
                            let calldata = state.calldata.clone();
                            state.memory.copy_calldata_symbolic_size(
                                dest, offset, size, max_size, &calldata,
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::CODESIZE => {
                state.stack.push(SymWord::Concrete(U256::from(code.len())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::CODECOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        state
                            .memory
                            .copy_symbolic_offset(dest, code.read_bytes_offset(offset, size));
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
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
                            state.memory.copy_symbolic_size_offset(
                                dest,
                                size,
                                code.read_bytes_offset(offset, max_size),
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::RETURNDATASIZE => {
                let size = state.return_data.len_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::RETURNDATACOPY => {
                let dest = state.stack.pop()?;
                let offset = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        if !self.assume_returndata_copy_in_bounds(
                            state,
                            offset.clone(),
                            SymWord::Concrete(U256::from(size)),
                        )? {
                            return Ok(StepOutcome::Revert);
                        }
                        let return_data = state.return_data.clone();
                        state.memory.copy_return_data_to_offset(
                            dest,
                            offset,
                            size,
                            &return_data,
                        )?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let available = state
                            .constrained_usize(&offset)
                            .map(|offset| state.return_data.len().saturating_sub(offset))
                            .unwrap_or(state.return_data.len());
                        let max_limit = available.min(self.config.max_calldata_bytes as usize);
                        let max_size = state
                            .upper_bound_usize(&size)
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
                            let return_data = state.return_data.clone();
                            if !self.assume_returndata_copy_in_bounds(
                                state,
                                offset.clone(),
                                size.clone(),
                            )? {
                                return Ok(StepOutcome::Revert);
                            }
                            state.memory.copy_return_data_symbolic_size(
                                dest,
                                offset,
                                size,
                                max_size,
                                &return_data,
                            )?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::POP => {
                state.stack.pop()?;
                Ok(StepOutcome::Continue)
            }
            opcode::MLOAD => {
                let offset = state.stack.pop()?;
                let value = state.memory.load_word_offset(offset)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::MSTORE => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_word_offset(offset, value);
                Ok(StepOutcome::Continue)
            }
            opcode::MSTORE8 => {
                let offset = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.memory.store_byte_offset(offset, value);
                Ok(StepOutcome::Continue)
            }
            opcode::SLOAD => {
                let key = state.stack.pop()?;
                state.record_sload(state.storage_address, key.clone());
                let concrete_key = state.constrained_word(&key);
                let value =
                    state.world.sload(executor, state.storage_address, key, concrete_key)?;
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::SSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.record_sstore(state.storage_address, key.clone());
                state.world.sstore(state.storage_address, key, value);
                Ok(StepOutcome::Continue)
            }
            opcode::TLOAD => {
                let key = state.stack.pop()?;
                let value = state.world.tload(state.storage_address, key);
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::TSTORE => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let key = state.stack.pop()?;
                let value = state.stack.pop()?;
                state.world.tstore(state.storage_address, key, value);
                Ok(StepOutcome::Continue)
            }
            opcode::JUMP => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(dest, "symbolic JUMP destination")?;
                ensure_jumpdest(dest, jumpdests)?;
                if !self.take_loop_jump(state, state.pc, dest) {
                    return Ok(StepOutcome::AssumeRejected);
                }
                state.pc = dest;
                Ok(StepOutcome::Continue)
            }
            opcode::JUMPI => {
                let dest = state.stack.pop()?;
                let dest = state.expect_constrained_usize(dest, "symbolic JUMPI destination")?;
                ensure_jumpdest(dest, jumpdests)?;
                let cond = state.stack.pop()?;
                match cond.truth() {
                    Some(true) => {
                        if !self.take_loop_jump(state, state.pc, dest) {
                            return Ok(StepOutcome::AssumeRejected);
                        }
                        state.pc = dest;
                        Ok(StepOutcome::Continue)
                    }
                    Some(false) => Ok(StepOutcome::Continue),
                    None => {
                        if cond.contains_gasleft() {
                            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                        }
                        let op_pc = state.pc.saturating_sub(1);
                        let _branch_span = trace_span!("jumpi_branch", pc = op_pc, dest).entered();
                        let true_cond = cond.nonzero_bool();
                        let false_cond = true_cond.clone().not();
                        let fallthrough = state.pc;
                        let mut true_state = state.clone();
                        true_state.constraints.push(true_cond);
                        true_state.pc = dest;
                        let mut false_state = state.clone();
                        false_state.constraints.push(false_cond);
                        false_state.pc = fallthrough;

                        let true_feasible = self.take_loop_jump(&mut true_state, fallthrough, dest)
                            && self.branch_is_sat_or_defer(&true_state.constraints)?;
                        let false_feasible =
                            self.branch_is_sat_or_defer(&false_state.constraints)?;
                        trace!(true_feasible, false_feasible, "JUMPI symbolic branch");
                        if true_feasible {
                            worklist.push_back(true_state);
                        }
                        if false_feasible {
                            worklist.push_back(false_state);
                        }
                        Ok(StepOutcome::Forked)
                    }
                }
            }
            opcode::PC => {
                let pc = state.pc - 1;
                state.stack.push(SymWord::Concrete(U256::from(pc)))?;
                Ok(StepOutcome::Continue)
            }
            opcode::MSIZE => {
                let size = state.memory.size_word();
                state.stack.push(size)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GAS => {
                let gas = state.fresh_gasleft();
                state.stack.push(gas)?;
                Ok(StepOutcome::Continue)
            }
            opcode::JUMPDEST => Ok(StepOutcome::Continue),
            opcode::MCOPY => {
                let dest = state.stack.pop()?;
                let src = state.stack.pop()?;
                let size = state.stack.pop()?;
                match state.constrained_usize(&size) {
                    Some(size) => {
                        state.memory.copy_memory_to_offset(dest, src, size)?;
                    }
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
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
                            state.memory.copy_memory_symbolic_size(dest, src, size, max_size)?;
                        }
                    }
                }
                Ok(StepOutcome::Continue)
            }
            opcode::RETURN => self.return_or_revert(state, false),
            opcode::REVERT => self.return_or_revert(state, true),
            opcode::INVALID => Ok(StepOutcome::Failure),
            opcode::CALL => self.call(executor, state, worklist, completed_paths, CallKind::Call),
            opcode::CALLCODE => {
                self.call(executor, state, worklist, completed_paths, CallKind::CallCode)
            }
            opcode::DELEGATECALL => {
                self.call(executor, state, worklist, completed_paths, CallKind::DelegateCall)
            }
            opcode::STATICCALL => {
                self.call(executor, state, worklist, completed_paths, CallKind::StaticCall)
            }
            opcode::CREATE => {
                self.create(executor, state, worklist, completed_paths, CreateKind::Create)
            }
            opcode::CREATE2 => {
                self.create(executor, state, worklist, completed_paths, CreateKind::Create2)
            }
            opcode::SELFDESTRUCT => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
                    return Ok(StepOutcome::Revert);
                }
                let spec_id: SpecId = executor.spec_id().into();
                let (beneficiary_word, beneficiary) = state.pop_address_word_or_symbolic_slot()?;
                if spec_id < SpecId::CANCUN
                    || state.world.was_created_in_current_transaction(state.address)
                {
                    state.world.selfdestruct_legacy(executor, state.address, beneficiary)?;
                } else {
                    if state.constrained_word(&beneficiary_word).is_none() {
                        return Err(SymbolicError::Unsupported(
                            "symbolic SELFDESTRUCT beneficiary",
                        ));
                    }
                    state.world.selfdestruct_cancun_existing(executor, state.address, beneficiary);
                }
                state.return_data = SymReturnData::default();
                Ok(StepOutcome::Halt)
            }
            opcode::CHAINID => {
                let value = state.block.chain_id.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BASEFEE => {
                let value = state.block.basefee.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GASPRICE => {
                let gas_price = state.gas_price.clone();
                state.stack.push(gas_price)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOBHASH => {
                let index = state.stack.pop()?;
                let index = state.expect_constrained_usize(index, "symbolic BLOBHASH index")?;
                let hash = state.block.blob_hash(index);
                state.stack.push(SymWord::Concrete(U256::from_be_slice(hash.as_slice())))?;
                Ok(StepOutcome::Continue)
            }
            opcode::COINBASE => {
                let coinbase = state.block.coinbase;
                state.stack.push(SymWord::Concrete(address_word(coinbase)))?;
                Ok(StepOutcome::Continue)
            }
            opcode::TIMESTAMP => {
                let value = state.block.timestamp.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::NUMBER => {
                let value = state.block.number.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::DIFFICULTY => {
                let value = state.block.difficulty.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::GASLIMIT => {
                let value = state.block.gaslimit.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::BLOBBASEFEE => {
                let value = state.block.blob_basefee.clone();
                state.stack.push(value)?;
                Ok(StepOutcome::Continue)
            }
            opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
                if state.is_static {
                    state.return_data = SymReturnData::default();
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
                let (data_len, data) = match state.constrained_usize(&size) {
                    Some(size) => (
                        SymWord::Concrete(U256::from(size)),
                        state.memory.read_bytes_offset(offset, size),
                    ),
                    None if state.constrained_word(&size).is_some() => {
                        return Ok(StepOutcome::Revert);
                    }
                    None => {
                        let max_limit = self.config.max_calldata_bytes as usize;
                        let max_size = state
                            .upper_bound_usize(&size)
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
                        let data =
                            state.memory.read_bytes_symbolic_size(offset, size.clone(), max_size);
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
                if data.iter().any(SymWord::contains_gasleft) {
                    return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
                }
                self.handle_log(
                    state,
                    SymbolicLog {
                        topics: log_topics.into(),
                        data_len,
                        data: data.into(),
                        emitter: state.address,
                    },
                )
            }
            _ => Err(SymbolicError::UnsupportedOpcode(op)),
        }
    }

    /// Implements the `assume_returndata_copy_in_bounds` symbolic executor helper.
    pub(super) fn assume_returndata_copy_in_bounds(
        &mut self,
        state: &mut PathState,
        offset: SymWord,
        size: SymWord,
    ) -> Result<bool, SymbolicError> {
        if offset.contains_gasleft() || size.contains_gasleft() {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let end = Expr::op(ExprOp::Add, offset.into_expr(), size.into_expr());
        let in_bounds = BoolExpr::cmp(BoolExprOp::Ule, end, state.return_data.len_expr());
        match in_bounds {
            BoolExpr::Const(value) => Ok(value),
            in_bounds => {
                let mut constraints = state.constraints.clone();
                constraints.push(in_bounds);
                if self.solver.is_sat(&constraints)? {
                    state.constraints = constraints;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Implements the `return_or_revert` symbolic executor helper.
    pub(super) fn return_or_revert(
        &mut self,
        state: &mut PathState,
        is_revert: bool,
    ) -> Result<StepOutcome, SymbolicError> {
        let offset = state.stack.pop()?;
        let size = state.stack.pop()?;
        match state.constrained_usize(&size) {
            Some(size) => {
                state.return_data = state.memory.return_data(offset.clone(), size)?;
                if is_revert {
                    Ok(self.classify_revert(state, offset, size))
                } else {
                    Ok(StepOutcome::Halt)
                }
            }
            None if state.constrained_word(&size).is_some() => Ok(StepOutcome::Revert),
            None => {
                let max_limit = self.config.max_calldata_bytes as usize;
                let max_size = state
                    .upper_bound_usize(&size)
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
                    state.memory.return_data_symbolic_size(offset, size, max_size)?;
                Ok(if is_revert { StepOutcome::Revert } else { StepOutcome::Halt })
            }
        }
    }

    /// Runs the `classify_revert` symbolic executor helper.
    pub(super) fn classify_revert(
        &self,
        state: &PathState,
        offset: SymWord,
        size: usize,
    ) -> StepOutcome {
        if state.call_depth == 0
            && let SymWord::Concrete(offset) = offset
            && offset <= U256::from(usize::MAX)
            && let Ok(data) = state.memory.read_concrete(offset.to::<usize>(), size)
            && is_assertion_revert(&data)
        {
            StepOutcome::Failure
        } else {
            StepOutcome::Revert
        }
    }
}
