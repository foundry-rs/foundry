use foundry_common::wallet::private_key_from_u256;

use super::*;

/// Implements the `foundry_cheatcode_min_input_size` cheatcode runtime helper.
pub(crate) fn foundry_cheatcode_min_input_size(selector: [u8; 4]) -> Option<usize> {
    if selector_in(
        selector,
        &[
            "recordLogs()",
            "record()",
            "stopRecord()",
            "assumeNoRevert()",
            "getRecordedLogs()",
            "getRecordedLogsJson()",
            "expectRevert()",
            "expectEmit()",
            "expectEmitAnonymous()",
            "clearMockedCalls()",
            "stopPrank()",
            "readCallers()",
            "getWallets()",
            "snapshot()",
            "snapshotState()",
            "deleteSnapshots()",
            "deleteStateSnapshots()",
            "activeFork()",
            "getChainId()",
            "getBlobhashes()",
            "getBlobBaseFee()",
            "getBlockNumber()",
            "getBlockTimestamp()",
            "pauseGasMetering()",
            "resumeGasMetering()",
            "resetGasMetering()",
            "lastCallGas()",
            "lastFrameGas()",
            "stopExpectSafeMemory()",
            "stopSnapshotGas()",
            "getEvmVersion()",
            "getFoundryVersion()",
            "projectRoot()",
            "unixTime()",
            "noAccessList()",
            "randomUint()",
            "randomInt()",
            "randomAddress()",
            "randomBool()",
            "randomBytes4()",
            "randomBytes8()",
        ],
    ) {
        return Some(abi_static_input_size(0));
    }

    if selector_in(
        selector,
        &[
            "assume(bool)",
            "skip(bool)",
            "skip(bool,string)",
            "assumeNoRevert((address,bool,bytes))",
            "assumeNoRevert((address,bool,bytes)[])",
            "accesses(address)",
            "expectRevert(bytes4)",
            "expectRevert(address)",
            "expectRevert(uint64)",
            "expectPartialRevert(bytes4)",
            "expectEmit(address)",
            "expectEmit(uint64)",
            "expectEmitAnonymous(address)",
            "prank(address)",
            "startPrank(address)",
            "addr(uint256)",
            "rememberKey(uint256)",
            "toString(address)",
            "toString(bytes)",
            "toString(bytes32)",
            "toString(bool)",
            "toString(uint256)",
            "toString(int256)",
            "parseBytes(string)",
            "parseAddress(string)",
            "parseUint(string)",
            "parseInt(string)",
            "parseBytes32(string)",
            "parseBool(string)",
            "toLowercase(string)",
            "toUppercase(string)",
            "trim(string)",
            "toBase64(bytes)",
            "toBase64(string)",
            "toBase64URL(bytes)",
            "toBase64URL(string)",
            "breakpoint(string)",
            "setEvmVersion(string)",
            "sleep(uint256)",
            "accessList((address,bytes32[])[])",
            "getNonce(address)",
            "resetNonce(address)",
            "allowCheatcodes(address)",
            "makePersistent(address)",
            "makePersistent(address[])",
            "revokePersistent(address)",
            "revokePersistent(address[])",
            "isPersistent(address)",
            "selectFork(uint256)",
            "createFork(string)",
            "createSelectFork(string)",
            "rollFork(uint256)",
            "rollFork(bytes32)",
            "revertTo(uint256)",
            "revertToState(uint256)",
            "revertToAndDelete(uint256)",
            "revertToStateAndDelete(uint256)",
            "deleteSnapshot(uint256)",
            "deleteStateSnapshot(uint256)",
            "warp(uint256)",
            "roll(uint256)",
            "prevrandao(bytes32)",
            "prevrandao(uint256)",
            "fee(uint256)",
            "blobBaseFee(uint256)",
            "chainId(uint256)",
            "difficulty(uint256)",
            "coinbase(address)",
            "txGasPrice(uint256)",
            "getLabel(address)",
            "snapshotGasLastCall(string)",
            "snapshotGasLastFrame(string)",
            "startSnapshotGas(string)",
            "stopSnapshotGas(string)",
            "cool(address)",
            "isContext(uint8)",
            "assertTrue(bool)",
            "assertTrue(bool,string)",
            "assertFalse(bool)",
            "assertFalse(bool,string)",
            "randomUint(uint256)",
            "randomInt(uint256)",
            "randomBytes(uint256)",
        ],
    ) {
        return Some(abi_static_input_size(1));
    }

    if selector_in(
        selector,
        &[
            "expectRevert(bytes4,address)",
            "expectRevert(bytes4,uint64)",
            "expectRevert(address,uint64)",
            "expectPartialRevert(bytes4,address)",
            "expectEmit(address,uint64)",
            "prank(address,address)",
            "prank(address,bool)",
            "startPrank(address,address)",
            "startPrank(address,bool)",
            "sign(uint256,bytes32)",
            "signCompact(uint256,bytes32)",
            "deriveKey(string,uint32)",
            "split(string,string)",
            "indexOf(string,string)",
            "contains(string,string)",
            "breakpoint(string,bool)",
            "expectSafeMemory(uint64,uint64)",
            "expectSafeMemoryCall(uint64,uint64)",
            "load(address,bytes32)",
            "makePersistent(address,address)",
            "computeCreateAddress(address,uint256)",
            "computeCreate2Address(bytes32,bytes32)",
            "setBlockhash(uint256,bytes32)",
            "deal(address,uint256)",
            "setNonce(address,uint64)",
            "setNonceUnsafe(address,uint64)",
            "createFork(string,uint256)",
            "createFork(string,bytes32)",
            "createSelectFork(string,uint256)",
            "createSelectFork(string,bytes32)",
            "rollFork(uint256,uint256)",
            "rollFork(uint256,bytes32)",
            "label(address,string)",
            "snapshotValue(string,uint256)",
            "snapshotGasLastCall(string,string)",
            "snapshotGasLastFrame(string,string)",
            "startSnapshotGas(string,string)",
            "stopSnapshotGas(string,string)",
            "warmSlot(address,bytes32)",
            "coolSlot(address,bytes32)",
            "assertEq(uint256,uint256)",
            "assertEq(uint256,uint256,string)",
            "assertEq(int256,int256)",
            "assertEq(int256,int256,string)",
            "assertEq(address,address)",
            "assertEq(address,address,string)",
            "assertEq(bytes32,bytes32)",
            "assertEq(bytes32,bytes32,string)",
            "assertEq(bool,bool)",
            "assertEq(bool,bool,string)",
            "assertNotEq(uint256,uint256)",
            "assertNotEq(uint256,uint256,string)",
            "assertNotEq(int256,int256)",
            "assertNotEq(int256,int256,string)",
            "assertNotEq(address,address)",
            "assertNotEq(address,address,string)",
            "assertNotEq(bytes32,bytes32)",
            "assertNotEq(bytes32,bytes32,string)",
            "assertNotEq(bool,bool)",
            "assertNotEq(bool,bool,string)",
            "assertLt(uint256,uint256)",
            "assertLt(uint256,uint256,string)",
            "assertLe(uint256,uint256)",
            "assertLe(uint256,uint256,string)",
            "assertGt(uint256,uint256)",
            "assertGt(uint256,uint256,string)",
            "assertGe(uint256,uint256)",
            "assertGe(uint256,uint256,string)",
            "assertLt(int256,int256)",
            "assertLt(int256,int256,string)",
            "assertGt(int256,int256)",
            "assertGt(int256,int256,string)",
            "assertLe(int256,int256)",
            "assertLe(int256,int256,string)",
            "assertGe(int256,int256)",
            "assertGe(int256,int256,string)",
            "assertEq(bool[],bool[])",
            "assertEq(bool[],bool[],string)",
            "assertEq(uint256[],uint256[])",
            "assertEq(uint256[],uint256[],string)",
            "assertEq(int256[],int256[])",
            "assertEq(int256[],int256[],string)",
            "assertEq(address[],address[])",
            "assertEq(address[],address[],string)",
            "assertEq(bytes32[],bytes32[])",
            "assertEq(bytes32[],bytes32[],string)",
            "assertEq(string[],string[])",
            "assertEq(string[],string[],string)",
            "assertEq(bytes[],bytes[])",
            "assertEq(bytes[],bytes[],string)",
            "assertNotEq(bool[],bool[])",
            "assertNotEq(bool[],bool[],string)",
            "assertNotEq(uint256[],uint256[])",
            "assertNotEq(uint256[],uint256[],string)",
            "assertNotEq(int256[],int256[])",
            "assertNotEq(int256[],int256[],string)",
            "assertNotEq(address[],address[])",
            "assertNotEq(address[],address[],string)",
            "assertNotEq(bytes32[],bytes32[])",
            "assertNotEq(bytes32[],bytes32[],string)",
            "assertNotEq(string[],string[])",
            "assertNotEq(string[],string[],string)",
            "assertNotEq(bytes[],bytes[])",
            "assertNotEq(bytes[],bytes[],string)",
            "randomUint(uint256,uint256)",
        ],
    ) {
        return Some(abi_static_input_size(2));
    }

    if selector_in(
        selector,
        &[
            "expectRevert(bytes4,address,uint64)",
            "deriveKey(string,string,uint32)",
            "deriveKey(string,uint32,string)",
            "rememberKeys(string,string,uint32)",
            "store(address,bytes32,bytes32)",
            "makePersistent(address,address,address)",
            "snapshotValue(string,string,uint256)",
            "replace(string,string,string)",
            "assertEqDecimal(uint256,uint256,uint256)",
            "assertEqDecimal(int256,int256,uint256)",
            "computeCreate2Address(bytes32,bytes32,address)",
            "bound(uint256,uint256,uint256)",
            "bound(int256,int256,int256)",
        ],
    ) {
        return Some(abi_static_input_size(3));
    }

    if selector_in(
        selector,
        &[
            "expectEmit(bool,bool,bool,bool)",
            "deriveKey(string,string,uint32,string)",
            "rememberKeys(string,string,string,uint32)",
            "assertEqDecimal(uint256,uint256,uint256,string)",
            "assertEqDecimal(int256,int256,uint256,string)",
        ],
    ) {
        return Some(abi_static_input_size(4));
    }
    if selector_in(selector, &["expectEmitAnonymous(bool,bool,bool,bool,bool)"]) {
        return Some(abi_static_input_size(5));
    }
    if selector_in(
        selector,
        &["expectEmit(bool,bool,bool,bool,address)", "expectEmit(bool,bool,bool,bool,uint64)"],
    ) {
        return Some(abi_static_input_size(5));
    }
    if selector_in(
        selector,
        &[
            "expectEmit(bool,bool,bool,bool,address,uint64)",
            "expectEmitAnonymous(bool,bool,bool,bool,bool,address)",
        ],
    ) {
        return Some(abi_static_input_size(6));
    }

    None
}

/// Returns the `symbolic_vm_cheatcode_min_input_size` cheatcode runtime helper result.
pub(crate) fn symbolic_vm_cheatcode_min_input_size(selector: [u8; 4]) -> Option<usize> {
    if selector_in(
        selector,
        &[
            "createUint256(string)",
            "createInt256(string)",
            "createBytes32(string)",
            "createAddress(string)",
            "createBool(string)",
            "createBytes(string)",
            "createString(string)",
            "createBytes4(string)",
            "createCalldata(string)",
            "snapshotState()",
        ],
    ) || (8..=256).step_by(8).any(|bits| {
        selector == selector_for(&format!("createUint{bits}(string)"))
            || selector == selector_for(&format!("createInt{bits}(string)"))
    }) || (1..=32).any(|bytes| selector == selector_for(&format!("createBytes{bytes}(string)")))
    {
        return Some(abi_static_input_size(0));
    }

    if selector_in(
        selector,
        &[
            "enableSymbolicStorage(address)",
            "setArbitraryStorage(address)",
            "snapshotStorage(address)",
        ],
    ) {
        return Some(abi_static_input_size(1));
    }
    if selector_in(
        selector,
        &[
            "createUint(uint256,string)",
            "createInt(uint256,string)",
            "createBytes(uint256,string)",
            "createString(uint256,string)",
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
pub(crate) fn selector_in(selector: [u8; 4], signatures: &[&str]) -> bool {
    signatures.iter().any(|signature| selector == selector_for(signature))
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
                    .into_iter()
                    .map(|topic| SymbolicAbiValue::FixedBytes {
                        bytes: word_bytes(topic),
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
        for (topic_idx, topic) in log.topics.into_iter().enumerate() {
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
        for byte in log.data.into_iter().take(len) {
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
            .map(|slot| SymbolicAbiValue::FixedBytes { bytes: word_bytes(slot), size: 32 })
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
        byte => SymWord::Expr(Expr::op(ExprOp::Shr, byte.into_expr(), Expr::Const(U256::from(4)))),
    };
    let low = match byte {
        SymWord::Concrete(value) => SymWord::Concrete(U256::from(value.to::<u8>() & 0x0f)),
        byte => {
            SymWord::Expr(Expr::op(ExprOp::And, byte.into_expr(), Expr::Const(U256::from(0x0f))))
        }
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
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::cmp(
                    BoolExprOp::Ult,
                    nibble.clone(),
                    Expr::Const(U256::from(10)),
                )),
                Box::new(Expr::op(ExprOp::Add, nibble.clone(), Expr::Const(U256::from(b'0')))),
                Box::new(Expr::op(ExprOp::Add, nibble, Expr::Const(U256::from(b'a' - 10)))),
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
            if return_data.len < prefix.len() {
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
            if return_data.len < data.len() {
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
pub(crate) fn selector_has_string_reason(selector: [u8; 4]) -> bool {
    selector_in(
        selector,
        &[
            "assertEq(bool[],bool[],string)",
            "assertEq(uint256[],uint256[],string)",
            "assertEq(int256[],int256[],string)",
            "assertEq(address[],address[],string)",
            "assertEq(bytes32[],bytes32[],string)",
            "assertEq(string[],string[],string)",
            "assertEq(bytes[],bytes[],string)",
            "assertNotEq(bool[],bool[],string)",
            "assertNotEq(uint256[],uint256[],string)",
            "assertNotEq(int256[],int256[],string)",
            "assertNotEq(address[],address[],string)",
            "assertNotEq(bytes32[],bytes32[],string)",
            "assertNotEq(string[],string[],string)",
            "assertNotEq(bytes[],bytes[],string)",
            "assertEqDecimal(uint256,uint256,uint256,string)",
            "assertEqDecimal(int256,int256,uint256,string)",
        ],
    )
}

/// Implements the `array_assertion_element_type` cheatcode runtime helper.
pub(crate) fn array_assertion_element_type(selector: [u8; 4]) -> Result<DynSolType, SymbolicError> {
    if selector_in(
        selector,
        &[
            "assertEq(bool[],bool[])",
            "assertEq(bool[],bool[],string)",
            "assertNotEq(bool[],bool[])",
            "assertNotEq(bool[],bool[],string)",
        ],
    ) {
        return Ok(DynSolType::Bool);
    }
    if selector_in(
        selector,
        &[
            "assertEq(uint256[],uint256[])",
            "assertEq(uint256[],uint256[],string)",
            "assertNotEq(uint256[],uint256[])",
            "assertNotEq(uint256[],uint256[],string)",
        ],
    ) {
        return Ok(DynSolType::Uint(256));
    }
    if selector_in(
        selector,
        &[
            "assertEq(int256[],int256[])",
            "assertEq(int256[],int256[],string)",
            "assertNotEq(int256[],int256[])",
            "assertNotEq(int256[],int256[],string)",
        ],
    ) {
        return Ok(DynSolType::Int(256));
    }
    if selector_in(
        selector,
        &[
            "assertEq(address[],address[])",
            "assertEq(address[],address[],string)",
            "assertNotEq(address[],address[])",
            "assertNotEq(address[],address[],string)",
        ],
    ) {
        return Ok(DynSolType::Address);
    }
    if selector_in(
        selector,
        &[
            "assertEq(bytes32[],bytes32[])",
            "assertEq(bytes32[],bytes32[],string)",
            "assertNotEq(bytes32[],bytes32[])",
            "assertNotEq(bytes32[],bytes32[],string)",
        ],
    ) {
        return Ok(DynSolType::FixedBytes(32));
    }
    if selector_in(
        selector,
        &[
            "assertEq(string[],string[])",
            "assertEq(string[],string[],string)",
            "assertNotEq(string[],string[])",
            "assertNotEq(string[],string[],string)",
        ],
    ) {
        return Ok(DynSolType::String);
    }
    if selector_in(
        selector,
        &[
            "assertEq(bytes[],bytes[])",
            "assertEq(bytes[],bytes[],string)",
            "assertNotEq(bytes[],bytes[])",
            "assertNotEq(bytes[],bytes[],string)",
        ],
    ) {
        return Ok(DynSolType::Bytes);
    }
    Err(SymbolicError::Unsupported("symbolic cheatcode ABI decode"))
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
        ExpectedRevertData::Prefix(revert_data)
    } else {
        ExpectedRevertData::Exact(revert_data)
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
