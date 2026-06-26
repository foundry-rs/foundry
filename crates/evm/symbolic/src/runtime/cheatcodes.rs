use alloy_sol_types::SolCall;
use foundry_cheatcodes_spec::Vm::*;
use foundry_common::wallet::private_key_from_u256;

use super::*;

/// Implements the `foundry_cheatcode_min_input_size` cheatcode runtime helper.
pub(crate) const fn foundry_cheatcode_min_input_size(selector: [u8; 4]) -> Option<usize> {
    match selector {
        recordLogsCall::SELECTOR
        | recordCall::SELECTOR
        | stopRecordCall::SELECTOR
        | assumeNoRevert_0Call::SELECTOR
        | getRecordedLogsCall::SELECTOR
        | getRecordedLogsJsonCall::SELECTOR
        | expectRevert_0Call::SELECTOR
        | expectEmit_2Call::SELECTOR
        | expectEmitAnonymous_2Call::SELECTOR
        | clearMockedCallsCall::SELECTOR
        | stopPrankCall::SELECTOR
        | readCallersCall::SELECTOR
        | getWalletsCall::SELECTOR
        | snapshotCall::SELECTOR
        | snapshotStateCall::SELECTOR
        | deleteSnapshotsCall::SELECTOR
        | deleteStateSnapshotsCall::SELECTOR
        | activeForkCall::SELECTOR
        | getChainIdCall::SELECTOR
        | getBlobhashesCall::SELECTOR
        | getBlobBaseFeeCall::SELECTOR
        | getBlockNumberCall::SELECTOR
        | getBlockTimestampCall::SELECTOR
        | pauseGasMeteringCall::SELECTOR
        | resumeGasMeteringCall::SELECTOR
        | resetGasMeteringCall::SELECTOR
        | lastCallGasCall::SELECTOR
        | stopExpectSafeMemoryCall::SELECTOR
        | stopSnapshotGas_0Call::SELECTOR
        | getEvmVersionCall::SELECTOR
        | getFoundryVersionCall::SELECTOR
        | projectRootCall::SELECTOR
        | unixTimeCall::SELECTOR
        | noAccessListCall::SELECTOR
        | randomUint_0Call::SELECTOR
        | randomInt_0Call::SELECTOR
        | randomAddressCall::SELECTOR
        | randomBoolCall::SELECTOR
        | randomBytes4Call::SELECTOR
        | randomBytes8Call::SELECTOR => Some(abi_static_input_size(0)),
        assumeCall::SELECTOR
        | skip_0Call::SELECTOR
        | skip_1Call::SELECTOR
        | assumeNoRevert_1Call::SELECTOR
        | assumeNoRevert_2Call::SELECTOR
        | accessesCall::SELECTOR
        | expectRevert_1Call::SELECTOR
        | expectRevert_3Call::SELECTOR
        | expectRevert_6Call::SELECTOR
        | expectPartialRevert_0Call::SELECTOR
        | expectEmit_3Call::SELECTOR
        | expectEmit_6Call::SELECTOR
        | expectEmitAnonymous_3Call::SELECTOR
        | prank_0Call::SELECTOR
        | startPrank_0Call::SELECTOR
        | addrCall::SELECTOR
        | rememberKeyCall::SELECTOR
        | toString_0Call::SELECTOR
        | toString_1Call::SELECTOR
        | toString_2Call::SELECTOR
        | toString_3Call::SELECTOR
        | toString_4Call::SELECTOR
        | toString_5Call::SELECTOR
        | parseBytesCall::SELECTOR
        | parseAddressCall::SELECTOR
        | parseUintCall::SELECTOR
        | parseIntCall::SELECTOR
        | parseBytes32Call::SELECTOR
        | parseBoolCall::SELECTOR
        | toLowercaseCall::SELECTOR
        | toUppercaseCall::SELECTOR
        | trimCall::SELECTOR
        | toBase64_0Call::SELECTOR
        | toBase64_1Call::SELECTOR
        | toBase64URL_0Call::SELECTOR
        | toBase64URL_1Call::SELECTOR
        | breakpoint_0Call::SELECTOR
        | setEvmVersionCall::SELECTOR
        | sleepCall::SELECTOR
        | accessListCall::SELECTOR
        | getNonce_0Call::SELECTOR
        | resetNonceCall::SELECTOR
        | allowCheatcodesCall::SELECTOR
        | makePersistent_0Call::SELECTOR
        | makePersistent_3Call::SELECTOR
        | revokePersistent_0Call::SELECTOR
        | revokePersistent_1Call::SELECTOR
        | isPersistentCall::SELECTOR
        | selectForkCall::SELECTOR
        | createFork_0Call::SELECTOR
        | createSelectFork_0Call::SELECTOR
        | rollFork_0Call::SELECTOR
        | rollFork_1Call::SELECTOR
        | revertToCall::SELECTOR
        | revertToStateCall::SELECTOR
        | revertToAndDeleteCall::SELECTOR
        | revertToStateAndDeleteCall::SELECTOR
        | deleteSnapshotCall::SELECTOR
        | deleteStateSnapshotCall::SELECTOR
        | warpCall::SELECTOR
        | rollCall::SELECTOR
        | prevrandao_0Call::SELECTOR
        | prevrandao_1Call::SELECTOR
        | feeCall::SELECTOR
        | blobBaseFeeCall::SELECTOR
        | chainIdCall::SELECTOR
        | difficultyCall::SELECTOR
        | coinbaseCall::SELECTOR
        | txGasPriceCall::SELECTOR
        | getLabelCall::SELECTOR
        | snapshotGasLastCall_0Call::SELECTOR
        | startSnapshotGas_0Call::SELECTOR
        | stopSnapshotGas_1Call::SELECTOR
        | coolCall::SELECTOR
        | isContextCall::SELECTOR
        | assertTrue_0Call::SELECTOR
        | assertTrue_1Call::SELECTOR
        | assertFalse_0Call::SELECTOR
        | assertFalse_1Call::SELECTOR
        | randomUint_2Call::SELECTOR
        | randomInt_1Call::SELECTOR
        | randomBytesCall::SELECTOR => Some(abi_static_input_size(1)),
        expectRevert_4Call::SELECTOR
        | expectRevert_7Call::SELECTOR
        | expectRevert_9Call::SELECTOR
        | expectPartialRevert_1Call::SELECTOR
        | expectEmit_7Call::SELECTOR
        | prank_1Call::SELECTOR
        | prank_2Call::SELECTOR
        | startPrank_1Call::SELECTOR
        | startPrank_2Call::SELECTOR
        | sign_1Call::SELECTOR
        | signCompact_1Call::SELECTOR
        | deriveKey_0Call::SELECTOR
        | splitCall::SELECTOR
        | indexOfCall::SELECTOR
        | containsCall::SELECTOR
        | breakpoint_1Call::SELECTOR
        | expectSafeMemoryCall::SELECTOR
        | expectSafeMemoryCallCall::SELECTOR
        | loadCall::SELECTOR
        | makePersistent_1Call::SELECTOR
        | computeCreateAddressCall::SELECTOR
        | computeCreate2Address_1Call::SELECTOR
        | setBlockhashCall::SELECTOR
        | dealCall::SELECTOR
        | setNonceCall::SELECTOR
        | setNonceUnsafeCall::SELECTOR
        | createFork_1Call::SELECTOR
        | createFork_2Call::SELECTOR
        | createSelectFork_1Call::SELECTOR
        | createSelectFork_2Call::SELECTOR
        | rollFork_2Call::SELECTOR
        | rollFork_3Call::SELECTOR
        | labelCall::SELECTOR
        | snapshotValue_0Call::SELECTOR
        | snapshotGasLastCall_1Call::SELECTOR
        | startSnapshotGas_1Call::SELECTOR
        | stopSnapshotGas_2Call::SELECTOR
        | warmSlotCall::SELECTOR
        | coolSlotCall::SELECTOR
        | assertEq_2Call::SELECTOR
        | assertEq_3Call::SELECTOR
        | assertEq_4Call::SELECTOR
        | assertEq_5Call::SELECTOR
        | assertEq_6Call::SELECTOR
        | assertEq_7Call::SELECTOR
        | assertEq_8Call::SELECTOR
        | assertEq_9Call::SELECTOR
        | assertEq_0Call::SELECTOR
        | assertEq_1Call::SELECTOR
        | assertNotEq_2Call::SELECTOR
        | assertNotEq_3Call::SELECTOR
        | assertNotEq_4Call::SELECTOR
        | assertNotEq_5Call::SELECTOR
        | assertNotEq_6Call::SELECTOR
        | assertNotEq_7Call::SELECTOR
        | assertNotEq_8Call::SELECTOR
        | assertNotEq_9Call::SELECTOR
        | assertNotEq_0Call::SELECTOR
        | assertNotEq_1Call::SELECTOR
        | assertLt_0Call::SELECTOR
        | assertLt_1Call::SELECTOR
        | assertLe_0Call::SELECTOR
        | assertLe_1Call::SELECTOR
        | assertGt_0Call::SELECTOR
        | assertGt_1Call::SELECTOR
        | assertGe_0Call::SELECTOR
        | assertGe_1Call::SELECTOR
        | assertLt_2Call::SELECTOR
        | assertLt_3Call::SELECTOR
        | assertGt_2Call::SELECTOR
        | assertGt_3Call::SELECTOR
        | assertLe_2Call::SELECTOR
        | assertLe_3Call::SELECTOR
        | assertGe_2Call::SELECTOR
        | assertGe_3Call::SELECTOR
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
        | assertNotEq_27Call::SELECTOR
        | randomUint_1Call::SELECTOR => Some(abi_static_input_size(2)),
        expectRevert_10Call::SELECTOR
        | deriveKey_1Call::SELECTOR
        | deriveKey_2Call::SELECTOR
        | rememberKeys_0Call::SELECTOR
        | storeCall::SELECTOR
        | makePersistent_2Call::SELECTOR
        | snapshotValue_1Call::SELECTOR
        | replaceCall::SELECTOR
        | assertEqDecimal_0Call::SELECTOR
        | assertEqDecimal_2Call::SELECTOR
        | computeCreate2Address_0Call::SELECTOR
        | bound_0Call::SELECTOR
        | bound_1Call::SELECTOR => Some(abi_static_input_size(3)),
        expectEmit_0Call::SELECTOR
        | deriveKey_3Call::SELECTOR
        | rememberKeys_1Call::SELECTOR
        | assertEqDecimal_1Call::SELECTOR
        | assertEqDecimal_3Call::SELECTOR => Some(abi_static_input_size(4)),
        expectEmitAnonymous_0Call::SELECTOR
        | expectEmit_1Call::SELECTOR
        | expectEmit_4Call::SELECTOR => Some(abi_static_input_size(5)),
        expectEmit_5Call::SELECTOR | expectEmitAnonymous_1Call::SELECTOR => {
            Some(abi_static_input_size(6))
        }
        _ => None,
    }
}

/// Returns the `symbolic_vm_cheatcode_min_input_size` cheatcode runtime helper result.
pub(crate) fn symbolic_vm_cheatcode_min_input_size(selector: [u8; 4]) -> Option<usize> {
    let selectors = symbolic_vm_selectors();
    if selector_in(
        selector,
        &[
            selectors.create_uint256,
            selectors.create_int256,
            selectors.create_bytes32,
            selectors.create_address,
            selectors.create_bool,
            selectors.create_bytes,
            selectors.create_string,
            selectors.create_bytes4,
            selectors.create_calldata,
            snapshotStateCall::SELECTOR,
        ],
    ) || symbolic_create_uint_selectors().iter().any(|(_, candidate)| selector == *candidate)
        || symbolic_create_int_selectors().iter().any(|(_, candidate)| selector == *candidate)
        || symbolic_create_bytes_selectors().iter().any(|(_, candidate)| selector == *candidate)
    {
        return Some(abi_static_input_size(0));
    }

    if selector_in(
        selector,
        &[
            selectors.enable_symbolic_storage,
            setArbitraryStorage_0Call::SELECTOR,
            selectors.snapshot_storage,
        ],
    ) {
        return Some(abi_static_input_size(1));
    }
    if selector_in(
        selector,
        &[
            selectors.create_uint,
            selectors.create_int,
            selectors.create_bytes_sized,
            selectors.create_string_sized,
        ],
    ) {
        return Some(abi_static_input_size(1));
    }

    None
}

/// Returns the `abi_static_input_size` cheatcode runtime helper result.
pub(crate) const fn abi_static_input_size(words: usize) -> usize {
    4 + words * 32
}

/// Returns the `selector_in` cheatcode runtime helper result.
pub(crate) fn selector_in(selector: [u8; 4], selectors: &[[u8; 4]]) -> bool {
    selectors.contains(&selector)
}

/// Returns the `abi_bytes_return` cheatcode runtime helper result.
pub(crate) fn abi_bytes_return(bytes: Vec<SymWord>) -> SymReturnData {
    abi_bytes_return_with_len(SymWord::Concrete(U256::from(bytes.len())), bytes)
}

/// Returns the `abi_bytes_return_with_len` cheatcode runtime helper result.
pub(crate) fn abi_bytes_return_with_len(len: SymWord, bytes: Vec<SymWord>) -> SymReturnData {
    let mut out = word_bytes(SymWord::Concrete(U256::from(32)));
    out.extend(word_bytes(len));
    out.extend(bytes.iter().cloned());
    out.resize(64 + bytes.len().next_multiple_of(32), SymWord::zero());
    SymReturnData::from_symbolic_bytes(out)
}

/// Returns the `abi_concrete_bytes_return` cheatcode runtime helper result.
pub(crate) fn abi_concrete_bytes_return(bytes: impl IntoIterator<Item = u8>) -> SymReturnData {
    abi_bytes_return(bytes.into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect())
}

/// Returns the `abi_concrete_value_return` cheatcode runtime helper result.
pub(crate) fn abi_concrete_value_return(value: DynSolValue) -> SymReturnData {
    SymReturnData::from_symbolic_bytes(
        value.abi_encode().into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
    )
}

/// Implements the `recorded_logs_return_data` cheatcode runtime helper.
pub(crate) fn recorded_logs_return_data(logs: Vec<SymbolicLog>) -> SymReturnData {
    let value = SymbolicAbiValue::Array {
        elements: logs
            .into_iter()
            .map(|log| {
                let topics = log
                    .topics
                    .iter()
                    .cloned()
                    .map(|topic| SymbolicAbiValue::FixedBytes {
                        bytes: word_bytes(topic).into(),
                        size: 32,
                    })
                    .collect();
                SymbolicAbiValue::Tuple {
                    elements: vec![
                        SymbolicAbiValue::Array { elements: topics },
                        SymbolicAbiValue::Bytes { len: log.data_len, bytes: log.data },
                        SymbolicAbiValue::Address {
                            word: SymWord::Concrete(address_word(log.emitter)),
                        },
                    ],
                }
            })
            .collect(),
    };
    SymReturnData::from_symbolic_bytes(encode_sequence(std::iter::once(&value)))
}

/// Implements the `recorded_logs_json_return_data` cheatcode runtime helper.
pub(crate) fn recorded_logs_json_return_data(
    logs: Vec<SymbolicLog>,
) -> Result<SymReturnData, SymbolicError> {
    let mut bytes = Vec::new();
    push_ascii(&mut bytes, "[");
    for (log_idx, log) in logs.into_iter().enumerate() {
        if log_idx > 0 {
            push_ascii(&mut bytes, ",");
        }
        push_ascii(&mut bytes, "{\"topics\":[");
        for (topic_idx, topic) in log.topics.iter().cloned().enumerate() {
            if topic_idx > 0 {
                push_ascii(&mut bytes, ",");
            }
            push_ascii(&mut bytes, "\"0x");
            push_hex_word(&mut bytes, topic);
            push_ascii(&mut bytes, "\"");
        }
        push_ascii(&mut bytes, "],\"data\":\"0x");

        let len = log
            .data_len
            .into_concrete("symbolic vm.getRecordedLogsJson data length")
            .and_then(|len| {
                if len > U256::from(usize::MAX) {
                    Err(SymbolicError::Unsupported("symbolic vm.getRecordedLogsJson data length"))
                } else {
                    Ok(len.to::<usize>())
                }
            })?;
        if len > log.data.len() {
            return Err(SymbolicError::Unsupported("symbolic vm.getRecordedLogsJson data length"));
        }
        for byte in log.data.iter().take(len).cloned() {
            push_hex_byte(&mut bytes, byte);
        }

        push_ascii(&mut bytes, "\",\"emitter\":\"");
        push_ascii(&mut bytes, &format!("{}", log.emitter));
        push_ascii(&mut bytes, "\"}");
    }
    push_ascii(&mut bytes, "]");
    Ok(abi_bytes_return(bytes))
}

/// Implements the `accesses_return_data` cheatcode runtime helper.
pub(crate) fn accesses_return_data(
    record: Option<&AccessRecord>,
    target: Address,
) -> SymReturnData {
    let reads = record.and_then(|record| record.reads.get(&target)).cloned().unwrap_or_default();
    let writes = record.and_then(|record| record.writes.get(&target)).cloned().unwrap_or_default();
    let values = [storage_slots_abi_array(reads), storage_slots_abi_array(writes)];
    SymReturnData::from_symbolic_bytes(encode_sequence(values.iter()))
}

/// Implements the `complete_cheatcode_call` cheatcode runtime helper.
pub(crate) fn complete_cheatcode_call(
    state: &mut PathState,
    out_offset: SymWord,
    out_size: &BoundedCopySize,
    return_data: SymReturnData,
) -> Result<(), SymbolicError> {
    state.return_data = return_data;
    let return_data = state.return_data.clone();
    state.memory.copy_call_output_offset(out_offset, out_size, &return_data)?;
    state.stack.push(SymWord::Concrete(U256::from(1)))?;
    Ok(())
}

/// Implements the `storage_slots_abi_array` cheatcode runtime helper.
pub(crate) fn storage_slots_abi_array(slots: Vec<SymWord>) -> SymbolicAbiValue {
    SymbolicAbiValue::Array {
        elements: slots
            .into_iter()
            .map(|slot| SymbolicAbiValue::FixedBytes { bytes: word_bytes(slot).into(), size: 32 })
            .collect(),
    }
}

/// Applies the `push_ascii` cheatcode runtime helper.
pub(crate) fn push_ascii(out: &mut Vec<SymWord>, value: &str) {
    out.extend(value.bytes().map(|byte| SymWord::Concrete(U256::from(byte))));
}

/// Applies the `push_hex_word` cheatcode runtime helper.
pub(crate) fn push_hex_word(out: &mut Vec<SymWord>, word: SymWord) {
    for byte in word_bytes(word) {
        push_hex_byte(out, byte);
    }
}

/// Applies the `push_hex_byte` cheatcode runtime helper.
pub(crate) fn push_hex_byte(out: &mut Vec<SymWord>, byte: SymWord) {
    let byte = low_byte(byte);
    let high = match byte.clone() {
        SymWord::Concrete(value) => SymWord::Concrete(U256::from(value.to::<u8>() >> 4)),
        byte => {
            SymWord::from_expr(Expr::op(ExprOp::Shr, byte.into_expr(), Expr::Const(U256::from(4))))
        }
    };
    let low = match byte {
        SymWord::Concrete(value) => SymWord::Concrete(U256::from(value.to::<u8>() & 0x0f)),
        byte => SymWord::from_expr(Expr::op(
            ExprOp::And,
            byte.into_expr(),
            Expr::Const(U256::from(0x0f)),
        )),
    };
    out.push(hex_nibble_ascii(high));
    out.push(hex_nibble_ascii(low));
}

/// Implements the `hex_nibble_ascii` cheatcode runtime helper.
pub(crate) fn hex_nibble_ascii(nibble: SymWord) -> SymWord {
    match low_byte(nibble) {
        SymWord::Concrete(value) => {
            let nibble = value.to::<u8>() & 0x0f;
            let byte = if nibble < 10 { b'0' + nibble } else { b'a' + (nibble - 10) };
            SymWord::Concrete(U256::from(byte))
        }
        nibble => {
            let nibble = nibble.into_expr();
            SymWord::from_expr(Expr::ite(
                BoolExpr::cmp(BoolExprOp::Ult, nibble.clone(), Expr::Const(U256::from(10))),
                Expr::op(ExprOp::Add, nibble.clone(), Expr::Const(U256::from(b'0'))),
                Expr::op(ExprOp::Add, nibble, Expr::Const(U256::from(b'a' - 10))),
            ))
        }
    }
}

/// Returns the `read_abi_word_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_word_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
) -> Result<SymWord, SymbolicError> {
    memory.load_word(args_offset + index * 32)
}

/// Returns the `read_abi_concrete_word_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_concrete_word_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<U256, SymbolicError> {
    read_abi_word_arg(memory, args_offset, index)?.into_concrete(reason)
}

/// Returns the `read_abi_constrained_word_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_constrained_word_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<U256, SymbolicError> {
    let word = read_abi_word_arg(&state.memory, args_offset, index)?;
    state.expect_constrained_word(word, reason)
}

/// Returns the `read_abi_constrained_address_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_constrained_address_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<Address, SymbolicError> {
    Ok(word_to_address(read_abi_constrained_word_arg(state, args_offset, index, reason)?))
}

/// Returns the `read_abi_address_or_symbolic_slot_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_address_or_symbolic_slot_arg(
    state: &mut PathState,
    args_offset: usize,
    index: usize,
) -> Result<Address, SymbolicError> {
    let word = read_abi_word_arg(&state.memory, args_offset, index)?;
    Ok(state.address_or_symbolic_slot(word))
}

/// Returns the `read_abi_address_word_or_symbolic_slot_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_address_word_or_symbolic_slot_arg(
    state: &mut PathState,
    args_offset: usize,
    index: usize,
) -> Result<(Address, SymWord), SymbolicError> {
    let word = read_abi_word_arg(&state.memory, args_offset, index)?;
    let address = state.address_or_symbolic_slot(word.clone());
    Ok((address, word))
}

/// Returns the `read_abi_address_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_address_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<Address, SymbolicError> {
    Ok(word_to_address(read_abi_concrete_word_arg(memory, args_offset, index, reason)?))
}

/// Returns the `read_abi_bool_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_bool_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<bool, SymbolicError> {
    Ok(!read_abi_concrete_word_arg(memory, args_offset, index, reason)?.is_zero())
}

/// Returns the `read_abi_u64_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_u64_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<u64, SymbolicError> {
    let value = read_abi_concrete_word_arg(memory, args_offset, index, reason)?;
    if value > U256::from(u64::MAX) {
        return Err(SymbolicError::Unsupported(reason));
    }
    Ok(value.to())
}

/// Returns the `read_abi_u32_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_u32_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<u32, SymbolicError> {
    let value = read_abi_concrete_word_arg(memory, args_offset, index, reason)?;
    if value > U256::from(u32::MAX) {
        return Err(SymbolicError::Unsupported(reason));
    }
    Ok(value.to())
}

/// Returns the `read_abi_bytes4_words_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_bytes4_words_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
) -> Vec<SymWord> {
    memory.read_bytes(args_offset + index * 32, 4)
}

/// Returns the `read_abi_dynamic_bytes_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_dynamic_bytes_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<Vec<u8>, SymbolicError> {
    let offset = read_abi_concrete_word_arg(memory, args_offset, index, reason)?.to::<usize>();
    let len = memory.load_word(args_offset + offset)?.into_usize(reason)?;
    memory.read_concrete(args_offset + offset + 32, len)
}

/// Returns the `read_abi_symbolic_dynamic_bytes_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_symbolic_dynamic_bytes_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    max_len: usize,
    reason: &'static str,
) -> Result<Vec<SymWord>, SymbolicError> {
    let offset = read_abi_word_arg(&state.memory, args_offset, index)?;
    let offset = state.expect_constrained_usize(offset, reason)?;
    let len_offset = args_offset.checked_add(offset).ok_or(SymbolicError::Unsupported(reason))?;
    let len = state.memory.load_word(len_offset)?;
    let len = state.expect_constrained_usize(len, reason)?;
    if len > max_len {
        return Err(SymbolicError::Unsupported(reason));
    }
    let data_offset = len_offset.checked_add(32).ok_or(SymbolicError::Unsupported(reason))?;
    Ok(state.memory.read_bytes(data_offset, len))
}

/// Returns the `read_abi_dynamic_return_data_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_dynamic_return_data_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    max_len: usize,
    reason: &'static str,
) -> Result<SymReturnData, SymbolicError> {
    Ok(SymReturnData::from_symbolic_bytes(read_abi_symbolic_dynamic_bytes_arg(
        state,
        args_offset,
        index,
        max_len,
        reason,
    )?))
}

/// Returns the `read_abi_symbolic_dynamic_bytes_array_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_symbolic_dynamic_bytes_array_arg(
    state: &PathState,
    args_offset: usize,
    index: usize,
    max_array_len: usize,
    max_bytes_len: usize,
) -> Result<Vec<SymReturnData>, SymbolicError> {
    let offset = read_abi_word_arg(&state.memory, args_offset, index)?;
    let offset = state.expect_constrained_usize(offset, "symbolic vm.mockCalls returns offset")?;
    let array_offset = args_offset
        .checked_add(offset)
        .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns offset"))?;
    let array_data_offset = array_offset
        .checked_add(32)
        .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns offset"))?;
    let len = state.memory.load_word(array_offset)?;
    let len = state.expect_constrained_usize(len, "symbolic vm.mockCalls returns length")?;
    if len > max_array_len {
        return Err(SymbolicError::Unsupported("symbolic vm.mockCalls returns length"));
    }

    let mut values = Vec::with_capacity(len);
    for value_idx in 0..len {
        let head_offset = array_offset
            .checked_add(32)
            .and_then(|offset| offset.checked_add(value_idx.saturating_mul(32)))
            .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns element offset"))?;
        let value_offset = state.memory.load_word(head_offset)?;
        let value_offset = state.expect_constrained_usize(
            value_offset,
            "symbolic vm.mockCalls returns element offset",
        )?;
        let len_offset = array_data_offset
            .checked_add(value_offset)
            .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns element offset"))?;
        let value_len = state.memory.load_word(len_offset)?;
        let value_len = state
            .expect_constrained_usize(value_len, "symbolic vm.mockCalls returns element length")?;
        if value_len > max_bytes_len {
            return Err(SymbolicError::Unsupported("symbolic vm.mockCalls returns element length"));
        }
        let data_offset = len_offset
            .checked_add(32)
            .ok_or(SymbolicError::Unsupported("symbolic vm.mockCalls returns element offset"))?;
        values.push(SymReturnData::from_symbolic_bytes(
            state.memory.read_bytes(data_offset, value_len),
        ));
    }

    Ok(values)
}

/// Returns the `read_abi_string_arg` cheatcode runtime helper result.
pub(crate) fn read_abi_string_arg(
    memory: &SymMemory,
    args_offset: usize,
    index: usize,
    reason: &'static str,
) -> Result<String, SymbolicError> {
    String::from_utf8(read_abi_dynamic_bytes_arg(memory, args_offset, index, reason)?)
        .map_err(|_| SymbolicError::Unsupported(reason))
}

/// Implements the `expected_revert_match_condition` cheatcode runtime helper.
pub(crate) fn expected_revert_match_condition(
    expected: &ExpectedRevert,
    reverter: Address,
    return_data: &SymReturnData,
) -> Option<BoolExpr> {
    let mut conditions = Vec::new();
    if let Some(expected_reverter) = &expected.reverter {
        conditions.push(address_match_condition(expected_reverter, reverter));
    }
    match &expected.data {
        ExpectedRevertData::Any => {}
        ExpectedRevertData::Prefix(prefix) => {
            if return_data.len() < prefix.len() {
                return None;
            }
            conditions.push(BoolExpr::cmp(
                BoolExprOp::Uge,
                return_data.len_expr(),
                Expr::Const(U256::from(prefix.len())),
            ));
            conditions.extend(prefix.iter().enumerate().map(|(offset, expected)| {
                BoolExpr::eq(return_data.byte(offset).into_expr(), expected.clone().into_expr())
            }));
        }
        ExpectedRevertData::Exact(data) => {
            if return_data.len() < data.len() {
                return None;
            }
            conditions
                .push(BoolExpr::eq(return_data.len_expr(), Expr::Const(U256::from(data.len()))));
            conditions.extend(data.iter().enumerate().map(|(offset, expected)| {
                BoolExpr::eq(return_data.byte(offset).into_expr(), expected.clone().into_expr())
            }));
        }
    }
    Some(BoolExpr::and(conditions))
}

/// Returns the `decode_cheatcode_args` cheatcode runtime helper result.
pub(crate) fn decode_cheatcode_args(
    state: &PathState,
    in_offset: usize,
    in_size: usize,
    tys: Vec<DynSolType>,
) -> Result<Vec<DynSolValue>, SymbolicError> {
    let data = state.memory.read_concrete(in_offset + 4, in_size.saturating_sub(4))?;
    let value = DynSolType::Tuple(tys)
        .abi_decode_sequence(&data)
        .map_err(|_| SymbolicError::Unsupported("symbolic cheatcode ABI decode"))?;
    let DynSolValue::Tuple(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    Ok(values)
}

/// Returns the `selector_has_string_reason` cheatcode runtime helper result.
pub(crate) const fn selector_has_string_reason(selector: [u8; 4]) -> bool {
    matches!(
        selector,
        assertEq_15Call::SELECTOR
            | assertEq_17Call::SELECTOR
            | assertEq_19Call::SELECTOR
            | assertEq_21Call::SELECTOR
            | assertEq_23Call::SELECTOR
            | assertEq_25Call::SELECTOR
            | assertEq_27Call::SELECTOR
            | assertNotEq_15Call::SELECTOR
            | assertNotEq_17Call::SELECTOR
            | assertNotEq_19Call::SELECTOR
            | assertNotEq_21Call::SELECTOR
            | assertNotEq_23Call::SELECTOR
            | assertNotEq_25Call::SELECTOR
            | assertNotEq_27Call::SELECTOR
            | assertEqDecimal_1Call::SELECTOR
            | assertEqDecimal_3Call::SELECTOR
    )
}

/// Implements the `array_assertion_element_type` cheatcode runtime helper.
pub(crate) const fn array_assertion_element_type(
    selector: [u8; 4],
) -> Result<DynSolType, SymbolicError> {
    match selector {
        assertEq_14Call::SELECTOR
        | assertEq_15Call::SELECTOR
        | assertNotEq_14Call::SELECTOR
        | assertNotEq_15Call::SELECTOR => Ok(DynSolType::Bool),
        assertEq_16Call::SELECTOR
        | assertEq_17Call::SELECTOR
        | assertNotEq_16Call::SELECTOR
        | assertNotEq_17Call::SELECTOR => Ok(DynSolType::Uint(256)),
        assertEq_18Call::SELECTOR
        | assertEq_19Call::SELECTOR
        | assertNotEq_18Call::SELECTOR
        | assertNotEq_19Call::SELECTOR => Ok(DynSolType::Int(256)),
        assertEq_20Call::SELECTOR
        | assertEq_21Call::SELECTOR
        | assertNotEq_20Call::SELECTOR
        | assertNotEq_21Call::SELECTOR => Ok(DynSolType::Address),
        assertEq_22Call::SELECTOR
        | assertEq_23Call::SELECTOR
        | assertNotEq_22Call::SELECTOR
        | assertNotEq_23Call::SELECTOR => Ok(DynSolType::FixedBytes(32)),
        assertEq_24Call::SELECTOR
        | assertEq_25Call::SELECTOR
        | assertNotEq_24Call::SELECTOR
        | assertNotEq_25Call::SELECTOR => Ok(DynSolType::String),
        assertEq_26Call::SELECTOR
        | assertEq_27Call::SELECTOR
        | assertNotEq_26Call::SELECTOR
        | assertNotEq_27Call::SELECTOR => Ok(DynSolType::Bytes),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

/// Returns the `dyn_string` cheatcode runtime helper result.
pub(crate) fn dyn_string(value: &DynSolValue) -> Result<String, SymbolicError> {
    match value {
        DynSolValue::String(value) => Ok(value.clone()),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

/// Returns the `dyn_bytes` cheatcode runtime helper result.
pub(crate) fn dyn_bytes(value: &DynSolValue) -> Result<Vec<u8>, SymbolicError> {
    match value {
        DynSolValue::Bytes(value) => Ok(value.clone()),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

/// Returns the `dyn_bool` cheatcode runtime helper result.
pub(crate) const fn dyn_bool(value: &DynSolValue) -> Result<bool, SymbolicError> {
    match value {
        DynSolValue::Bool(value) => Ok(*value),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

/// Returns the `dyn_address` cheatcode runtime helper result.
pub(crate) const fn dyn_address(value: &DynSolValue) -> Result<Address, SymbolicError> {
    match value {
        DynSolValue::Address(value) => Ok(*value),
        _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
    }
}

/// Returns the `dyn_potential_revert` cheatcode runtime helper result.
pub(crate) fn dyn_potential_revert(value: &DynSolValue) -> Result<ExpectedRevert, SymbolicError> {
    let DynSolValue::Tuple(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"));
    };
    let [reverter, partial_match, revert_data] = values.as_slice() else {
        return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"));
    };

    let reverter = dyn_address(reverter)?;
    let reverter = (reverter != Address::ZERO).then(|| SymWord::Concrete(address_word(reverter)));
    let revert_data = dyn_bytes(revert_data)?
        .into_iter()
        .map(|byte| SymWord::Concrete(U256::from(byte)))
        .collect();
    let data = if dyn_bool(partial_match)? {
        ExpectedRevertData::prefix(revert_data)
    } else {
        ExpectedRevertData::exact(revert_data)
    };
    Ok(ExpectedRevert { data, reverter, remaining: 1 })
}

/// Returns the `dyn_potential_reverts` cheatcode runtime helper result.
pub(crate) fn dyn_potential_reverts(
    value: &DynSolValue,
) -> Result<Vec<ExpectedRevert>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic vm.assumeNoRevert decode"));
    };
    values.iter().map(dyn_potential_revert).collect()
}

/// Returns the `dyn_address_array` cheatcode runtime helper result.
pub(crate) fn dyn_address_array(value: &DynSolValue) -> Result<Vec<Address>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    values.iter().map(dyn_address).collect()
}

/// Returns the `dyn_bytes32_array` cheatcode runtime helper result.
pub(crate) fn dyn_bytes32_array(value: &DynSolValue) -> Result<Vec<B256>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    values
        .iter()
        .map(|value| match value {
            DynSolValue::FixedBytes(bytes, 32) => Ok(*bytes),
            _ => Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode")),
        })
        .collect()
}

/// Returns the `dyn_string_array` cheatcode runtime helper result.
pub(crate) fn dyn_string_array(value: &DynSolValue) -> Result<Vec<String>, SymbolicError> {
    let DynSolValue::Array(values) = value else {
        return Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"));
    };
    values.iter().map(dyn_string).collect()
}

/// Returns the `parse_env_array` cheatcode runtime helper result.
pub(crate) fn parse_env_array<F>(
    value: &str,
    delimiter: &str,
    mut parser: F,
) -> Result<DynSolValue, SymbolicError>
where
    F: FnMut(&str) -> Result<DynSolValue, SymbolicError>,
{
    if delimiter.is_empty() {
        return Err(SymbolicError::Unsupported("symbolic env delimiter"));
    }
    value.split(delimiter).map(&mut parser).collect::<Result<Vec<_>, _>>().map(DynSolValue::Array)
}

/// Returns the `parse_env_bool_value` cheatcode runtime helper result.
pub(crate) fn parse_env_bool_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Bool(parse_env_bool(value)?))
}

/// Returns the `parse_env_uint_value` cheatcode runtime helper result.
pub(crate) fn parse_env_uint_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Uint(parse_env_uint(value)?, 256))
}

/// Returns the `parse_env_int_value` cheatcode runtime helper result.
pub(crate) fn parse_env_int_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Int(I256::from_raw(parse_env_int(value)?), 256))
}

/// Returns the `parse_env_address_value` cheatcode runtime helper result.
pub(crate) fn parse_env_address_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Address(parse_env_address(value)?))
}

/// Returns the `parse_env_bytes32_value` cheatcode runtime helper result.
pub(crate) fn parse_env_bytes32_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::FixedBytes(B256::from(parse_env_bytes32(value)?.to_be_bytes::<32>()), 32))
}

/// Returns the `parse_env_string_value` cheatcode runtime helper result.
pub(crate) fn parse_env_string_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::String(value.to_string()))
}

/// Returns the `parse_env_bytes_value` cheatcode runtime helper result.
pub(crate) fn parse_env_bytes_value(value: &str) -> Result<DynSolValue, SymbolicError> {
    Ok(DynSolValue::Bytes(parse_env_bytes(value)?))
}

/// Returns the `parse_env_uint` cheatcode runtime helper result.
pub(crate) fn parse_env_uint(value: &str) -> Result<U256, SymbolicError> {
    value.parse::<U256>().map_err(|_| SymbolicError::Unsupported("symbolic env uint parse"))
}

/// Returns the `parse_env_int` cheatcode runtime helper result.
pub(crate) fn parse_env_int(value: &str) -> Result<U256, SymbolicError> {
    if let Some(value) = value.strip_prefix('-') {
        let magnitude = parse_env_uint(value)?;
        Ok(U256::ZERO.wrapping_sub(magnitude))
    } else {
        parse_env_uint(value)
    }
}

/// Returns the `parse_env_bool` cheatcode runtime helper result.
pub(crate) fn parse_env_bool(value: &str) -> Result<bool, SymbolicError> {
    match value {
        "true" | "1" | "TRUE" | "True" => Ok(true),
        "false" | "0" | "FALSE" | "False" => Ok(false),
        _ => Err(SymbolicError::Unsupported("symbolic env bool parse")),
    }
}

/// Returns the `parse_env_bytes` cheatcode runtime helper result.
pub(crate) fn parse_env_bytes(value: &str) -> Result<Vec<u8>, SymbolicError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    hex::decode(value).map_err(|_| SymbolicError::Unsupported("symbolic env bytes parse"))
}

/// Returns the `parse_env_bytes32` cheatcode runtime helper result.
pub(crate) fn parse_env_bytes32(value: &str) -> Result<U256, SymbolicError> {
    let bytes = parse_env_bytes(value)?;
    if bytes.len() != 32 {
        return Err(SymbolicError::Unsupported("symbolic env bytes32 parse"));
    }
    Ok(U256::from_be_slice(&bytes))
}

/// Returns the `parse_env_address` cheatcode runtime helper result.
pub(crate) fn parse_env_address(value: &str) -> Result<Address, SymbolicError> {
    value.parse::<Address>().map_err(|_| SymbolicError::Unsupported("symbolic env address parse"))
}

/// Implements the `private_key_signer` cheatcode runtime helper.
pub(crate) fn private_key_signer(private_key: U256) -> Result<PrivateKeySigner, SymbolicError> {
    private_key_from_u256(private_key)
        .map_err(|_| SymbolicError::Unsupported("symbolic private key parse"))
}

/// Implements the `private_key_address` cheatcode runtime helper.
pub(crate) fn private_key_address(private_key: U256) -> Result<Address, SymbolicError> {
    Ok(private_key_signer(private_key)?.address())
}

/// Implements the `sign_hash_words` cheatcode runtime helper.
pub(crate) fn sign_hash_words(
    private_key: U256,
    digest: U256,
) -> Result<Vec<SymWord>, SymbolicError> {
    let signer = private_key_signer(private_key)?;
    let digest = B256::from(digest.to_be_bytes::<32>());
    let sig = signer
        .sign_hash_sync(&digest)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.sign"))?;
    Ok(vec![
        SymWord::Concrete(U256::from(sig.v() as u64 + 27)),
        SymWord::Concrete(sig.r()),
        SymWord::Concrete(sig.s()),
    ])
}

/// Implements the `sign_compact_hash_words` cheatcode runtime helper.
pub(crate) fn sign_compact_hash_words(
    private_key: U256,
    digest: U256,
) -> Result<Vec<SymWord>, SymbolicError> {
    let signer = private_key_signer(private_key)?;
    let digest = B256::from(digest.to_be_bytes::<32>());
    let sig = signer
        .sign_hash_sync(&digest)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.signCompact"))?;
    let y_parity = U256::from(sig.v() as u64) << 255;
    Ok(vec![SymWord::Concrete(sig.r()), SymWord::Concrete(sig.s() | y_parity)])
}

/// Computes the `derive_private_key` cheatcode runtime helper result.
pub(crate) fn derive_private_key<W: Wordlist>(
    mnemonic: &str,
    path: &str,
    index: u32,
) -> Result<U256, SymbolicError> {
    foundry_common::wallet::derive_private_key::<W>(mnemonic, path, index)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.deriveKey"))
}

/// Computes the `derive_private_key_with_language` cheatcode runtime helper result.
pub(crate) fn derive_private_key_with_language(
    mnemonic: &str,
    path: &str,
    index: u32,
    language: &str,
) -> Result<U256, SymbolicError> {
    foundry_common::wallet::derive_private_key_with_language(mnemonic, path, index, language)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.deriveKey language"))
}

/// Implements the `artifact_json_path` cheatcode runtime helper.
pub(crate) fn artifact_json_path(path: &str) -> PathBuf {
    if path.ends_with(".json") {
        return PathBuf::from(path);
    }

    let mut parts = path.split(':');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();

    if first.contains('.') {
        let file = Path::new(first);
        let contract = second
            .map(str::to_string)
            .or_else(|| file.file_stem().map(|stem| stem.to_string_lossy().to_string()))
            .unwrap_or_else(|| first.to_string());
        PathBuf::from("out").join(first).join(format!("{contract}.json"))
    } else {
        let contract = first;
        PathBuf::from("out").join(format!("{contract}.sol")).join(format!("{contract}.json"))
    }
}

/// Returns fallback paths for symbolic artifact lookup.
pub(crate) fn artifact_json_fallback_paths(path: &str) -> Vec<PathBuf> {
    if path.ends_with(".json") {
        return Vec::new();
    }

    let mut parts = path.split(':');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    let Some(file_name) = first.rsplit(['/', '\\']).find(|part| !part.is_empty()) else {
        return Vec::new();
    };

    if !first.contains('/') && !first.contains('\\') {
        return Vec::new();
    }

    let contract = second.map(str::to_string).unwrap_or_else(|| {
        file_name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file_name).to_string()
    });
    vec![PathBuf::from("out").join(file_name).join(format!("{contract}.json"))]
}

/// Implements the `artifact_code` cheatcode runtime helper.
pub(crate) fn artifact_code(path: &str, deployed: bool) -> Result<Vec<u8>, SymbolicError> {
    let mut data = None;
    for path in std::iter::once(artifact_json_path(path)).chain(artifact_json_fallback_paths(path))
    {
        if let Ok(contents) = std::fs::read_to_string(path) {
            data = Some(contents);
            break;
        }
    }
    let data = data.ok_or(SymbolicError::Unsupported("symbolic vm.getCode artifact"))?;
    let artifact: serde_json::Value = serde_json::from_str(&data)
        .map_err(|_| SymbolicError::Unsupported("symbolic vm.getCode artifact"))?;
    let key = if deployed { "deployedBytecode" } else { "bytecode" };
    let object = artifact
        .get(key)
        .and_then(|value| value.get("object").or(Some(value)))
        .and_then(serde_json::Value::as_str)
        .ok_or(SymbolicError::Unsupported("symbolic vm.getCode artifact"))?;
    hex::decode(object).map_err(|_| SymbolicError::Unsupported("symbolic vm.getCode artifact"))
}
