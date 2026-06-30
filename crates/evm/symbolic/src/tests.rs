use super::{abi::*, runtime::*, *};

fn empty_state() -> PathState {
    PathState::new(
        Address::ZERO,
        Address::ZERO,
        U256::ZERO,
        SymbolicCalldata::selector_only(&Function::parse("empty()").unwrap()).unwrap(),
        false,
    )
}

fn add_words(left: SymExpr, right: SymExpr) -> SymExpr {
    SymExpr::op(SymExprOp::Add, left, right)
}

fn precompile_address(index: u8) -> Address {
    let mut bytes = [0u8; 20];
    bytes[19] = index;
    Address::from(bytes)
}

#[test]
fn precompile_number_respects_active_spec() {
    for number in 1..=4 {
        assert_eq!(
            precompile_number_for_spec(precompile_address(number), SpecId::FRONTIER),
            Some(number)
        );
    }

    for number in 5..=8 {
        assert_eq!(precompile_number_for_spec(precompile_address(number), SpecId::FRONTIER), None);
        assert_eq!(
            precompile_number_for_spec(precompile_address(number), SpecId::BYZANTIUM),
            Some(number)
        );
    }

    assert_eq!(precompile_number_for_spec(precompile_address(9), SpecId::BYZANTIUM), None);
    assert_eq!(precompile_number_for_spec(precompile_address(9), SpecId::ISTANBUL), Some(9));

    assert_eq!(precompile_number_for_spec(precompile_address(10), SpecId::ISTANBUL), None);
    assert_eq!(precompile_number_for_spec(precompile_address(10), SpecId::CANCUN), Some(10));
}

#[test]
fn pop_worklist_respects_exploration_order() {
    let mut bfs_worklist = VecDeque::from([1, 2, 3]);
    assert_eq!(
        super::executor::pop_worklist(&mut bfs_worklist, SymbolicExplorationOrder::Bfs),
        Some(1)
    );
    assert_eq!(
        super::executor::pop_worklist(&mut bfs_worklist, SymbolicExplorationOrder::Bfs),
        Some(2)
    );

    let mut dfs_worklist = VecDeque::from([1, 2, 3]);
    assert_eq!(
        super::executor::pop_worklist(&mut dfs_worklist, SymbolicExplorationOrder::Dfs),
        Some(3)
    );
    assert_eq!(
        super::executor::pop_worklist(&mut dfs_worklist, SymbolicExplorationOrder::Dfs),
        Some(2)
    );
}

#[test]
fn local_batches_respect_exploration_order() {
    let mut bfs_batch = VecDeque::from([1, 2, 3]);
    let mut bfs_worklist = VecDeque::from([10]);
    assert_eq!(super::executor::pop_batch(&mut bfs_batch, SymbolicExplorationOrder::Bfs), Some(1));
    super::executor::spill_batch(bfs_batch, &mut bfs_worklist, SymbolicExplorationOrder::Bfs);
    assert_eq!(
        super::executor::pop_worklist(&mut bfs_worklist, SymbolicExplorationOrder::Bfs),
        Some(10)
    );
    assert_eq!(
        super::executor::pop_worklist(&mut bfs_worklist, SymbolicExplorationOrder::Bfs),
        Some(2)
    );

    let mut dfs_batch = VecDeque::from([1, 2, 3]);
    let mut dfs_worklist = VecDeque::from([10]);
    assert_eq!(super::executor::pop_batch(&mut dfs_batch, SymbolicExplorationOrder::Dfs), Some(3));
    super::executor::spill_batch(dfs_batch, &mut dfs_worklist, SymbolicExplorationOrder::Dfs);
    assert_eq!(
        super::executor::pop_worklist(&mut dfs_worklist, SymbolicExplorationOrder::Dfs),
        Some(2)
    );
    assert_eq!(
        super::executor::pop_worklist(&mut dfs_worklist, SymbolicExplorationOrder::Dfs),
        Some(1)
    );
}

#[test]
fn binary_helpers_use_evm_operand_order() {
    let mut state = empty_state();
    state.stack.push(SymExpr::constant(U256::from(2))).unwrap();
    state.stack.push(SymExpr::constant(U256::from(10))).unwrap();

    state.bin_word(SymExprOp::Sub).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymExpr::constant(U256::from(8)));
}

#[test]
fn comparison_helpers_use_evm_operand_order() {
    let mut state = empty_state();
    state.stack.push(SymExpr::constant(U256::from(2))).unwrap();
    state.stack.push(SymExpr::constant(U256::from(10))).unwrap();

    state.cmp_word(SymBoolExprOp::Ult).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymExpr::constant(U256::ZERO));
}

#[test]
fn exp_helper_uses_evm_operand_order() {
    let mut state = empty_state();
    state.stack.push(SymExpr::constant(U256::ZERO)).unwrap();
    state.stack.push(SymExpr::constant(U256::from(0x100))).unwrap();

    state.exp_word().unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymExpr::constant(U256::from(1)));
}

#[test]
fn exp_helper_expands_symbolic_base_for_bounded_concrete_exponent() {
    let mut state = empty_state();
    state.stack.push(SymExpr::constant(U256::from(16))).unwrap();
    state.stack.push(SymExpr::var("base")).unwrap();

    state.exp_word().unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        result.eval_model(&BTreeMap::from([("base".to_string(), U256::from(2))])).unwrap(),
        U256::from(65536)
    );
}

#[test]
fn exp_helper_expands_bounded_symbolic_exponent() {
    let mut state = empty_state();
    let exponent = SymExpr::var("exponent");
    state.constraints.push(SymBoolExpr::cmp(
        SymBoolExprOp::Ule,
        exponent.clone(),
        SymExpr::constant(U256::from(5)),
    ));
    state.stack.push(exponent).unwrap();
    state.stack.push(SymExpr::constant(U256::from(3))).unwrap();

    state.exp_word().unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        result.eval_model(&BTreeMap::from([("exponent".to_string(), U256::from(5))])).unwrap(),
        U256::from(243)
    );
}

#[test]
fn shift_helpers_accept_symbolic_amounts() {
    let mut shl = empty_state();
    shl.stack.push(SymExpr::constant(U256::from(1))).unwrap();
    shl.stack.push(SymExpr::var("shift")).unwrap();
    shl.shift_word(ShiftKind::Shl).unwrap();
    let shifted = shl.stack.pop().unwrap();

    assert_eq!(
        shifted.eval_model(&BTreeMap::from([("shift".to_string(), U256::from(5))])).unwrap(),
        U256::from(32)
    );
    assert_eq!(
        shifted.eval_model(&BTreeMap::from([("shift".to_string(), U256::from(256))])).unwrap(),
        U256::ZERO
    );

    let mut shr = empty_state();
    shr.stack.push(SymExpr::constant(U256::from(1) << 255)).unwrap();
    shr.stack.push(SymExpr::var("shift")).unwrap();
    shr.shift_word(ShiftKind::Shr).unwrap();
    let shifted = shr.stack.pop().unwrap();

    assert_eq!(
        shifted.eval_model(&BTreeMap::from([("shift".to_string(), U256::from(255))])).unwrap(),
        U256::from(1)
    );
    assert_eq!(
        shifted.eval_model(&BTreeMap::from([("shift".to_string(), U256::from(256))])).unwrap(),
        U256::ZERO
    );

    let mut sar = empty_state();
    sar.stack.push(SymExpr::constant(U256::MAX)).unwrap();
    sar.stack.push(SymExpr::var("shift")).unwrap();
    sar.shift_word(ShiftKind::Sar).unwrap();
    let shifted = sar.stack.pop().unwrap();

    assert_eq!(
        shifted.eval_model(&BTreeMap::from([("shift".to_string(), U256::from(300))])).unwrap(),
        U256::MAX
    );
}

#[test]
fn symbolic_division_guards_zero_divisor() {
    let mut state = empty_state();
    state.stack.push(SymExpr::var("den")).unwrap();
    state.stack.push(SymExpr::var("num")).unwrap();

    state.bin_word_div_zero_guard(SymExprOp::UDiv).unwrap();

    assert_eq!(
        state.stack.pop().unwrap(),
        SymExpr::ite(
            SymBoolExpr::eq(SymExpr::var("den"), SymExpr::constant(U256::ZERO)),
            SymExpr::constant(U256::ZERO),
            SymExpr::op(SymExprOp::UDiv, SymExpr::var("num"), SymExpr::var("den")),
        )
    );
}

#[test]
fn symbolic_byte_extracts_with_concrete_index() {
    assert_eq!(
        byte_word(U256::from(0), SymExpr::var("word")),
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(SymExprOp::Shr, SymExpr::var("word"), SymExpr::constant(U256::from(248))),
            SymExpr::constant(U256::from(0xff))
        )
    );
}

#[test]
fn symbolic_byte_extracts_with_symbolic_index() {
    let word = U256::from_be_bytes(core::array::from_fn::<_, 32, _>(|idx| idx as u8));
    let byte = byte_word_dynamic(SymExpr::var("index"), SymExpr::constant(word));

    let in_range = BTreeMap::from([("index".to_string(), U256::from(9))]);
    assert_eq!(byte.eval_model(&in_range).unwrap(), U256::from(9));

    let out_of_range = BTreeMap::from([("index".to_string(), U256::from(32))]);
    assert_eq!(byte.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn symbolic_signextend_accepts_symbolic_index() {
    let value = SymExpr::constant(U256::from(0x80));
    let extended = signextend_word_dynamic(SymExpr::var("index"), value);

    let zero_index = BTreeMap::from([("index".to_string(), U256::ZERO)]);
    assert_eq!(extended.eval_model(&zero_index).unwrap(), U256::MAX - U256::from(0x7f));

    let one_index = BTreeMap::from([("index".to_string(), U256::from(1))]);
    assert_eq!(extended.eval_model(&one_index).unwrap(), U256::from(0x80));
}

#[test]
fn symbolic_byte_preserves_concrete_packed_selector_bytes() {
    let selector = U256::from(0x12345678);
    let packed = SymExpr::op(
        SymExprOp::Or,
        SymExpr::op(
            SymExprOp::Shl,
            SymExpr::constant(selector),
            SymExpr::constant(U256::from(224)),
        ),
        SymExpr::op(SymExprOp::Shr, SymExpr::var("arg"), SymExpr::constant(U256::from(32))),
    );

    assert_eq!(byte_word(U256::from(0), packed.clone()), SymExpr::constant(U256::from(0x12)));
    assert_eq!(byte_word(U256::from(1), packed.clone()), SymExpr::constant(U256::from(0x34)));
    assert_eq!(byte_word(U256::from(2), packed.clone()), SymExpr::constant(U256::from(0x56)));
    assert_eq!(byte_word(U256::from(3), packed), SymExpr::constant(U256::from(0x78)));
}

#[test]
fn word_reassembly_preserves_split_symbolic_word() {
    let original =
        SymExpr::op(SymExprOp::Add, SymExpr::var("value"), SymExpr::constant(U256::from(1)));
    let bytes = original.clone().into_byte_exprs();

    assert_eq!(SymExpr::from_bytes(bytes), original);
}

#[test]
fn symbolic_address_aliases_match_abi_encoded_address_words() {
    let source = SymExpr::var("beneficiary");
    let masked = SymExpr::op(
        SymExprOp::And,
        source.clone(),
        SymExpr::constant((U256::from(1) << 160) - U256::from(1)),
    );
    let mut encoded = vec![SymExpr::zero(); 12];
    encoded.extend((12..32).map(|idx| byte_word(U256::from(idx), masked.clone())));
    let reassembled = SymExpr::from_bytes(encoded);

    assert!(source.symbolic_address_equivalent(&reassembled));
    assert_eq!(source.symbolic_address_key(), reassembled.symbolic_address_key());
}

#[test]
fn selector_shift_simplifies_to_concrete_word() {
    let selector = U256::from(0x12345678);
    let call_word = SymExpr::op(
        SymExprOp::Or,
        SymExpr::op(
            SymExprOp::Shl,
            SymExpr::constant(selector),
            SymExpr::constant(U256::from(224)),
        ),
        SymExpr::op(SymExprOp::Shr, SymExpr::var("arg"), SymExpr::constant(U256::from(32))),
    );
    let selector_expr = SymExpr::op(SymExprOp::Shr, call_word, SymExpr::constant(U256::from(224)));

    assert_eq!(selector_expr.known_word(), Some(selector));
}

#[test]
fn selector_equality_folds_known_word_expressions() {
    let selector = U256::from(0x12345678u32);
    let other = U256::from(0x9a8325a0u32);
    let call_word = SymExpr::op(
        SymExprOp::Or,
        SymExpr::op(
            SymExprOp::Shl,
            SymExpr::constant(selector),
            SymExpr::constant(U256::from(224)),
        ),
        SymExpr::op(SymExprOp::Shr, SymExpr::var("arg"), SymExpr::constant(U256::from(32))),
    );
    let selector_expr = SymExpr::op(SymExprOp::Shr, call_word, SymExpr::constant(U256::from(224)));

    assert_eq!(
        SymBoolExpr::eq(selector_expr.clone(), SymExpr::constant(selector)),
        SymBoolExpr::constant(true)
    );
    assert_eq!(
        SymBoolExpr::eq(selector_expr, SymExpr::constant(other)),
        SymBoolExpr::constant(false)
    );
}

#[test]
fn calldata_selector_load_simplifies_to_concrete_word() {
    let function = Function::parse("check(bytes32)").unwrap();
    let calldata =
        SymbolicCalldata::variants(&function, &SymbolicConfig::default()).unwrap().remove(0);
    let selector = U256::from_be_slice(function.selector().as_slice());
    let loaded = calldata.call_data().load_word(SymExpr::zero()).unwrap();
    let selector_expr = SymExpr::op(SymExprOp::Shr, loaded, SymExpr::constant(U256::from(224)));

    assert_eq!(selector_expr.known_word(), Some(selector));
    assert_eq!(
        SymBoolExpr::eq(selector_expr, SymExpr::constant(selector)),
        SymBoolExpr::constant(true)
    );
}

#[test]
fn artifact_json_fallback_paths_uses_foundry_artifact_basename() {
    assert_eq!(
        artifact_json_fallback_paths("src/01_NomadZeroRoot.sol:NomadLike"),
        vec![std::path::PathBuf::from("out/01_NomadZeroRoot.sol/NomadLike.json")]
    );
    assert_eq!(
        artifact_json_fallback_paths(r"src\01_NomadZeroRoot.sol:NomadLike"),
        vec![std::path::PathBuf::from("out/01_NomadZeroRoot.sol/NomadLike.json")]
    );
    assert_eq!(
        artifact_json_fallback_paths(r"src\01_NomadZeroRoot.sol"),
        vec![std::path::PathBuf::from("out/01_NomadZeroRoot.sol/01_NomadZeroRoot.json")]
    );
}

#[test]
fn dynamic_calldata_encodes_bounded_bytes() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = SymbolicCalldata::variants(&function, &config).unwrap().remove(0);
    let call_data = calldata.call_data();

    assert_eq!(call_data.size_word(), SymExpr::constant(U256::from(100)));
    assert_eq!(call_data.load(4).unwrap(), SymExpr::constant(U256::from(32)));
    assert_eq!(call_data.load(36).unwrap(), SymExpr::constant(U256::from(3)));
    assert_eq!(
        call_data.read_bytes_offset(SymExpr::constant(U256::from(71)), 1).byte(0),
        SymExpr::zero()
    );

    let model = BTreeMap::from([
        ("calldata_0_0".to_string(), U256::from(1)),
        ("calldata_0_1".to_string(), U256::from(2)),
        ("calldata_0_2".to_string(), U256::from(3)),
    ]);
    assert_eq!(calldata.model_to_args(&model).unwrap(), vec![DynSolValue::Bytes(vec![1, 2, 3])]);
}

#[test]
fn calldata_word_loads_preserve_structural_words() {
    let function = Function::parse("check(uint256,bool,address)").unwrap();
    let calldata =
        SymbolicCalldata::variants(&function, &SymbolicConfig::default()).unwrap().remove(0);
    let call_data = calldata.call_data();

    assert_eq!(call_data.load(4).unwrap(), SymExpr::var("calldata_0"));
    assert_eq!(call_data.load(36).unwrap(), SymExpr::var("calldata_1"));
    assert_eq!(call_data.load(68).unwrap(), SymExpr::var("calldata_2"));
}

#[test]
fn calldata_copy_to_memory_preserves_structural_words() {
    let function = Function::parse("check(uint256,uint256)").unwrap();
    let calldata =
        SymbolicCalldata::variants(&function, &SymbolicConfig::default()).unwrap().remove(0);
    let mut memory = SymMemory::default();

    memory
        .copy_calldata_to_offset(
            SymExpr::constant(U256::from(0x80)),
            SymExpr::constant(U256::from(4)),
            64,
            &calldata.call_data(),
        )
        .unwrap();

    assert_eq!(memory.load_word(0x80).unwrap(), SymExpr::var("calldata_0"));
    assert_eq!(memory.load_word(0xa0).unwrap(), SymExpr::var("calldata_1"));
}

#[test]
fn byte_concat_word_load_assembles_word_fragments() {
    let selector = [0xde, 0xad, 0xbe, 0xef];
    let word = SymExpr::var("calldata_0");
    let bytes =
        SymBytes::concat([SymBytes::concrete(selector.to_vec()), word.clone().into_bytes()]);

    assert_eq!(bytes.word_at(4), word);

    let mut word_value = [0u8; 32];
    for (idx, byte) in word_value.iter_mut().enumerate() {
        *byte = idx as u8 + 1;
    }
    let mut expected = [0u8; 32];
    expected[..4].copy_from_slice(&selector);
    expected[4..].copy_from_slice(&word_value[..28]);

    assert_eq!(
        bytes
            .word_at(0)
            .eval_model(&BTreeMap::from([(
                "calldata_0".to_string(),
                U256::from_be_bytes(word_value)
            )]))
            .unwrap(),
        U256::from_be_bytes(expected)
    );
}

#[test]
fn byte_concat_right_aligned_word_assembles_push_fragments() {
    let immediate = SymExpr::var("push_data");
    let bytes = SymBytes::concat([
        SymBytes::concrete(vec![opcode::PUSH32]),
        immediate.clone().into_bytes(),
    ]);

    assert_eq!(bytes.right_aligned_word(1, 32), immediate);
    assert_eq!(
        bytes
            .right_aligned_word(29, 4)
            .eval_model(&BTreeMap::from([("push_data".to_string(), U256::from(0x12345678))]))
            .unwrap(),
        U256::from(0x12345678)
    );
}

#[test]
fn calldata_load_accepts_symbolic_offsets() {
    let calldata = SymCalldata::from_bytes(SymBytes::exprs(
        (0u8..40).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect(),
    ));
    let loaded = calldata.load_word(SymExpr::var("offset")).unwrap();
    let expected = SymExpr::from_bytes((1u8..33).map(|idx| SymExpr::constant(U256::from(idx + 1))));

    assert_eq!(
        loaded.eval_model(&BTreeMap::from([("offset".to_string(), U256::from(1))])).unwrap(),
        expected.eval_model(&BTreeMap::new()).unwrap()
    );
    assert_eq!(
        loaded.eval_model(&BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
        U256::ZERO
    );
}

#[test]
fn calldata_preserves_symbolic_size_for_call_frames() {
    let mut memory = SymMemory::default();
    memory.store_bytes(
        0,
        SymBytes::exprs(vec![
            SymExpr::constant(U256::from(0xaa)),
            SymExpr::constant(U256::from(0xbb)),
            SymExpr::constant(U256::from(0xcc)),
            SymExpr::constant(U256::from(0xdd)),
        ]),
    );
    let size = SymExpr::var("size");
    let bounded_size = BoundedCopySize::Symbolic { size, max_size: 4 };
    let input = bounded_size.read_from_memory(&memory, SymExpr::constant(U256::ZERO));
    let calldata = bounded_size.calldata(input);
    let model = BTreeMap::from([("size".to_string(), U256::from(2))]);

    assert_eq!(calldata.size_word().eval_model(&model).unwrap(), U256::from(2));
    let bytes = calldata.read_bytes_offset(SymExpr::zero(), 4);
    assert_eq!(bytes.byte(0).eval_model(&model).unwrap(), U256::from(0xaa));
    assert_eq!(bytes.byte(1).eval_model(&model).unwrap(), U256::from(0xbb));
    assert_eq!(bytes.byte(2).eval_model(&model).unwrap(), U256::ZERO);
    assert_eq!(bytes.byte(3).eval_model(&model).unwrap(), U256::ZERO);
}

#[test]
fn memory_copies_unaligned_symbolic_calldata_bytes() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = SymbolicCalldata::variants(&function, &config).unwrap().remove(0);
    let mut memory = SymMemory::default();

    memory
        .copy_calldata_to_offset(
            SymExpr::constant(U256::from(1)),
            SymExpr::constant(U256::from(68)),
            3,
            &calldata.call_data(),
        )
        .unwrap();
    let word = memory.load_word(0).unwrap();

    let model = BTreeMap::from([
        ("calldata_0_0".to_string(), U256::from(1)),
        ("calldata_0_1".to_string(), U256::from(2)),
        ("calldata_0_2".to_string(), U256::from(3)),
    ]);
    let mut expected = [0u8; 32];
    expected[1..4].copy_from_slice(&[1, 2, 3]);
    assert_eq!(word.eval_model(&model).unwrap(), U256::from_be_bytes(expected));
}

#[test]
fn memory_copies_symbolic_calldata_offset() {
    let calldata = SymCalldata::from_bytes(SymBytes::exprs(
        (0u8..40).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect(),
    ));
    let mut memory = SymMemory::default();

    memory.copy_calldata_to_offset(SymExpr::zero(), SymExpr::var("offset"), 2, &calldata).unwrap();
    let word = memory.load_word(0).unwrap();

    let mut expected = [0u8; 32];
    expected[..2].copy_from_slice(&[4, 5]);
    assert_eq!(
        word.eval_model(&BTreeMap::from([("offset".to_string(), U256::from(3))])).unwrap(),
        U256::from_be_bytes(expected)
    );
    assert_eq!(
        word.eval_model(&BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
        U256::ZERO
    );
}

#[test]
fn memory_copies_symbolic_calldata_size_with_guarded_tail() {
    let calldata = SymCalldata::from_bytes(SymBytes::exprs(
        (0u8..8).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect(),
    ));
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));

    memory
        .copy_calldata_symbolic_size(
            SymExpr::constant(U256::ZERO),
            SymExpr::constant(U256::ZERO),
            SymExpr::var("size"),
            4,
            &calldata,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_copies_symbolic_bytecode_size_with_guarded_tail() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));
    memory
        .copy_bytes_size_offset(
            SymExpr::zero(),
            SymExpr::var("size"),
            SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_copies_folded_symbolic_size_prefix_only() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));
    memory
        .copy_bytes_size_offset(
            SymExpr::zero(),
            SymExpr::constant(U256::from(2)),
            SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
        )
        .unwrap();

    assert_eq!(
        memory.read_bytes(0, 4).eval_model(&BTreeMap::new()).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
}

#[test]
fn memory_skips_folded_zero_symbolic_size_copy() {
    let mut memory = SymMemory::default();
    memory
        .copy_bytes_size_offset(
            SymExpr::zero(),
            SymExpr::zero(),
            SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
        )
        .unwrap();

    assert_eq!(memory.size_word(), SymExpr::zero());
    assert_eq!(memory.read_bytes(0, 4).eval_model(&BTreeMap::new()).unwrap(), vec![0, 0, 0, 0]);
}

#[test]
fn memory_reads_symbolic_size_with_zero_guarded_tail() {
    let mut memory = SymMemory::default();
    memory.store_bytes(
        32,
        SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
    );

    let bytes =
        memory.read_bytes_symbolic_size(SymExpr::constant(U256::from(32)), SymExpr::var("size"), 4);

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(bytes.eval_model(&size_two).unwrap(), vec![1, 2, 0, 0]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(bytes.eval_model(&size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_reads_folded_symbolic_size_prefix_only() {
    let mut memory = SymMemory::default();
    memory.store_bytes(
        32,
        SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
    );

    let bytes = memory.read_bytes_symbolic_size(
        SymExpr::constant(U256::from(32)),
        SymExpr::constant(U256::from(2)),
        4,
    );

    assert_eq!(bytes.eval_model(&BTreeMap::new()).unwrap(), vec![1, 2, 0, 0]);
}

#[test]
fn memory_copies_symbolic_memory_size_with_guarded_tail() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));
    memory.store_bytes(
        32,
        SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
    );

    memory
        .copy_memory_symbolic_size(
            SymExpr::constant(U256::ZERO),
            SymExpr::constant(U256::from(32)),
            SymExpr::var("size"),
            4,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_copies_symbolic_size_to_symbolic_dest() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));
    memory.store_bytes(
        0x20,
        SymBytes::exprs((0u8..4).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect()),
    );

    memory
        .copy_memory_symbolic_size(
            SymExpr::var("dest"),
            SymExpr::constant(U256::from(0x20)),
            SymExpr::var("size"),
            4,
        )
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("size".to_string(), U256::from(2)),
    ]);
    assert_eq!(memory.read_bytes(0x80, 4).eval_model(&model).unwrap(), vec![1, 2, 0xaa, 0xaa]);
}

#[test]
fn memory_copies_symbolic_returndata_size_with_guarded_tail() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));

    memory
        .copy_return_data_symbolic_size(
            SymExpr::constant(U256::ZERO),
            SymExpr::constant(U256::ZERO),
            SymExpr::var("size"),
            4,
            &return_data,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn returndata_reads_symbolic_offset() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let bytes = return_data.read_bytes_offset(SymExpr::var("offset"), 2);

    let offset_one = BTreeMap::from([("offset".to_string(), U256::from(1))]);
    assert_eq!(bytes.eval_model(&offset_one).unwrap(), vec![2, 3]);

    let offset_four = BTreeMap::from([("offset".to_string(), U256::from(4))]);
    assert_eq!(bytes.eval_model(&offset_four).unwrap(), vec![0, 0]);
}

#[test]
fn memory_return_data_accepts_symbolic_size() {
    let mut memory = SymMemory::default();
    memory.store_bytes(
        0,
        SymBytes::exprs(
            vec![1, 2, 3, 4].into_iter().map(|byte| SymExpr::constant(U256::from(byte))).collect(),
        ),
    );

    let return_data = memory
        .return_data_symbolic_size(SymExpr::constant(U256::ZERO), SymExpr::var("len"), 4)
        .unwrap();

    let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
    assert_eq!(return_data.len_word().eval_model(&len_two).unwrap(), U256::from(2));
    assert_eq!(return_data.byte(0).eval_model(&len_two).unwrap(), U256::from(1));
    assert_eq!(return_data.byte(2).eval_model(&len_two).unwrap(), U256::ZERO);
}

#[test]
fn call_output_preserves_memory_beyond_symbolic_returndata_size() {
    let return_data = SymReturnData::from_bytes_with_len(
        SymBytes::exprs(
            vec![1, 2, 3, 4].into_iter().map(|byte| SymExpr::constant(U256::from(byte))).collect(),
        ),
        SymExpr::var("len"),
    );
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));

    memory
        .copy_call_output_offset(
            SymExpr::constant(U256::ZERO),
            &BoundedCopySize::Concrete(4),
            &return_data,
        )
        .unwrap();

    let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&len_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let len_four = BTreeMap::from([("len".to_string(), U256::from(4))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&len_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn nested_dynamic_calldata_uses_preorder_lengths() {
    let function = Function::parse("check((uint256[],bytes))").unwrap();
    let config = SymbolicConfig { array_lengths: vec![2, 3], ..Default::default() };
    let calldata = SymbolicCalldata::variants(&function, &config).unwrap().remove(0);
    let call_data = calldata.call_data();

    assert_eq!(call_data.load(4).unwrap(), SymExpr::constant(U256::from(32)));
    assert_eq!(call_data.load(36).unwrap(), SymExpr::constant(U256::from(64)));
    assert_eq!(call_data.load(68).unwrap(), SymExpr::constant(U256::from(160)));
    assert_eq!(call_data.load(100).unwrap(), SymExpr::constant(U256::from(2)));
    assert_eq!(call_data.load(196).unwrap(), SymExpr::constant(U256::from(3)));
}

#[test]
fn memory_round_trips_symbolic_words_as_bytes() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word(7, value.clone());

    let model = BTreeMap::from([("word".to_string(), U256::from(0x1234))]);
    assert_eq!(
        memory.load_word(7).unwrap().eval_model(&model).unwrap(),
        value.eval_model(&model).unwrap()
    );
}

#[test]
fn memory_load_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word(7, value);
    let loaded = memory.load_word_offset(SymExpr::var("offset")).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));

    let out_of_range = BTreeMap::from([
        ("offset".to_string(), U256::from(39)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn memory_store_word_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word_offset(SymExpr::var("offset"), value);
    let loaded = memory.load_word(7).unwrap();

    let matching = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&matching).unwrap(), U256::from(0x1234));

    let non_matching = BTreeMap::from([
        ("offset".to_string(), U256::from(100)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&non_matching).unwrap(), U256::ZERO);
}

#[test]
fn memory_size_tracks_concrete_and_symbolic_extents() {
    let mut memory = SymMemory::default();

    memory.store_word(7, SymExpr::constant(U256::from(0x55)));
    assert_eq!(memory.size_word(), SymExpr::constant(U256::from(64)));

    memory.store_word_offset(SymExpr::var("offset"), SymExpr::var("word"));
    let size = memory.size_word();

    let below_concrete = BTreeMap::from([
        ("offset".to_string(), U256::from(9)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(size.eval_model(&below_concrete).unwrap(), U256::from(64));

    let above_concrete = BTreeMap::from([
        ("offset".to_string(), U256::from(70)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(size.eval_model(&above_concrete).unwrap(), U256::from(128));
}

#[test]
fn memory_concrete_write_overrides_older_symbolic_write() {
    let mut memory = SymMemory::default();

    memory.store_word_offset(SymExpr::var("offset"), SymExpr::var("word"));
    memory.store_word(7, SymExpr::constant(U256::from(0x55)));
    let loaded = memory.load_word(7).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x55));
}

#[test]
fn memory_dynamic_read_respects_concrete_overwrite_epoch() {
    let mut memory = SymMemory::default();

    memory.store_byte_offset(SymExpr::var("write_offset"), SymExpr::var("byte"));
    memory.store_byte(5, SymExpr::constant(U256::from(0x55)));
    let loaded = memory.byte_dynamic_with_delta(&SymExpr::var("read_offset"), 0);

    let model = BTreeMap::from([
        ("write_offset".to_string(), U256::from(5)),
        ("read_offset".to_string(), U256::from(5)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x55));
}

#[test]
fn memory_store_byte_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();

    memory.store_byte_offset(SymExpr::var("offset"), SymExpr::var("byte"));
    let loaded = memory.byte(0x80);

    let matching = BTreeMap::from([
        ("offset".to_string(), U256::from(0x80)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(loaded.eval_model(&matching).unwrap(), U256::from(0xab));

    let non_matching = BTreeMap::from([
        ("offset".to_string(), U256::from(0x81)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(loaded.eval_model(&non_matching).unwrap(), U256::ZERO);
}

#[test]
fn memory_read_bytes_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word(7, value);
    let loaded =
        SymExpr::from_bytes(memory.read_bytes_offset(SymExpr::var("offset"), 32).materialize());

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));

    let out_of_range = BTreeMap::from([
        ("offset".to_string(), U256::from(39)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn memory_return_data_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word(7, value);
    let return_data = memory.return_data(SymExpr::var("offset"), 32).unwrap();
    let loaded = return_data.load_word(0).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_copy_accepts_symbolic_source_offsets() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word(7, value);
    memory
        .copy_memory_to_offset(SymExpr::constant(U256::from(64)), SymExpr::var("src"), 32)
        .unwrap();
    let loaded = memory.load_word(64).unwrap();

    let model = BTreeMap::from([
        ("src".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_copy_accepts_symbolic_destination_offsets() {
    let mut memory = SymMemory::default();
    let value = SymExpr::var("word");

    memory.store_word(7, value);
    memory
        .copy_memory_to_offset(SymExpr::var("dest"), SymExpr::constant(U256::from(7)), 32)
        .unwrap();
    let loaded = memory.load_word(64).unwrap();

    let matching = BTreeMap::from([
        ("dest".to_string(), U256::from(64)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&matching).unwrap(), U256::from(0x1234));

    let non_matching = BTreeMap::from([
        ("dest".to_string(), U256::from(96)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&non_matching).unwrap(), U256::ZERO);
}

#[test]
fn memory_call_output_accepts_symbolic_destination_offsets() {
    let mut memory = SymMemory::default();
    let return_data = SymReturnData::from_bytes(SymExpr::var("word").into_bytes());

    memory
        .copy_call_output_offset(SymExpr::var("dest"), &BoundedCopySize::Concrete(32), &return_data)
        .unwrap();
    let loaded = memory.load_word(64).unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(64)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_call_output_accepts_symbolic_size_with_guarded_tail() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    memory.store_bytes(0, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));

    memory
        .copy_call_output_offset(
            SymExpr::constant(U256::ZERO),
            &BoundedCopySize::Symbolic { size: SymExpr::var("size"), max_size: 4 },
            &return_data,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(memory.read_bytes(0, 4).eval_model(&size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_call_output_accepts_symbolic_destination_and_size() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));

    memory
        .copy_call_output_offset(
            SymExpr::var("dest"),
            &BoundedCopySize::Symbolic { size: SymExpr::var("size"), max_size: 4 },
            &return_data,
        )
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("size".to_string(), U256::from(2)),
    ]);
    assert_eq!(memory.read_bytes(0x80, 4).eval_model(&model).unwrap(), vec![1, 2, 0xaa, 0xaa]);
}

#[test]
fn memory_call_output_accepts_symbolic_destination_and_return_len() {
    let return_data = SymReturnData::from_bytes_with_len(
        SymBytes::exprs(
            vec![1, 2, 3, 4].into_iter().map(|byte| SymExpr::constant(U256::from(byte))).collect(),
        ),
        SymExpr::var("len"),
    );
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, SymBytes::exprs(vec![SymExpr::constant(U256::from(0xaa)); 4]));

    memory
        .copy_call_output_offset(SymExpr::var("dest"), &BoundedCopySize::Concrete(4), &return_data)
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("len".to_string(), U256::from(2)),
    ]);
    assert_eq!(memory.read_bytes(0x80, 4).eval_model(&model).unwrap(), vec![1, 2, 0xaa, 0xaa]);
}

#[test]
fn create_address_helpers_match_alloy_primitives() {
    let creator = Address::from([0x11; 20]);
    let initcode = vec![opcode::PUSH1, 0x00, opcode::PUSH1, 0x00, opcode::RETURN];
    let salt = U256::from(7);

    assert_ne!(creator.create(3), creator.create(4));
    assert_ne!(
        creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode),
        creator.create2_from_code((salt + U256::from(1)).to_be_bytes::<32>(), &initcode)
    );
}

#[test]
fn compute_create2_cheatcode_helper_matches_create2_terms() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymExpr::var("salt");
    let initcode = vec![opcode::STOP];
    let initcode_hash = SymExpr::constant(U256::from_be_bytes(keccak256(&initcode).0));

    let cheatcode_word = compute_create2_address_word(
        &mut state,
        SymExpr::constant(address_word(creator)),
        salt.clone(),
        initcode_hash,
    )
    .unwrap();
    let opcode_word =
        create2_address_word(&mut state, creator, salt, &SymCode::concrete(initcode)).unwrap().0;

    assert_eq!(cheatcode_word, opcode_word);
}

#[test]
fn compute_create2_cheatcode_helper_accepts_symbolic_init_code_hash() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymExpr::var("salt");
    let initcode_hash = SymExpr::var("initcode_hash");

    let first = compute_create2_address_word(
        &mut state,
        SymExpr::constant(address_word(creator)),
        salt.clone(),
        initcode_hash.clone(),
    )
    .unwrap();
    let second = compute_create2_address_word(
        &mut state,
        SymExpr::constant(address_word(creator)),
        salt,
        initcode_hash,
    )
    .unwrap();

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create2_address_")));
}

#[test]
fn compute_create_cheatcode_helper_accepts_symbolic_nonce() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let nonce = SymExpr::var("nonce");

    let first = compute_create_address_word(
        &mut state,
        SymExpr::constant(address_word(creator)),
        nonce.clone(),
    )
    .unwrap();
    let second =
        compute_create_address_word(&mut state, SymExpr::constant(address_word(creator)), nonce)
            .unwrap();

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create_address_")));
}

#[test]
fn compute_create_cheatcode_helper_accepts_symbolic_deployer() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let deployer = SymExpr::var("deployer");
    let nonce = SymExpr::var("nonce");

    let first = compute_create_address_word(&mut state, deployer.clone(), nonce.clone())
        .expect("symbolic deployer is supported");
    let second = compute_create_address_word(&mut state, deployer, nonce)
        .expect("symbolic deployer is supported");

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create_address_")));
}

#[test]
fn compute_create2_cheatcode_helper_accepts_symbolic_deployer() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let deployer = SymExpr::var("deployer");
    let salt = SymExpr::var("salt");
    let initcode_hash = SymExpr::var("initcode_hash");

    let first = compute_create2_address_word(
        &mut state,
        deployer.clone(),
        salt.clone(),
        initcode_hash.clone(),
    )
    .expect("symbolic deployer is supported");
    let second = compute_create2_address_word(&mut state, deployer, salt, initcode_hash)
        .expect("symbolic deployer is supported");

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create2_address_")));
}

#[test]
fn recorded_logs_return_data_matches_abi_encoding() {
    let emitter = Address::from([0x33; 20]);
    let topic = B256::from([0x11; 32]);
    let log = SymbolicLog::new(
        vec![SymExpr::constant(U256::from_be_bytes(topic.0))],
        SymExpr::constant(U256::from(2)),
        SymBytes::exprs(vec![
            SymExpr::constant(U256::from(0x22)),
            SymExpr::constant(U256::from(0x33)),
        ]),
        emitter,
    );

    let encoded =
        recorded_logs_return_data(vec![log]).read_concrete("recorded log return data").unwrap();
    let expected = DynSolValue::Array(vec![DynSolValue::Tuple(vec![
        DynSolValue::Array(vec![DynSolValue::FixedBytes(topic, 32)]),
        DynSolValue::Bytes(vec![0x22, 0x33]),
        DynSolValue::Address(emitter),
    ])])
    .abi_encode();

    assert_eq!(encoded, expected);
}

#[test]
fn recorded_logs_json_return_data_accepts_symbolic_topics_and_data() {
    let emitter = Address::from([0x33; 20]);
    let log = SymbolicLog::new(
        vec![SymExpr::var("topic")],
        SymExpr::constant(U256::from(2)),
        SymBytes::exprs(vec![SymExpr::constant(U256::from(0x12)), SymExpr::var("byte")]),
        emitter,
    );

    let return_data = recorded_logs_json_return_data(vec![log]).unwrap();
    let encoded =
        SymBytes::exprs((0..return_data.len()).map(|idx| return_data.byte(idx)).collect())
            .eval_model(&BTreeMap::from([
                ("topic".to_string(), U256::from(0xabcd)),
                ("byte".to_string(), U256::from(0xef)),
            ]))
            .unwrap();
    let decoded = DynSolType::String.abi_decode(&encoded).unwrap();
    let DynSolValue::String(json) = decoded else { panic!("expected string return") };

    assert!(json.contains("\"topics\":[\"0x"));
    assert!(json.contains("abcd"));
    assert!(json.contains("\"data\":\"0x12ef\""));
    assert!(json.contains(&format!("\"emitter\":\"{emitter}\"")));
}

#[test]
fn abi_bytes_encoding_accepts_symbolic_length() {
    let bytes = SymBytes::exprs(vec![
        SymExpr::constant(U256::from(0x22)),
        SymExpr::constant(U256::from(0x33)),
        SymExpr::constant(U256::from(0x44)),
    ]);
    let encoded = encode_packed_bytes_with_len(SymExpr::var("len"), &bytes);

    assert_eq!(
        encoded
            .word_at(0)
            .eval_model(&BTreeMap::from([("len".to_string(), U256::from(2))]))
            .unwrap(),
        U256::from(2)
    );
}

#[test]
fn symbolic_world_resolves_symbolic_create2_address_aliases() {
    let mut world = SymbolicWorld::default();
    let word = SymExpr::var("create2_address");
    let address = world.symbolic_address_slot(word.clone());
    let masked = SymExpr::op(
        SymExprOp::And,
        word.clone(),
        SymExpr::constant((U256::from(1) << 160) - U256::from(1)),
    );

    assert_eq!(world.resolve_address(&word), Some(address));
    assert_eq!(world.resolve_address(&masked), Some(address));
    assert_eq!(world.symbolic_address_slot(word), address);
    assert_ne!(address, Address::ZERO);
}

#[test]
fn symbolic_create2_accepts_symbolic_salt() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymExpr::var("salt");
    let initcode = SymCode::concrete(vec![opcode::STOP]);

    let (word, address) = create2_address_word(&mut state, creator, salt, &initcode).unwrap();

    assert!(word.as_const().is_none());
    assert_eq!(state.world.resolve_address(&word), Some(address));
    assert_eq!(state.constraints.len(), 1);
    assert_ne!(address, Address::ZERO);
}

#[test]
fn symbolic_create2_initcode_identity_uses_byte_semantics() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymExpr::var("salt");
    let init_word = SymExpr::var("init_word");
    let structural = SymCode::from_bytes(init_word.clone().into_bytes());
    let materialized = SymCode::from_byte_exprs(init_word.into_byte_exprs());

    let (structural_word, structural_address) =
        create2_address_word(&mut state, creator, salt.clone(), &structural).unwrap();
    let (materialized_word, materialized_address) =
        create2_address_word(&mut state, creator, salt, &materialized).unwrap();

    assert_eq!(structural_word, materialized_word);
    assert_eq!(structural_address, materialized_address);
}

#[test]
fn symbolic_return_data_can_be_installed_as_runtime_code() {
    let data = SymReturnData::from_bytes(SymBytes::exprs(vec![SymExpr::var("runtime_byte")]));

    let code = data.to_code().unwrap();

    assert_eq!(code.read_bytes(0, 1).materialize(), vec![SymExpr::var("runtime_byte")]);
}

#[test]
fn symbolic_runtime_size_is_not_installed_as_concrete_code() {
    let data = SymReturnData::from_bytes_with_len(
        SymBytes::exprs(vec![SymExpr::constant(U256::from(opcode::STOP))]),
        SymExpr::var("runtime_len"),
    );

    assert!(matches!(
        data.to_code(),
        Err(SymbolicError::Unsupported("CREATE with symbolic runtime size not modeled"))
    ));
}

#[test]
fn symbolic_world_tracks_created_code_and_nonce_overlay() {
    let created = Address::from([0x22; 20]);
    let mut world = SymbolicWorld::default();

    world.install_code(created, SymCode::concrete(vec![opcode::STOP]));
    world.set_nonce(created, 1);

    assert_eq!(world.cached_code(created), Some(&SymCode::concrete(vec![opcode::STOP])));
    assert_eq!(world.cached_nonce(created), Some(1));
}

#[test]
fn symbolic_codecopy_preserves_symbolic_constructor_bytes() {
    let mut memory = SymMemory::default();
    let initcode = SymCode::from_bytes(SymBytes::exprs(vec![
        SymExpr::constant(U256::from(opcode::STOP)),
        SymExpr::var("constructor_arg_byte"),
    ]));

    memory.store_bytes(0, initcode.read_bytes(0, 2));

    assert_eq!(memory.byte(0), SymExpr::constant(U256::from(opcode::STOP)));
    assert_eq!(memory.byte(1), SymExpr::var("constructor_arg_byte"));
}

#[test]
fn symbolic_codecopy_accepts_symbolic_offsets() {
    let code = SymCode::from_bytes(SymBytes::exprs(
        (0u8..40).map(|idx| SymExpr::constant(U256::from(idx + 1))).collect(),
    ));
    let mut memory = SymMemory::default();

    memory.store_bytes(0, code.read_bytes_offset(SymExpr::var("offset"), 2));
    let word = memory.load_word(0).unwrap();

    let mut expected = [0u8; 32];
    expected[..2].copy_from_slice(&[4, 5]);
    assert_eq!(
        word.eval_model(&BTreeMap::from([("offset".to_string(), U256::from(3))])).unwrap(),
        U256::from_be_bytes(expected)
    );
    assert_eq!(
        word.eval_model(&BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
        U256::ZERO
    );
}

#[test]
fn symbolic_initcode_accepts_symbolic_memory_offsets() {
    let mut memory = SymMemory::default();

    memory.store_bytes(
        7,
        SymBytes::exprs(vec![SymExpr::constant(U256::from(opcode::STOP)), SymExpr::var("arg")]),
    );
    let initcode = SymCode::from_memory_offset(&memory, SymExpr::var("offset"), 2);
    let word = SymExpr::from_bytes(
        initcode
            .read_bytes(0, 2)
            .materialize()
            .into_iter()
            .chain(std::iter::repeat_with(SymExpr::zero).take(30)),
    );

    let mut expected = [0u8; 32];
    expected[0] = opcode::STOP;
    expected[1] = 0x2a;
    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("arg".to_string(), U256::from(0x2a)),
    ]);
    assert_eq!(word.eval_model(&model).unwrap(), U256::from_be_bytes(expected));
}

#[test]
fn path_state_extracts_constrained_symbolic_usize() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let offset = SymExpr::var("offset");

    state.constraints.push(SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(7))));

    assert_eq!(state.constrained_usize(&offset), Some(7));
}

#[test]
fn path_state_extracts_symbolic_usize_upper_bound() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let size = SymExpr::var("size");

    state.constraints.push(SymBoolExpr::cmp(
        SymBoolExprOp::Ult,
        size.clone(),
        SymExpr::constant(U256::from(5)),
    ));

    assert_eq!(state.upper_bound_usize(&size), Some(4));
}

#[test]
fn path_state_extracts_constrained_symbolic_usize_from_encoded_bool_word() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let offset = SymExpr::var("offset");
    let offset_expr = offset.clone();
    let mask = SymExpr::constant(U256::from(0xffff));

    state.constraints.push(SymBoolExpr::eq(
        SymExpr::op(SymExprOp::And, offset_expr.clone(), mask.clone()),
        offset_expr.clone(),
    ));
    let condition = SymBoolExpr::eq(
        SymExpr::constant(U256::from(0x80)),
        SymExpr::op(SymExprOp::And, mask, offset_expr),
    );
    let bool_byte = SymExpr::op(
        SymExprOp::And,
        SymExpr::op(
            SymExprOp::Shr,
            SymExpr::ite(
                condition,
                SymExpr::constant(U256::from(1)),
                SymExpr::constant(U256::ZERO),
            ),
            SymExpr::constant(U256::ZERO),
        ),
        SymExpr::constant(U256::from(0xff)),
    );
    state.constraints.push(
        SymBoolExpr::eq(
            SymExpr::op(SymExprOp::Or, SymExpr::constant(U256::ZERO), bool_byte),
            SymExpr::constant(U256::ZERO),
        )
        .not(),
    );

    assert_eq!(state.constrained_usize(&offset), Some(0x80));
}

#[test]
fn path_state_evaluates_compound_constrained_symbolic_word() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let value = SymExpr::var("value");
    state.constraints.push(SymBoolExpr::eq(value.clone(), SymExpr::constant(U256::from(0xbeef))));

    let encoded_word = SymExpr::op(
        SymExprOp::Or,
        SymExpr::constant(U256::ZERO),
        SymExpr::op(SymExprOp::And, value, SymExpr::constant(U256::from(u64::MAX))),
    );

    assert_eq!(state.constrained_word(&encoded_word), Some(U256::from(0xbeef)));
}

#[test]
fn symbolic_push_data_reconstructs_symbolic_word() {
    let code = SymCode::from_bytes(SymBytes::exprs(vec![
        SymExpr::constant(U256::from(opcode::PUSH2)),
        SymExpr::var("immutable_hi"),
        SymExpr::var("immutable_lo"),
    ]));

    let word = code.push_data_word(1, 2);

    assert!(word.as_const().is_none());
}

#[test]
fn symbolic_push32_preserves_structural_word() {
    let immediate = SymExpr::var("immutable");
    let code = SymCode::from_bytes(SymBytes::concat([
        SymBytes::concrete(vec![opcode::PUSH32]),
        immediate.clone().into_bytes(),
    ]));

    assert_eq!(code.push_data_word(1, 32), immediate);
}

#[test]
fn abi_bytes_return_encodes_symbolic_bytes() {
    let ret = abi_bytes_return(vec![
        SymExpr::var("calldata_byte_0"),
        SymExpr::constant(U256::from(0x42)),
    ]);

    assert_eq!(
        SymExpr::from_bytes((0..32).map(|idx| ret.byte(idx))),
        SymExpr::constant(U256::from(32))
    );
    assert_eq!(
        SymExpr::from_bytes((32..64).map(|idx| ret.byte(idx))),
        SymExpr::constant(U256::from(2))
    );
    assert_eq!(ret.byte(64), SymExpr::var("calldata_byte_0"));
    assert_eq!(ret.byte(65), SymExpr::constant(U256::from(0x42)));
}

#[test]
fn abi_bytes_return_can_encode_symbolic_length() {
    let ret = abi_bytes_return_with_len(
        SymExpr::var("len"),
        vec![SymExpr::var("byte_0"), SymExpr::var("byte_1")],
    );

    assert_eq!(
        SymExpr::from_bytes((0..32).map(|idx| ret.byte(idx))),
        SymExpr::constant(U256::from(32))
    );
    assert_eq!(SymExpr::from_bytes((32..64).map(|idx| ret.byte(idx))), SymExpr::var("len"));
    assert_eq!(ret.byte(64), SymExpr::var("byte_0"));
    assert_eq!(ret.byte(65), SymExpr::var("byte_1"));
}

#[test]
fn symbolic_keccak_is_deterministic_for_same_symbolic_bytes() {
    let bytes = SymExpr::var("slot_key").into_byte_exprs();

    let first = keccak_word(bytes.clone());
    let second = keccak_word(bytes);

    assert_eq!(first, second);
    assert!(first.is_keccak());
}

#[test]
fn symbolic_keccak_tracks_symbolic_length() {
    let bytes = vec![SymExpr::var("byte_0"), SymExpr::var("byte_1"), SymExpr::zero()];
    let len = SymExpr::var("len");

    let word = keccak_word_with_len(bytes, len);

    let (hash_len, hash_bytes_len) =
        word.keccak_len_and_byte_count().expect("expected symbolic keccak term");
    assert_eq!(hash_len, &SymExpr::var("len"));
    assert_eq!(hash_bytes_len, 3);
}

#[test]
fn sym_expr_eval_computes_symbolic_keccak_from_model() {
    let owner = Address::from([0x11; 20]);
    let slot = U256::from(1);
    let mut bytes = SymExpr::var("owner").into_byte_exprs();
    bytes.extend(SymExpr::constant(slot).into_byte_exprs());
    let word = keccak_word(bytes);

    let mut input = Vec::new();
    input.extend(address_word(owner).to_be_bytes::<32>());
    input.extend(slot.to_be_bytes::<32>());
    let expected = U256::from_be_bytes(keccak256(input).0);
    let model = BTreeMap::from([("owner".to_string(), address_word(owner))]);

    assert_eq!(word.eval_model(&model).unwrap(), expected);
}

#[test]
fn symbolic_hash_precompiles_are_deterministic_for_same_symbolic_input() {
    let input = vec![SymExpr::var("input_0"), SymExpr::var("input_1")];

    let input_len = SymExpr::constant(U256::from(input.len()));
    let sha = execute_symbolic_precompile(
        precompile_address(2),
        SymBytes::exprs(input.clone()),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let sha_again = execute_symbolic_precompile(
        precompile_address(2),
        SymBytes::exprs(input.clone()),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let sha_word = SymExpr::from_bytes((0..32).map(|idx| sha.byte(idx)));
    let sha_again_word = SymExpr::from_bytes((0..32).map(|idx| sha_again.byte(idx)));

    assert_eq!(sha.len(), 32);
    assert_eq!(sha_word, sha_again_word);
    assert_eq!(sha_word.hash_algorithm(), Some("sha256"));

    let ecrecover = execute_symbolic_precompile(
        precompile_address(1),
        SymBytes::exprs(input.clone()),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let ecrecover_again = execute_symbolic_precompile(
        precompile_address(1),
        SymBytes::exprs(input.clone()),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(ecrecover.len(), 32);
    for idx in 0..12 {
        assert_eq!(ecrecover.byte(idx), SymExpr::zero());
    }
    for idx in 0..32 {
        assert_eq!(ecrecover.byte(idx), ecrecover_again.byte(idx));
    }

    let ripemd = execute_symbolic_precompile(
        precompile_address(3),
        SymBytes::exprs(input.clone()),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let ripemd_again = execute_symbolic_precompile(
        precompile_address(3),
        SymBytes::exprs(input),
        input_len,
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(ripemd.len(), 32);
    for idx in 0..12 {
        assert_eq!(ripemd.byte(idx), SymExpr::zero());
    }
    for idx in 0..32 {
        assert_eq!(ripemd.byte(idx), ripemd_again.byte(idx));
    }
}

#[test]
fn identity_precompile_preserves_symbolic_input_len() {
    let input = vec![
        SymExpr::constant(U256::from(1)),
        SymExpr::constant(U256::from(2)),
        SymExpr::constant(U256::from(3)),
        SymExpr::constant(U256::from(4)),
    ];
    let input_len = SymExpr::var("size");
    let return_data = execute_symbolic_precompile(
        precompile_address(4),
        SymBytes::exprs(input),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(return_data.len(), 4);
    assert_eq!(return_data.len_word(), input_len);
    assert_eq!(return_data.byte(0), SymExpr::constant(U256::from(1)));
    assert_eq!(return_data.byte(3), SymExpr::constant(U256::from(4)));
}

#[test]
fn advanced_precompiles_accept_symbolic_payloads() {
    let mut modexp_input = vec![SymExpr::zero(); 99];
    modexp_input[31] = SymExpr::constant(U256::from(1));
    modexp_input[63] = SymExpr::constant(U256::from(1));
    modexp_input[95] = SymExpr::constant(U256::from(1));
    modexp_input[96] = SymExpr::var("base");
    modexp_input[97] = SymExpr::constant(U256::from(5));
    modexp_input[98] = SymExpr::constant(U256::from(13));

    let modexp = execute_symbolic_precompile(
        precompile_address(5),
        SymBytes::exprs(modexp_input.clone()),
        SymExpr::constant(U256::from(modexp_input.len())),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let modexp_again = execute_symbolic_precompile(
        precompile_address(5),
        SymBytes::exprs(modexp_input.clone()),
        SymExpr::constant(U256::from(modexp_input.len())),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(modexp.len(), 1);
    assert_eq!(modexp.byte(0), modexp_again.byte(0));

    let mut blake_input = vec![SymExpr::var("blake_input"); 213];
    blake_input[212] = SymExpr::zero();
    let blake = execute_symbolic_precompile(
        precompile_address(9),
        SymBytes::exprs(blake_input),
        SymExpr::constant(U256::from(213)),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(blake.len(), 64);
}

#[test]
fn validity_sensitive_symbolic_precompiles_report_incomplete() {
    let bn_input = vec![SymExpr::var("point"); 128];
    let err = execute_symbolic_precompile(
        precompile_address(6),
        SymBytes::exprs(bn_input),
        SymExpr::constant(U256::from(128)),
        SpecId::CANCUN,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SymbolicError::Unsupported("symbolic bn254 precompile validity not modeled")
    ));

    let blake_input = vec![SymExpr::var("blake_input"); 213];
    let err = execute_symbolic_precompile(
        precompile_address(9),
        SymBytes::exprs(blake_input),
        SymExpr::constant(U256::from(213)),
        SpecId::CANCUN,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SymbolicError::Unsupported("symbolic blake2f precompile final flag not modeled")
    ));
}

#[test]
fn symbolic_storage_read_after_write_accepts_symbolic_keys() {
    let address = Address::from([0x11; 20]);
    let key = SymExpr::var("slot");
    let value = SymExpr::var("value");
    let writes = vec![StorageWrite::new(address, key.clone(), value.clone())];

    assert_eq!(StorageWrite::select_from(&writes, address, key, SymExpr::zero()), value);
}

#[test]
fn symbolic_storage_uses_conditional_value_for_maybe_equal_key() {
    let address = Address::from([0x11; 20]);
    let write_key = SymExpr::var("write_slot");
    let read_key = SymExpr::var("read_slot");
    let value = SymExpr::var("value");
    let writes = vec![StorageWrite::new(address, write_key.clone(), value.clone())];

    assert_eq!(
        StorageWrite::select_from(&writes, address, read_key.clone(), SymExpr::zero()),
        SymExpr::ite(SymBoolExpr::eq(read_key, write_key), value, SymExpr::constant(U256::ZERO),)
    );
}

#[test]
fn symbolic_storage_key_equality_decomposes_keccak_offsets() {
    let owner = SymExpr::var("owner");
    let base = keccak_word(owner.into_byte_exprs());
    let left = add_words(base.clone(), SymExpr::var("left_index"));
    let right = add_words(base, SymExpr::var("right_index"));

    assert_eq!(
        left.storage_key_eq(&right),
        SymBoolExpr::eq(SymExpr::var("left_index"), SymExpr::var("right_index"))
    );
}

#[test]
fn symbolic_storage_key_equality_expands_distinct_keccak_bases() {
    let left_base = keccak_word(vec![SymExpr::var("left_owner")]);
    let right_base = keccak_word(vec![SymExpr::var("right_owner")]);
    let index = SymExpr::var("index");

    let left = add_words(left_base, index.clone());
    let right = add_words(right_base, index);
    let condition = left.storage_key_eq(&right);

    assert_eq!(condition, SymBoolExpr::eq(SymExpr::var("left_owner"), SymExpr::var("right_owner")));
}

#[test]
fn symbolic_storage_key_equality_rejects_concrete_plain_slot_alias() {
    let owner = SymExpr::var("owner");
    let layout_key = add_words(keccak_word(owner.into_byte_exprs()), SymExpr::constant(U256::ZERO));

    assert_eq!(
        layout_key.storage_key_eq(&SymExpr::constant(U256::ZERO)),
        SymBoolExpr::constant(false)
    );
}

#[test]
fn nested_mapping_key_does_not_alias_plain_mapping_key_under_model() {
    let owner = SymExpr::var("owner");
    let spender = SymExpr::var("spender");
    let recipient = SymExpr::var("recipient");

    let mut balance_key_bytes = recipient.into_byte_exprs();
    balance_key_bytes.extend(SymExpr::constant(U256::ZERO).into_byte_exprs());
    let balance_key = keccak_word(balance_key_bytes);

    let mut inner_key_bytes = owner.into_byte_exprs();
    inner_key_bytes.extend(SymExpr::constant(U256::from(1)).into_byte_exprs());
    let inner_key = keccak_word(inner_key_bytes);

    let mut allowance_key_bytes = spender.into_byte_exprs();
    allowance_key_bytes.extend(inner_key.into_byte_exprs());
    let allowance_key = keccak_word(allowance_key_bytes);

    let same_address = precompile_address(0x60);
    let model = BTreeMap::from([
        ("owner".to_string(), address_word(Address::from([0x11; 20]))),
        ("spender".to_string(), address_word(same_address)),
        ("recipient".to_string(), address_word(same_address)),
    ]);
    let condition = balance_key.storage_key_eq(&allowance_key);

    assert_eq!(condition, SymBoolExpr::constant(false));
    assert!(!condition.eval_model(&model).unwrap());
}

#[test]
fn symbolic_world_snapshot_restores_overlay_state() {
    let address = Address::from([0x11; 20]);
    let mut world = SymbolicWorld::default();
    world.sstore(address, SymExpr::constant(U256::from(1)), SymExpr::constant(U256::from(2)));

    let snapshot = world.snapshot_state();
    world.sstore(address, SymExpr::constant(U256::from(1)), SymExpr::constant(U256::from(3)));

    assert!(world.restore_snapshot(snapshot));
    assert_eq!(world.storage_len(), 1);
    assert_eq!(world.storage_value(0), Some(&SymExpr::constant(U256::from(2))));
}

#[test]
fn symbolic_world_tracks_current_transaction_created_accounts() {
    let first = Address::from([0x11; 20]);
    let second = Address::from([0x22; 20]);
    let mut world = SymbolicWorld::default();

    world.mark_current_transaction_created(first);
    let snapshot = world.snapshot_state();
    world.mark_current_transaction_created(second);

    assert!(world.was_created_in_current_transaction(first));
    assert!(world.was_created_in_current_transaction(second));

    assert!(world.restore_snapshot(snapshot));
    assert!(world.was_created_in_current_transaction(first));
    assert!(!world.was_created_in_current_transaction(second));

    world.clear_transaction_scoped_state();
    assert!(!world.was_created_in_current_transaction(first));
}

#[test]
fn extra_dynamic_lengths_are_rejected() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![1, 2], ..Default::default() };

    let err = SymbolicCalldata::variants(&function, &config).unwrap_err();

    assert!(err.to_string().contains("symbolic.array_lengths has 2 entries"));
}

#[test]
fn positional_dynamic_lengths_allow_shorter_expanded_variants() {
    let function = Function::parse("check(bytes[])").unwrap();
    let config = SymbolicConfig {
        default_array_lengths: vec![1, 2],
        array_lengths: vec![4, 4],
        ..Default::default()
    };

    let variants = SymbolicCalldata::variants(&function, &config).unwrap();

    assert_eq!(variants.len(), 2);
    let element_counts = variants
        .iter()
        .map(|calldata| {
            calldata
                .call_data()
                .load(36)
                .unwrap()
                .as_const()
                .and_then(|value| usize::try_from(value).ok())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(element_counts, vec![1, 2]);
}

#[test]
fn extra_dynamic_lengths_are_rejected_after_expansion() {
    let function = Function::parse("check(bytes[])").unwrap();
    let config = SymbolicConfig {
        default_array_lengths: vec![1, 2],
        array_lengths: vec![4, 4, 4],
        ..Default::default()
    };

    let err = SymbolicCalldata::variants(&function, &config).unwrap_err();

    assert!(err.to_string().contains("ABI used at most 2 positional dynamic leaves"));
}

#[test]
fn calldata_variant_expansion_respects_path_width() {
    let function = Function::parse("check(bytes[])").unwrap();
    let config = SymbolicConfig {
        width: Some(2),
        default_array_lengths: vec![1, 2],
        default_bytes_lengths: vec![1, 2],
        ..Default::default()
    };

    let err = SymbolicCalldata::variants(&function, &config).unwrap_err();

    assert!(matches!(err, SymbolicError::CalldataVariantLimit(2)));
}

#[test]
fn symbolic_signextend_uses_sign_bit_ite() {
    assert_eq!(
        signextend_word(U256::ZERO, SymExpr::var("word")),
        SymExpr::ite(
            SymBoolExpr::eq(
                SymExpr::op(
                    SymExprOp::And,
                    SymExpr::var("word"),
                    SymExpr::constant(U256::from(0x80))
                ),
                SymExpr::constant(U256::ZERO),
            ),
            SymExpr::op(SymExprOp::And, SymExpr::var("word"), SymExpr::constant(U256::from(0x7f))),
            SymExpr::op(SymExprOp::Or, SymExpr::var("word"), SymExpr::constant(!U256::from(0x7f))),
        )
    );
}

#[test]
fn parse_smt_hex_model_values() {
    let output = "\
sat
(
  (define-fun calldata_0 () (_ BitVec 256)
#x000000000000000000000000000000000000000000000000000000000000002a)
)
";

    let model = parse_model(output).unwrap();

    assert_eq!(model.get(&Symbol::intern("calldata_0")), Some(&U256::from(42)));
}

#[test]
fn parse_smt_binary_model_values() {
    let output = "\
sat
(
  (define-fun calldata_0 () (_ BitVec 8)
    #b00101010)
)
";

    let model = parse_model(output).unwrap();

    assert_eq!(model.get(&Symbol::intern("calldata_0")), Some(&U256::from(42)));
}

#[test]
fn parse_model_rejects_oversized_bitvector_literals() {
    let output =
        format!("sat\n((define-fun calldata_0 () (_ BitVec 257) #b{}))\n", "1".repeat(257));

    let err = parse_model(&output).unwrap_err();

    assert!(err.to_string().contains("exceeds 256 bits"));
}

#[test]
fn validate_solver_model_output_rejects_unsatisfied_models() {
    let output = "\
sat
(
  (define-fun calldata_0 () (_ BitVec 8)
    #x00)
)
";
    let constraints =
        vec![SymBoolExpr::eq(SymExpr::var("calldata_0"), SymExpr::constant(U256::from(1)))];

    let err = validate_solver_model_output(output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
fn fallback_model_finds_wrapping_arithmetic_riddle_candidate() {
    let var = SymExpr::var("calldata_0");
    let msg_sender = U256::from_str_radix("1804c8ab1f12e6bbf3894d4083f33e07309d1f38", 16).unwrap();
    let constraints = vec![
        SymBoolExpr::cmp(
            SymBoolExprOp::Ult,
            SymExpr::op(SymExprOp::Mul, var.clone(), var.clone()),
            SymExpr::constant(msg_sender),
        ),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, var.clone(), SymExpr::constant(msg_sender)),
        SymBoolExpr::eq(
            SymExpr::op(SymExprOp::And, var.clone(), SymExpr::constant(U256::from(0x800))),
            SymExpr::constant(U256::ZERO),
        )
        .not(),
        SymBoolExpr::eq(
            SymExpr::op(SymExprOp::And, var, SymExpr::constant(U256::from(0x10000))),
            SymExpr::constant(U256::ZERO),
        ),
    ];

    let model = fallback_single_var_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn expression_op_simplifies_exact_arithmetic_identities() {
    let x = SymExpr::var("x");

    assert_eq!(
        SymExpr::op(SymExprOp::Mul, x.clone(), SymExpr::constant(U256::ZERO)),
        SymExpr::constant(U256::ZERO)
    );
    assert_eq!(SymExpr::op(SymExprOp::Mul, SymExpr::constant(U256::from(1)), x.clone()), x);
    assert_eq!(
        SymExpr::op(SymExprOp::UDiv, SymExpr::var("x"), SymExpr::constant(U256::from(1))),
        SymExpr::var("x")
    );
    assert_eq!(
        SymExpr::op(SymExprOp::URem, SymExpr::var("x"), SymExpr::constant(U256::from(1))),
        SymExpr::constant(U256::ZERO)
    );
    assert_eq!(
        SymExpr::op(SymExprOp::Sub, SymExpr::var("x"), SymExpr::var("x")),
        SymExpr::constant(U256::ZERO)
    );
    assert_eq!(
        SymExpr::op(SymExprOp::And, SymExpr::var("x"), SymExpr::constant(U256::MAX)),
        SymExpr::var("x")
    );
    assert_eq!(SymExpr::op(SymExprOp::And, x.clone(), x.clone()), x);
    assert_eq!(
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(
                SymExprOp::And,
                SymExpr::var("x"),
                SymExpr::constant((U256::from(1) << 160) - U256::from(1))
            ),
            SymExpr::constant((U256::from(1) << 160) - U256::from(1))
        ),
        SymExpr::op(
            SymExprOp::And,
            SymExpr::var("x"),
            SymExpr::constant((U256::from(1) << 160) - U256::from(1))
        )
    );
    assert_eq!(
        SymExpr::op(
            SymExprOp::Mul,
            SymExpr::constant(U256::from(6)),
            SymExpr::constant(U256::from(7))
        ),
        SymExpr::constant(U256::from(42))
    );
}

#[test]
fn expression_constructors_reuse_live_nodes() {
    let two = SymExpr::constant(U256::from(2));
    assert!(two.ptr_eq(&SymExpr::constant(U256::from(2))));

    let first = SymExpr::op(SymExprOp::Add, SymExpr::var("x"), two);
    let second = SymExpr::op(SymExprOp::Add, SymExpr::var("x"), SymExpr::constant(U256::from(2)));
    assert!(first.ptr_eq(&second));

    let inverted = SymExpr::not(first.clone());
    assert!(inverted.ptr_eq(&SymExpr::not(second)));
    assert!(SymExpr::not(inverted).ptr_eq(&first));
}

#[test]
fn bool_constructors_reuse_live_nodes() {
    let first = SymBoolExpr::cmp(SymBoolExprOp::Ult, SymExpr::var("x"), SymExpr::var("y"));
    let second = SymBoolExpr::cmp(SymBoolExprOp::Ult, SymExpr::var("x"), SymExpr::var("y"));
    assert!(first.ptr_eq(&second));

    let first_not = first.clone().not();
    let second_not = second.not();
    assert!(first_not.ptr_eq(&second_not));
    assert!(first_not.not().ptr_eq(&first));
}

#[test]
fn bool_word_equality_simplifies_to_condition() {
    let condition = SymBoolExpr::cmp(SymBoolExprOp::Ult, SymExpr::var("x"), SymExpr::var("y"));
    let word = SymExpr::from_bool(condition.clone());

    assert_eq!(SymBoolExpr::eq(word.clone(), SymExpr::constant(U256::from(1))), condition);
    assert_eq!(SymBoolExpr::eq(SymExpr::constant(U256::from(1)), word.clone()), condition);
    assert_eq!(SymBoolExpr::eq(word.clone(), SymExpr::constant(U256::ZERO)), condition.not());
    assert_eq!(
        SymBoolExpr::eq(word, SymExpr::constant(U256::from(2))),
        SymBoolExpr::constant(false)
    );
}

#[test]
fn inverted_bool_word_equality_simplifies_to_condition() {
    let condition = SymBoolExpr::cmp(SymBoolExprOp::Ult, SymExpr::var("x"), SymExpr::var("y"));
    let word = SymExpr::ite(
        condition.clone(),
        SymExpr::constant(U256::ZERO),
        SymExpr::constant(U256::from(1)),
    );

    assert_eq!(
        SymBoolExpr::eq(word.clone(), SymExpr::constant(U256::from(1))),
        condition.clone().not()
    );
    assert_eq!(SymBoolExpr::eq(word, SymExpr::constant(U256::ZERO)), condition);
}

#[test]
fn bool_comparison_folds_reflexive_operands() {
    let x = || SymExpr::var("x");

    assert_eq!(SymBoolExpr::cmp(SymBoolExprOp::Ult, x(), x()), SymBoolExpr::constant(false));
    assert_eq!(SymBoolExpr::cmp(SymBoolExprOp::Ugt, x(), x()), SymBoolExpr::constant(false));
    assert_eq!(SymBoolExpr::cmp(SymBoolExprOp::Ule, x(), x()), SymBoolExpr::constant(true));
    assert_eq!(SymBoolExpr::cmp(SymBoolExprOp::Uge, x(), x()), SymBoolExpr::constant(true));
    assert_eq!(SymBoolExpr::cmp(SymBoolExprOp::Slt, x(), x()), SymBoolExpr::constant(false));
    assert_eq!(SymBoolExpr::cmp(SymBoolExprOp::Sgt, x(), x()), SymBoolExpr::constant(false));
}

#[test]
fn bool_comparison_folds_unsigned_boundaries() {
    let x = || SymExpr::var("x");

    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, SymExpr::constant(U256::ZERO), x()),
        SymBoolExpr::constant(false)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Ule, SymExpr::constant(U256::ZERO), x()),
        SymBoolExpr::constant(true)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Ult, x(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::constant(false)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Uge, x(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::constant(true)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Ult, SymExpr::constant(U256::MAX), x()),
        SymBoolExpr::constant(false)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Uge, SymExpr::constant(U256::MAX), x()),
        SymBoolExpr::constant(true)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, x(), SymExpr::constant(U256::MAX)),
        SymBoolExpr::constant(false)
    );
    assert_eq!(
        SymBoolExpr::cmp(SymBoolExprOp::Ule, x(), SymExpr::constant(U256::MAX)),
        SymBoolExpr::constant(true)
    );
}

#[test]
fn exact_modular_arithmetic_handles_zero_modulus_and_wide_intermediates() {
    assert_eq!(U256::MAX.add_mod(U256::from(2), U256::ZERO), U256::ZERO);
    assert_eq!(U256::MAX.mul_mod(U256::MAX, U256::ZERO), U256::ZERO);

    assert_eq!(U256::MAX.add_mod(U256::from(2), U256::MAX), U256::from(2));
    assert_eq!(U256::MAX.mul_mod(U256::MAX, U256::MAX), U256::ZERO);

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert_eq!(
        SymExpr::addmod(
            SymExpr::var("a"),
            SymExpr::constant(U256::from(2)),
            SymExpr::constant(U256::MAX)
        )
        .eval_model(&model)
        .unwrap(),
        U256::from(2)
    );
    assert_eq!(
        SymExpr::mulmod(SymExpr::var("a"), SymExpr::var("a"), SymExpr::constant(U256::MAX))
            .eval_model(&model)
            .unwrap(),
        U256::ZERO
    );
}

#[test]
fn exact_modular_arithmetic_smt_widens_before_modulo() {
    let addmod =
        SymExpr::addmod(SymExpr::var("a"), SymExpr::constant(U256::from(2)), SymExpr::var("m"))
            .smt();
    let mulmod = SymExpr::mulmod(SymExpr::var("a"), SymExpr::var("b"), SymExpr::var("m")).smt();

    assert!(addmod.contains("((_ zero_extend 256) a)"));
    assert!(addmod.contains("bvadd"));
    assert!(addmod.contains("bvurem"));
    assert!(mulmod.contains("((_ zero_extend 256) b)"));
    assert!(mulmod.contains("bvmul"));
    assert!(mulmod.contains("((_ extract 255 0)"));
}

#[test]
fn exact_addmod_model_validation_rejects_wrapping_false_pass() {
    let constraints = vec![SymBoolExpr::eq(
        SymExpr::addmod(
            SymExpr::var("a"),
            SymExpr::constant(U256::from(2)),
            SymExpr::constant(U256::MAX),
        ),
        SymExpr::constant(U256::from(1)),
    )];
    let output = format!("sat\n((define-fun a () (_ BitVec 256) #x{}))\n", "f".repeat(64));

    let err = validate_solver_model_output(&output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
fn solver_normalizes_udiv_zero_predicates_without_bvudiv() {
    let numerator = SymExpr::var("numerator");
    let denominator = SymExpr::var("denominator");
    let div = SymExpr::op(SymExprOp::UDiv, numerator, denominator);
    let original = SymBoolExpr::eq(div, SymExpr::constant(U256::ZERO));
    let normalized = normalize_bool_for_solver(original.clone());

    assert!(!normalized.smt().contains("bvudiv"));

    for (num, den) in [
        (U256::ZERO, U256::ZERO),
        (U256::from(1), U256::ZERO),
        (U256::from(1), U256::from(2)),
        (U256::from(2), U256::from(2)),
        (U256::from(3), U256::from(2)),
        (U256::MAX, U256::MAX),
    ] {
        let model =
            BTreeMap::from([("numerator".to_string(), num), ("denominator".to_string(), den)]);
        assert_eq!(
            original.eval_model(&model).unwrap(),
            normalized.eval_model(&model).unwrap(),
            "num={num} den={den}"
        );
    }
}

#[test]
fn solver_normalizes_udiv_nonzero_predicates_without_bvudiv() {
    let numerator = SymExpr::var("numerator");
    let denominator = SymExpr::var("denominator");
    let div = SymExpr::op(SymExprOp::UDiv, numerator, denominator);
    let original = SymBoolExpr::cmp(SymBoolExprOp::Ugt, div, SymExpr::constant(U256::ZERO));
    let normalized = normalize_bool_for_solver(original.clone());

    assert!(!normalized.smt().contains("bvudiv"));

    for (num, den) in [
        (U256::ZERO, U256::ZERO),
        (U256::from(1), U256::ZERO),
        (U256::from(1), U256::from(2)),
        (U256::from(2), U256::from(2)),
        (U256::from(3), U256::from(2)),
        (U256::MAX, U256::MAX),
    ] {
        let model =
            BTreeMap::from([("numerator".to_string(), num), ("denominator".to_string(), den)]);
        assert_eq!(
            original.eval_model(&model).unwrap(),
            normalized.eval_model(&model).unwrap(),
            "num={num} den={den}"
        );
    }
}

#[test]
fn solver_normalizes_constraint_batches_by_flattening_and_deduping() {
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let a = SymBoolExpr::cmp(SymBoolExprOp::Ult, x.clone(), SymExpr::constant(U256::from(10)));
    let b = SymBoolExpr::eq(y.clone(), SymExpr::constant(U256::from(3)));
    let grouped = vec![
        SymBoolExpr::raw_and(vec![b.clone(), SymBoolExpr::constant(true), a.clone()]),
        a.clone(),
        SymBoolExpr::raw_and(vec![b.clone()]),
    ];

    let normalized = normalize_constraints_for_solver(&grouped);

    assert_eq!(normalized, vec![b, a]);

    let unsat = normalize_constraints_for_solver(&[
        SymBoolExpr::eq(x, y),
        SymBoolExpr::constant(false),
        SymBoolExpr::constant(true),
    ]);
    assert_eq!(unsat, vec![SymBoolExpr::constant(false)]);
}

#[test]
fn solver_normalizes_erc4626_style_share_zero_predicate() {
    let assets = SymExpr::var("assets");
    let supply = SymExpr::var("supply");
    let total_assets = SymExpr::var("total_assets");
    let shares =
        SymExpr::op(SymExprOp::UDiv, SymExpr::op(SymExprOp::Mul, assets, supply), total_assets);
    let constraints = vec![SymBoolExpr::eq(shares, SymExpr::constant(U256::ZERO))];
    let normalized = normalize_constraints_for_solver(&constraints);

    assert_eq!(normalized.len(), 1);
    assert!(!normalized[0].smt().contains("bvudiv"));
    assert!(normalized[0].smt().contains("bvmul"));
}

#[test]
fn solver_rebuilds_word_from_extracted_byte_terms() {
    let masked =
        SymExpr::op(SymExprOp::And, SymExpr::var("word"), SymExpr::constant(U256::from(u64::MAX)));
    let rebuilt = normalize_expr_for_solver(SymExpr::from_bytes(masked.clone().into_byte_exprs()));

    assert_eq!(rebuilt, normalize_expr_for_solver(masked));
}

#[test]
fn solver_rebuilds_word_from_shifted_fragments() {
    let word = SymExpr::var("word");
    let low_bits =
        SymExpr::op(SymExprOp::And, word.clone(), SymExpr::constant(mask_bits(U256::MAX, 32)));
    let high_bits = SymExpr::op(
        SymExprOp::Shl,
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(SymExprOp::Shr, word.clone(), SymExpr::constant(U256::from(32))),
            SymExpr::constant(mask_bits(U256::MAX, 224)),
        ),
        SymExpr::constant(U256::from(32)),
    );
    let rebuilt = normalize_expr_for_solver(SymExpr::op(SymExprOp::Or, low_bits, high_bits));

    assert_eq!(rebuilt, word);
}

#[test]
fn solver_simplifies_selector_prefixed_word_fragment() {
    let x = SymExpr::var("calldata_0");
    let y = SymExpr::var("calldata_1");
    let low_32 = SymExpr::constant(mask_bits(U256::MAX, 32));
    let low_224 = SymExpr::constant(mask_bits(U256::MAX, 224));
    let selector = SymExpr::constant(U256::from(0x771602f7u64) << 224);

    let first_word = SymExpr::op(
        SymExprOp::Or,
        selector,
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(SymExprOp::Shr, x.clone(), SymExpr::constant(U256::from(32))),
            low_224.clone(),
        ),
    );
    let second_word = SymExpr::op(
        SymExprOp::Or,
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(SymExprOp::Shr, y.clone(), SymExpr::constant(U256::from(32))),
            low_224.clone(),
        ),
        SymExpr::op(
            SymExprOp::Shl,
            SymExpr::op(SymExprOp::And, x.clone(), low_32.clone()),
            SymExpr::constant(U256::from(224)),
        ),
    );
    let rebuilt = SymExpr::op(
        SymExprOp::Or,
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(SymExprOp::Shr, second_word.clone(), SymExpr::constant(U256::from(224))),
            low_32,
        ),
        SymExpr::op(
            SymExprOp::Shl,
            SymExpr::op(SymExprOp::And, first_word, low_224),
            SymExpr::constant(U256::from(32)),
        ),
    );

    assert_eq!(normalize_expr_for_solver(rebuilt), x);

    let rebuilt_next = SymExpr::op(
        SymExprOp::Or,
        SymExpr::op(SymExprOp::And, y.clone(), SymExpr::constant(mask_bits(U256::MAX, 32))),
        SymExpr::op(
            SymExprOp::Shl,
            SymExpr::op(SymExprOp::And, second_word, SymExpr::constant(mask_bits(U256::MAX, 224))),
            SymExpr::constant(U256::from(32)),
        ),
    );

    assert_eq!(normalize_expr_for_solver(rebuilt_next), y);
}

fn checked_mul_guard_word(zero_operand: &SymExpr, expected: &SymExpr) -> SymExpr {
    let operand_is_zero = SymBoolExpr::eq(zero_operand.clone(), SymExpr::constant(U256::ZERO));
    let checked_product = SymExpr::ite(
        operand_is_zero.clone(),
        SymExpr::constant(U256::ZERO),
        SymExpr::op(
            SymExprOp::UDiv,
            SymExpr::op(SymExprOp::Mul, zero_operand.clone(), expected.clone()),
            zero_operand.clone(),
        ),
    );

    SymExpr::op(
        SymExprOp::Or,
        SymExpr::from_bool(operand_is_zero),
        SymExpr::from_bool(SymBoolExpr::eq(checked_product, expected.clone())),
    )
}

#[test]
fn solver_normalizes_checked_mul_guard_for_bounded_operands() {
    let a = SymExpr::op(SymExprOp::And, SymExpr::var("a"), SymExpr::constant(U256::from(u64::MAX)));
    let b = SymExpr::op(SymExprOp::And, SymExpr::var("b"), SymExpr::constant(U256::from(u64::MAX)));
    let guard = checked_mul_guard_word(&a, &b);

    assert_eq!(
        normalize_bool_for_solver(SymBoolExpr::eq(guard, SymExpr::constant(U256::ZERO))),
        SymBoolExpr::constant(false)
    );
}

#[test]
fn solver_normalizes_checked_mul_guard_from_path_upper_bound() {
    let a = SymExpr::var("a");
    let factor = SymExpr::constant(U256::from(1_000_000_000_000_000_000u128));
    let guard_is_false =
        SymBoolExpr::eq(checked_mul_guard_word(&a, &factor), SymExpr::constant(U256::ZERO));
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ule, a, SymExpr::constant(U256::from(1000))),
        guard_is_false.clone(),
    ];
    let normalized = normalize_constraints_for_solver(&constraints);

    assert_eq!(normalized, vec![SymBoolExpr::constant(false)]);

    for value in [U256::ZERO, U256::from(1), U256::from(1000)] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert!(!guard_is_false.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_guard() {
    let a = SymExpr::var("a");
    let a_is_zero = SymBoolExpr::eq(a.clone(), SymExpr::constant(U256::ZERO));
    let checked_quotient = SymExpr::ite(
        a_is_zero.clone(),
        SymExpr::constant(U256::ZERO),
        SymExpr::op(SymExprOp::UDiv, a.clone(), a),
    );
    let guard = SymExpr::op(
        SymExprOp::Or,
        SymExpr::from_bool(a_is_zero),
        SymExpr::from_bool(SymBoolExpr::eq(checked_quotient, SymExpr::constant(U256::from(1)))),
    );
    let original = SymBoolExpr::eq(guard, SymExpr::constant(U256::ZERO));
    let normalized = normalize_bool_for_solver(original.clone());

    assert_eq!(normalized, SymBoolExpr::constant(false));

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert!(!original.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_word() {
    let a = SymExpr::var("a");
    let a_is_zero = SymBoolExpr::eq(a.clone(), SymExpr::constant(U256::ZERO));
    let checked_quotient = SymExpr::ite(
        a_is_zero.clone(),
        SymExpr::constant(U256::ZERO),
        SymExpr::op(SymExprOp::UDiv, a.clone(), a),
    );
    let normalized = normalize_expr_for_solver(checked_quotient.clone());

    assert!(!normalized.smt().contains("bvudiv"));
    assert_eq!(
        normalized,
        SymExpr::ite(
            a_is_zero.not(),
            SymExpr::constant(U256::from(1)),
            SymExpr::constant(U256::ZERO)
        )
    );

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert_eq!(
            checked_quotient.eval_model(&model).unwrap(),
            normalized.eval_model(&model).unwrap()
        );
    }
}

#[test]
fn solver_does_not_invert_guarded_zero_self_division() {
    let a = SymExpr::var("a");
    let a_is_zero = SymBoolExpr::eq(a.clone(), SymExpr::constant(U256::ZERO));
    let mirrored = SymExpr::ite(
        a_is_zero,
        SymExpr::op(SymExprOp::UDiv, a.clone(), a),
        SymExpr::constant(U256::ZERO),
    );
    let normalized = normalize_expr_for_solver(mirrored.clone());

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert_eq!(mirrored.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_add_overflow_guard() {
    let a = SymExpr::var("a");
    let a_is_zero = SymBoolExpr::eq(a.clone(), SymExpr::constant(U256::ZERO));
    let checked_quotient = SymExpr::ite(
        a_is_zero,
        SymExpr::constant(U256::ZERO),
        SymExpr::op(SymExprOp::UDiv, a.clone(), a),
    );

    assert_eq!(
        normalize_bool_for_solver(SymBoolExpr::cmp(
            SymBoolExprOp::Ugt,
            checked_quotient.clone(),
            SymExpr::op(SymExprOp::Add, SymExpr::constant(U256::from(1)), checked_quotient),
        )),
        SymBoolExpr::constant(false)
    );
}

#[test]
fn solver_normalizes_checked_mul_guard_with_context_bound() {
    let a = SymExpr::var("a");
    let scale = SymExpr::constant(U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&a, &scale);
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, a.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, a, SymExpr::constant(U256::from(1000))).not(),
        SymBoolExpr::eq(guard, SymExpr::constant(U256::ZERO)),
    ];

    assert_eq!(normalize_constraints_for_solver(&constraints), vec![SymBoolExpr::constant(false)]);
}

#[test]
fn solver_does_not_context_normalize_checked_mul_guard_without_tight_bound() {
    let a = SymExpr::var("a");
    let scale = SymExpr::constant(U256::from(1_000_000_000_000_000_000u128));
    let guard_is_zero =
        SymBoolExpr::eq(checked_mul_guard_word(&a, &scale), SymExpr::constant(U256::ZERO));

    assert_ne!(
        normalize_constraints_for_solver(std::slice::from_ref(&guard_is_zero)),
        vec![SymBoolExpr::constant(false)]
    );
    assert_ne!(
        normalize_constraints_for_solver(&[
            SymBoolExpr::cmp(SymBoolExprOp::Ule, a, SymExpr::constant(U256::MAX)),
            guard_is_zero.clone(),
        ]),
        vec![SymBoolExpr::constant(false)]
    );

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert!(guard_is_zero.eval_model(&model).unwrap());
}

#[test]
fn solver_does_not_normalize_unbounded_checked_mul_guard_to_tautology() {
    let a = SymExpr::var("a");
    let b = SymExpr::var("b");
    let original = SymBoolExpr::eq(checked_mul_guard_word(&a, &b), SymExpr::constant(U256::ZERO));
    let normalized = normalize_bool_for_solver(original.clone());

    assert_ne!(normalized, SymBoolExpr::constant(false));

    let model = BTreeMap::from([("a".to_string(), U256::MAX), ("b".to_string(), U256::from(2))]);
    assert!(original.eval_model(&model).unwrap());
    assert_eq!(original.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
}

#[test]
fn solver_does_not_normalize_checked_mul_guard_when_path_bound_can_overflow() {
    let a = SymExpr::var("a");
    let factor = SymExpr::constant(U256::from(2));
    let original =
        SymBoolExpr::eq(checked_mul_guard_word(&a, &factor), SymExpr::constant(U256::ZERO));
    let normalized = normalize_constraints_for_solver(&[
        SymBoolExpr::cmp(SymBoolExprOp::Ule, a, SymExpr::constant(U256::MAX)),
        original.clone(),
    ]);

    assert!(!matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)));

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert!(original.eval_model(&model).unwrap());
    assert!(normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn solver_normalizes_checked_add_overflow_guard_for_bounded_operands() {
    let a = SymExpr::op(SymExprOp::And, SymExpr::var("a"), SymExpr::constant(U256::from(u64::MAX)));
    let b = SymExpr::op(
        SymExprOp::UDiv,
        SymExpr::op(
            SymExprOp::Mul,
            SymExpr::op(SymExprOp::And, SymExpr::var("b"), SymExpr::constant(U256::from(u64::MAX))),
            a.clone(),
        ),
        SymExpr::op(
            SymExprOp::And,
            SymExpr::var("denominator"),
            SymExpr::constant(U256::MAX >> 64),
        ),
    );

    assert_eq!(
        normalize_bool_for_solver(SymBoolExpr::cmp(
            SymBoolExprOp::Ugt,
            a.clone(),
            SymExpr::op(SymExprOp::Add, a, b),
        )),
        SymBoolExpr::constant(false)
    );
}

#[test]
fn solver_does_not_normalize_unbounded_checked_add_overflow_guard() {
    let a = SymExpr::var("a");
    let b = SymExpr::var("b");
    let original =
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, a.clone(), SymExpr::op(SymExprOp::Add, a, b));
    let normalized = normalize_bool_for_solver(original.clone());

    assert_ne!(normalized, SymBoolExpr::constant(false));

    let model = BTreeMap::from([("a".to_string(), U256::MAX), ("b".to_string(), U256::from(1))]);
    assert!(original.eval_model(&model).unwrap());
    assert_eq!(original.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
}

#[test]
fn solver_detects_monotonic_product_contradiction() {
    let ink =
        SymExpr::op(SymExprOp::And, SymExpr::var("ink"), SymExpr::constant(U256::from(u64::MAX)));
    let art =
        SymExpr::op(SymExprOp::And, SymExpr::var("art"), SymExpr::constant(U256::from(u64::MAX)));
    let spot =
        SymExpr::op(SymExprOp::And, SymExpr::var("spot"), SymExpr::constant(U256::from(u64::MAX)));
    let rate =
        SymExpr::op(SymExprOp::And, SymExpr::var("rate"), SymExpr::constant(U256::from(u64::MAX)));
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, ink.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, art.clone(), ink.clone()),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, spot.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, rate.clone(), spot.clone()),
        SymBoolExpr::cmp(
            SymBoolExprOp::Ult,
            SymExpr::op(SymExprOp::Mul, ink, spot),
            SymExpr::op(SymExprOp::Mul, art, rate),
        )
        .not(),
    ];

    assert!(product_monotonic_unsat(&constraints));
}

#[test]
fn solver_does_not_prune_wrapping_product_inequality() {
    let ink = SymExpr::var("ink");
    let art = SymExpr::var("art");
    let spot = SymExpr::var("spot");
    let rate = SymExpr::var("rate");
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, ink.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, art.clone(), ink.clone()),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, spot.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, rate.clone(), spot.clone()),
        SymBoolExpr::cmp(
            SymBoolExprOp::Ult,
            SymExpr::op(SymExprOp::Mul, ink, spot),
            SymExpr::op(SymExprOp::Mul, art, rate),
        )
        .not(),
    ];

    assert!(!product_monotonic_unsat(&constraints));

    let model = BTreeMap::from([
        ("ink".to_string(), U256::MAX - U256::from(2)),
        ("spot".to_string(), U256::MAX - U256::from(2)),
        ("art".to_string(), U256::MAX - U256::from(1)),
        ("rate".to_string(), U256::MAX - U256::from(1)),
    ]);
    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_detection_flags_symbolic_mul_div_and_mod() {
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");

    assert!(SymExpr::op(SymExprOp::Mul, x.clone(), y.clone()).contains_hard_arith());
    assert!(SymExpr::op(SymExprOp::UDiv, x.clone(), y.clone()).contains_hard_arith());
    assert!(SymExpr::op(SymExprOp::URem, x.clone(), y).contains_hard_arith());
    assert!(
        !SymExpr::op(SymExprOp::Mul, x, SymExpr::constant(U256::from(1))).contains_hard_arith()
    );
}

#[test]
fn hard_arithmetic_fallback_finds_multi_variable_candidate() {
    let first = SymExpr::var("first");
    let donation = SymExpr::var("donation");
    let second = SymExpr::var("second");
    let denominator = SymExpr::op(SymExprOp::Add, first.clone(), donation.clone());
    let shares = SymExpr::op(
        SymExprOp::UDiv,
        SymExpr::op(SymExprOp::Mul, second.clone(), first.clone()),
        denominator,
    );
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, first, SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, donation, SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, second, SymExpr::constant(U256::ZERO)),
        SymBoolExpr::eq(shares, SymExpr::constant(U256::ZERO)),
    ];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_finds_wrapping_product_inequality_candidate() {
    let ink = SymExpr::var("ink");
    let art = SymExpr::var("art");
    let spot = SymExpr::var("spot");
    let rate = SymExpr::var("rate");
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, ink.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, art.clone(), ink.clone()),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, spot.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, rate.clone(), spot.clone()),
        SymBoolExpr::cmp(
            SymBoolExprOp::Ult,
            SymExpr::op(SymExprOp::Mul, ink, spot),
            SymExpr::op(SymExprOp::Mul, art, rate),
        )
        .not(),
    ];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_finds_exact_mulmod_wide_intermediate_candidate() {
    let a = SymExpr::var("a");
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, a.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::eq(
            SymExpr::mulmod(a.clone(), a, SymExpr::constant(U256::MAX)),
            SymExpr::constant(U256::ZERO),
        ),
    ];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    assert_eq!(model.get(&Symbol::intern("a")), Some(&U256::MAX));
}

#[test]
fn hard_arithmetic_fallback_rejects_unvalidated_partial_model() {
    let a = SymExpr::var("a");
    let b = SymExpr::var("b");
    let c = SymExpr::var("c");
    let d = SymExpr::var("d");
    let e = SymExpr::var("e");
    let f = SymExpr::var("f");
    let constraints = vec![
        SymBoolExpr::eq(SymExpr::op(SymExprOp::Mul, a, b), SymExpr::constant(U256::from(1))),
        SymBoolExpr::eq(c, SymExpr::constant(U256::from(3))),
        SymBoolExpr::eq(d, SymExpr::constant(U256::from(4))),
        SymBoolExpr::eq(e, SymExpr::constant(U256::from(5))),
        SymBoolExpr::eq(f, SymExpr::constant(U256::from(6))),
    ];

    let Some(model) = hard_arith_fallback_model(&constraints) else { return };

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_skips_symbolic_hashes() {
    let x = SymExpr::var("x");
    let hash = SymExpr::hash_symbol(Symbol::intern("hash"), "sha256", vec![x.clone()]);
    let constraints = vec![SymBoolExpr::eq(
        SymExpr::op(SymExprOp::Mul, x, hash),
        SymExpr::constant(U256::from(1)),
    )];

    assert!(hard_arith_fallback_model(&constraints).is_none());
}

#[test]
fn concrete_dynamic_array_return_uses_raw_abi_encoding() {
    let return_data = abi_concrete_value_return(DynSolValue::Array(vec![
        DynSolValue::Uint(U256::from(1), 256),
        DynSolValue::Uint(U256::from(2), 256),
    ]));
    let encoded = return_data.read_concrete("test return data").unwrap();
    let decoded = DynSolType::Array(Box::new(DynSolType::Uint(256))).abi_decode(&encoded).unwrap();

    assert_eq!(
        decoded,
        DynSolValue::Array(vec![
            DynSolValue::Uint(U256::from(1), 256),
            DynSolValue::Uint(U256::from(2), 256),
        ])
    );
}

#[test]
fn query_limit_is_enforced_before_spawning_solver() {
    let mut solver = SmtLibSubprocessSolver::new(
        Ok(vec![SolverCommand::new(vec!["missing-z3".to_string()], false).unwrap()]),
        None,
        0,
        false,
    );

    let err = solver.is_sat(&[]).unwrap_err();

    assert!(matches!(err, SymbolicError::SolverQueryLimit(0)));
    assert_eq!(solver.stats().solver_queries, 0);
}

#[test]
fn known_solver_names_resolve_to_smtlib_commands() {
    for solver in BUILTIN_SYMBOLIC_SOLVERS {
        assert!(symbolic_solver_is_builtin(solver));
        named_solver_command(solver).unwrap();
    }
    assert!(!symbolic_solver_is_builtin("yices-2.7.0"));
    assert!(!symbolic_solver_is_builtin("cvc5-1.3.4"));
    assert!(!symbolic_solver_is_builtin("bitwuzla-0.9.0"));

    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "yices".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program(), "yices-smt2");
    assert_eq!(command.args(), &["--bvconst-in-decimal".to_string()]);
    assert!(!command.smt_timeout());

    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "cvc5-int".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program(), "cvc5");
    assert!(command.args().contains(&"--bv-print-consts-as-indexed-symbols".to_string()));
    assert!(!command.smt_timeout());

    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "bitwuzla-abs".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program(), "bitwuzla");
    assert_eq!(command.args(), &["--produce-models".to_string(), "--abstraction".to_string()]);
    assert!(!command.smt_timeout());
}

#[test]
fn solver_command_overrides_solver_name() {
    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "z3".to_string(),
        solver_command: Some("custom-solver --flag 'two words'".to_string()),
        solver_portfolio: vec!["cvc5".to_string(), "bitwuzla".to_string()],
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(commands.len(), 1);
    assert_eq!(command.program(), "custom-solver");
    assert_eq!(command.args(), &["--flag".to_string(), "two words".to_string()]);
    assert!(!command.smt_timeout());
}

#[test]
fn split_solver_command_preserves_empty_quoted_args() {
    let parts = split_solver_command(r#"custom-solver "" arg ''"#).unwrap();

    assert_eq!(parts, vec!["custom-solver", "", "arg", ""]);
}

#[test]
fn split_solver_command_rejects_unterminated_double_quote() {
    let err = split_solver_command(r#"z3 "unterm"#).unwrap_err();

    assert!(
        matches!(err, SolverConfigError::UnterminatedQuote('"')),
        "expected UnterminatedQuote('\"'), got {err:?}"
    );
    assert_eq!(err.to_string(), r#"unterminated " quote in symbolic solver command"#);
}

#[test]
fn split_solver_command_rejects_unterminated_single_quote() {
    let err = split_solver_command("z3 'unterm").unwrap_err();

    assert!(
        matches!(err, SolverConfigError::UnterminatedQuote('\'')),
        "expected UnterminatedQuote('\\''), got {err:?}"
    );
    assert_eq!(err.to_string(), "unterminated ' quote in symbolic solver command");
}

#[test]
fn custom_solver_names_remain_z3_compatible() {
    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "/opt/solvers/z3-nightly".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program(), "/opt/solvers/z3-nightly");
    assert_eq!(command.args(), &["-in".to_string(), "-smt2".to_string()]);
    assert!(command.smt_timeout());
}

#[test]
fn solver_portfolio_resolves_parallel_commands() {
    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "z3".to_string(),
        solver_portfolio: vec![
            "z3".to_string(),
            "cvc5".to_string(),
            "custom-wrapper --stdin".to_string(),
            "  ".to_string(),
        ],
        ..Default::default()
    })
    .unwrap();

    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].program(), "z3");
    assert_eq!(commands[0].args(), &["-in".to_string(), "-smt2".to_string()]);
    assert!(commands[0].smt_timeout());
    assert_eq!(commands[1].program(), "cvc5");
    assert!(commands[1].args().contains(&"--bv-print-consts-as-indexed-symbols".to_string()));
    assert_eq!(commands[2].program(), "custom-wrapper");
    assert_eq!(commands[2].args(), &["--stdin".to_string()]);
    assert!(!commands[2].smt_timeout());
}

#[test]
fn solver_portfolio_availability_warning_reports_missing_entries() {
    let warning = symbolic_solver_portfolio_availability_warning(&SymbolicConfig {
        solver_portfolio: vec!["foundry-missing-symbolic-solver".to_string()],
        ..Default::default()
    })
    .unwrap();

    assert!(warning.contains("Symbolic solver portfolio is degraded"));
    assert!(warning.contains("foundry-missing-symbolic-solver"));
    assert!(warning.contains("No configured portfolio entries are currently available"));

    assert!(
        symbolic_solver_portfolio_availability_warning(&SymbolicConfig {
            solver_portfolio: vec!["foundry-missing-symbolic-solver".to_string()],
            solver_command: Some("custom-solver".to_string()),
            ..Default::default()
        })
        .is_none()
    );
}

#[cfg(unix)]
/// Returns a unique marker path for solver portfolio tests.
fn portfolio_test_marker(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "foundry-symbolic-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ))
}

#[cfg(unix)]
/// Returns a fake solver command that counts invocations before emitting `response`.
fn counted_solver_command(marker: &Path, response: &str) -> SolverCommand {
    counted_solver_command_sequence(marker, &[response])
}

/// Returns a fake solver command that emits one response per invocation.
fn counted_solver_command_sequence(marker: &Path, responses: &[&str]) -> SolverCommand {
    let mut args = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "count=$(cat \"$1\" 2>/dev/null || printf 0); \
                 count=$((count + 1)); printf '%s' \"$count\" > \"$1\"; \
                 cat >/dev/null; shift; \
                 idx=$count; while [ \"$idx\" -gt 1 ] && [ \"$#\" -gt 1 ]; do \
                    shift; idx=$((idx - 1)); \
                 done; \
                 printf '%s\\n' \"$1\""
            .to_string(),
        "sh".to_string(),
        marker.display().to_string(),
    ];
    args.extend(responses.iter().map(|response| response.to_string()));
    SolverCommand::new(args, false).unwrap()
}

#[cfg(unix)]
/// Returns how many times a counted fake solver command was invoked.
fn counted_solver_invocations(marker: &Path) -> usize {
    std::fs::read_to_string(marker).ok().and_then(|count| count.parse().ok()).unwrap_or_default()
}

#[cfg(unix)]
#[test]
fn is_sat_uses_validated_hard_arithmetic_fallback_before_solver() {
    let marker = portfolio_test_marker("hard-arith-is-sat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, x.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, y.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::eq(SymExpr::op(SymExprOp::Mul, x, y), SymExpr::constant(U256::from(4))),
    ];
    let normalized = normalize_constraints_for_solver(&constraints);
    let model = hard_arith_fallback_model(&normalized).unwrap();

    assert!(normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    assert!(solver.is_sat(&constraints).unwrap());
    assert!(solver.is_sat(&constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(solver.heuristic_witnesses(), 1);
    assert_eq!(counted_solver_invocations(&marker), 0);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn model_uses_validated_hard_arithmetic_fallback_cache() {
    let marker = portfolio_test_marker("hard-arith-model-cache");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let constraints = vec![
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, x.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, y.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::eq(SymExpr::op(SymExprOp::Mul, x, y), SymExpr::constant(U256::from(4))),
    ];

    let first = solver.model(&constraints).unwrap();
    assert!(constraints.iter().all(|constraint| constraint.eval_model(&first).unwrap()));
    let second = solver.model(&constraints).unwrap();
    assert_eq!(first, second);
    assert!(solver.is_sat(&constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.model_queries, 2);
    assert_eq!(stats.model_cache_hits, 1);
    assert_eq!(stats.sat_queries, 1);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(solver.heuristic_witnesses(), 1);
    assert_eq!(counted_solver_invocations(&marker), 0);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn is_sat_hard_arithmetic_without_witness_still_honors_solver_unsat() {
    let marker = portfolio_test_marker("hard-arith-is-sat-unsat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let constraints = vec![
        SymBoolExpr::eq(x.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::eq(SymExpr::op(SymExprOp::Mul, x, y), SymExpr::constant(U256::from(1))),
    ];
    let normalized = normalize_constraints_for_solver(&constraints);

    assert!(hard_arith_fallback_model(&normalized).is_none());
    assert!(!solver.is_sat(&constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 1);
    assert_eq!(stats.sat_cache_hits, 0);
    assert_eq!(solver.heuristic_witnesses(), 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_reuses_normalized_is_sat_results() {
    let marker = portfolio_test_marker("sat-cache");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let constraints = vec![
        SymBoolExpr::constant(true),
        SymBoolExpr::eq(x.clone(), SymExpr::constant(U256::from(1))),
        SymBoolExpr::eq(y.clone(), SymExpr::constant(U256::from(2))),
    ];
    let reordered_constraints = vec![
        SymBoolExpr::eq(y, SymExpr::constant(U256::from(2))),
        SymBoolExpr::eq(x, SymExpr::constant(U256::from(1))),
    ];

    assert!(solver.is_sat(&constraints).unwrap());
    assert!(solver.is_sat(&reordered_constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(stats.model_queries, 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_reuses_nested_commutative_results() {
    let marker = portfolio_test_marker("sat-cache-canonical");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let constraints = vec![SymBoolExpr::and(vec![
        SymBoolExpr::eq(
            SymExpr::op(SymExprOp::Add, x.clone(), y.clone()),
            SymExpr::constant(U256::from(3)),
        ),
        SymBoolExpr::eq(x.clone(), SymExpr::constant(U256::from(1))),
    ])];
    let reordered_constraints = vec![SymBoolExpr::and(vec![
        SymBoolExpr::eq(SymExpr::constant(U256::from(1)), x),
        SymBoolExpr::eq(
            SymExpr::constant(U256::from(3)),
            SymExpr::op(SymExprOp::Add, y, SymExpr::var("x")),
        ),
    ])];

    assert!(solver.is_sat(&constraints).unwrap());
    assert!(solver.is_sat(&reordered_constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_deduplicates_repeated_constraints() {
    let marker = portfolio_test_marker("sat-cache-dedup");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let constraint = SymBoolExpr::eq(x, SymExpr::constant(U256::from(1)));
    let duplicated = vec![constraint.clone(), constraint.clone()];
    let deduplicated = vec![constraint];

    assert!(solver.is_sat(&duplicated).unwrap());
    assert!(solver.is_sat(&deduplicated).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_deduplicates_nested_repeated_constraints() {
    let marker = portfolio_test_marker("sat-cache-nested-dedup");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let constraint = SymBoolExpr::eq(x, SymExpr::constant(U256::from(1)));
    let duplicated = vec![SymBoolExpr::raw_and(vec![constraint.clone(), constraint.clone()])];
    let deduplicated = vec![constraint];

    assert!(solver.is_sat(&duplicated).unwrap());
    assert!(solver.is_sat(&deduplicated).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_flattens_grouped_conjunction_keys() {
    let marker = portfolio_test_marker("sat-cache-and-flatten");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let z = SymExpr::var("z");
    let a = SymBoolExpr::eq(x, SymExpr::constant(U256::from(1)));
    let b = SymBoolExpr::eq(y, SymExpr::constant(U256::from(2)));
    let c = SymBoolExpr::eq(z, SymExpr::constant(U256::from(3)));
    let grouped = vec![SymBoolExpr::raw_and(vec![
        SymBoolExpr::raw_and(vec![b.clone(), c.clone()]),
        a.clone(),
    ])];
    let split = vec![c, a, b];

    assert!(solver.is_sat(&grouped).unwrap());
    assert!(solver.is_sat(&split).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_reuses_reversed_comparisons() {
    let marker = portfolio_test_marker("sat-cache-reversed-cmp");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 3, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");

    assert!(solver.is_sat(&[SymBoolExpr::cmp(SymBoolExprOp::Ugt, x.clone(), y.clone())]).unwrap());
    assert!(solver.is_sat(&[SymBoolExpr::cmp(SymBoolExprOp::Ult, y.clone(), x.clone())]).unwrap());
    assert!(solver.is_sat(&[SymBoolExpr::cmp(SymBoolExprOp::Uge, x.clone(), y.clone())]).unwrap());
    assert!(solver.is_sat(&[SymBoolExpr::cmp(SymBoolExprOp::Ule, y.clone(), x.clone())]).unwrap());
    assert!(solver.is_sat(&[SymBoolExpr::cmp(SymBoolExprOp::Sgt, x.clone(), y.clone())]).unwrap());
    assert!(solver.is_sat(&[SymBoolExpr::cmp(SymBoolExprOp::Slt, y, x)]).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 3);
    assert_eq!(stats.sat_queries, 6);
    assert_eq!(stats.sat_cache_hits, 3);
    assert_eq!(counted_solver_invocations(&marker), 3);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_reuses_unsat_branch_complement() {
    let marker = portfolio_test_marker("sat-cache-branch-complement");
    let commands = vec![counted_solver_command_sequence(&marker, &["sat", "unsat"])];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = SymExpr::var("x");
    let base = SymBoolExpr::cmp(SymBoolExprOp::Ult, x.clone(), SymExpr::constant(U256::from(10)));
    let condition = SymBoolExpr::eq(x, SymExpr::constant(U256::from(1)));

    assert!(solver.is_sat(std::slice::from_ref(&base)).unwrap());
    assert!(!solver.is_sat_branch(&[base.clone(), condition.clone()]).unwrap());
    assert!(solver.is_sat_branch(&[base, condition.not()]).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 2);
    assert_eq!(stats.sat_queries, 3);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(counted_solver_invocations(&marker), 2);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_does_not_reuse_unsat_branch_complement_with_unsat_base() {
    let marker = portfolio_test_marker("sat-cache-unsat-base-branch-complement");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = SymExpr::var("x");
    let base = SymBoolExpr::and(vec![
        SymBoolExpr::cmp(SymBoolExprOp::Uge, x.clone(), SymExpr::constant(U256::from(2))),
        SymBoolExpr::cmp(SymBoolExprOp::Ule, x.clone(), SymExpr::constant(U256::from(1))),
    ]);
    let condition = SymBoolExpr::eq(x, SymExpr::constant(U256::from(1)));

    assert!(!solver.is_sat_branch(&[base.clone(), condition.clone()]).unwrap());
    assert!(!solver.is_sat_branch(&[base, condition.not()]).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 2);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 0);
    assert_eq!(counted_solver_invocations(&marker), 2);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn sat_cache_does_not_cache_unknown_results() {
    let marker = portfolio_test_marker("sat-cache-unknown");
    let commands = vec![counted_solver_command(&marker, "unknown")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 4, false);
    let constraints = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)))];

    assert!(matches!(solver.is_sat(&constraints), Err(SymbolicError::SolverUnknown)));
    assert!(matches!(solver.is_sat(&constraints), Err(SymbolicError::SolverUnknown)));

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 2);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 0);
    assert_eq!(counted_solver_invocations(&marker), 2);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn model_cache_reuses_normalized_model_results() {
    let marker = portfolio_test_marker("model-cache");
    let model_output = "sat\n\
        ((define-fun x () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000001)\n\
        (define-fun y () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000002))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var("x");
    let y = SymExpr::var("y");
    let constraints = vec![
        SymBoolExpr::constant(true),
        SymBoolExpr::eq(x.clone(), SymExpr::constant(U256::from(1))),
        SymBoolExpr::eq(y.clone(), SymExpr::constant(U256::from(2))),
    ];
    let reordered_constraints = vec![
        SymBoolExpr::eq(y, SymExpr::constant(U256::from(2))),
        SymBoolExpr::eq(x, SymExpr::constant(U256::from(1))),
    ];

    assert_eq!(solver.model(&constraints).unwrap().get(&Symbol::intern("x")), Some(&U256::from(1)));
    assert_eq!(
        solver.model(&reordered_constraints).unwrap().get(&Symbol::intern("y")),
        Some(&U256::from(2))
    );

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.model_queries, 2);
    assert_eq!(stats.model_cache_hits, 1);
    assert_eq!(stats.sat_queries, 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn model_query_populates_sat_cache() {
    let marker = portfolio_test_marker("model-cache-sat");
    let model_output = "sat\n\
        ((define-fun x () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000001))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let constraints = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)))];

    assert_eq!(solver.model(&constraints).unwrap().get(&Symbol::intern("x")), Some(&U256::from(1)));
    assert!(solver.is_sat(&constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.model_queries, 1);
    assert_eq!(stats.sat_queries, 1);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(stats.model_cache_hits, 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn direct_contradiction_is_sat_short_circuits_locally() {
    let mut solver = SmtLibSubprocessSolver::new(Ok(Vec::new()), None, 1, false);
    let constraint = SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)));
    let constraints = vec![constraint.clone(), constraint.not()];

    assert!(!solver.is_sat(&constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(stats.sat_queries, 1);
}

#[cfg(unix)]
#[test]
fn is_sat_reuses_cached_unsat_subset() {
    let marker = portfolio_test_marker("unsat-subset-cache");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x_eq_one = SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)));
    let y_eq_two = SymBoolExpr::eq(SymExpr::var("y"), SymExpr::constant(U256::from(2)));

    assert!(!solver.is_sat(std::slice::from_ref(&x_eq_one)).unwrap());
    assert!(!solver.is_sat(&[x_eq_one, y_eq_two]).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert!(stats.smt_input_bytes > 0);
    assert!(stats.smt_max_query_bytes > 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn is_sat_does_not_reuse_cached_sat_subset() {
    let marker = portfolio_test_marker("sat-subset-cache");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x_eq_one = SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)));
    let y_eq_two = SymBoolExpr::eq(SymExpr::var("y"), SymExpr::constant(U256::from(2)));

    assert!(solver.is_sat(std::slice::from_ref(&x_eq_one)).unwrap());
    assert!(matches!(
        solver.is_sat(&[x_eq_one, y_eq_two]),
        Err(SymbolicError::SolverQueryLimit(1))
    ));

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn model_short_circuits_when_sat_cache_proved_unsat() {
    let marker = portfolio_test_marker("model-cache-unsat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let constraints = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)))];

    assert!(!solver.is_sat(&constraints).unwrap());
    assert!(
        matches!(solver.model(&constraints), Err(SymbolicError::Solver(message)) if message.contains("unsat"))
    );

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.sat_queries, 1);
    assert_eq!(stats.sat_cache_hits, 0);
    assert_eq!(stats.model_queries, 1);
    assert_eq!(stats.model_cache_hits, 0);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn portfolio_sat_beats_early_unsat() {
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'unsat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; sleep 0.1; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];

    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, false);

    assert!(solver.is_sat(&[]).unwrap());
}

#[cfg(unix)]
#[test]
fn portfolio_scheduler_promotes_recent_sat_winner() {
    let marker = portfolio_test_marker("adaptive-slow-leader");
    let marker_arg = marker.display().to_string();
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "printf started > \"$1\"; cat >/dev/null; sleep 0.3; printf 'unknown\n'"
                    .to_string(),
                "sh".to_string(),
                marker_arg,
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 2, false);

    let first = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)))];
    let second = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(2)))];

    assert!(solver.is_sat(&first).unwrap());
    assert!(marker.exists());
    let _ = std::fs::remove_file(&marker);

    assert!(solver.is_sat(&second).unwrap());
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn portfolio_scheduler_promotes_recent_unsat_winner() {
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; sleep 0.3; printf 'unknown\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'unsat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 2, true);

    solver.capture_diagnostics();

    let first = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(1)))];
    let second = vec![SymBoolExpr::eq(SymExpr::var("x"), SymExpr::constant(U256::from(2)))];

    assert!(!solver.is_sat(&first).unwrap());
    assert!(!solver.is_sat(&second).unwrap());

    let diagnostics = solver.take_diagnostics().unwrap();
    let last_outcomes =
        diagnostics.rsplit("--- symbolic solver portfolio outcomes ---").next().unwrap_or_default();
    assert!(last_outcomes.contains("#1 scheduled +0.000ns"));
    assert!(last_outcomes.contains("printf 'unsat"));
}

#[cfg(unix)]
#[test]
fn portfolio_scheduler_penalizes_invalid_models() {
    let marker = portfolio_test_marker("adaptive-invalid-model");
    let marker_arg = marker.display().to_string();
    let invalid_model = "sat\n((define-fun calldata_0 () (_ BitVec 256) #x0000000000000000000000000000000000000000000000000000000000000000))\n";
    let valid_model = "sat\n((define-fun calldata_0 () (_ BitVec 256) #x0000000000000000000000000000000000000000000000000000000000000001))\n";
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                format!("printf started > \"$1\"; cat >/dev/null; printf '{}'", invalid_model),
                "sh".to_string(),
                marker_arg,
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                format!("cat >/dev/null; printf '{}'", valid_model),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 16, false);

    for query in 0..=PORTFOLIO_SCHEDULER_HISTORY {
        let constraints = vec![
            SymBoolExpr::eq(SymExpr::var("calldata_0"), SymExpr::constant(U256::from(1))),
            SymBoolExpr::eq(
                SymExpr::var(&format!("portfolio_query_{query}")),
                SymExpr::constant(U256::ZERO),
            ),
        ];
        assert_eq!(
            solver.model(&constraints).unwrap().get(&Symbol::intern("calldata_0")),
            Some(&U256::from(1))
        );
        if query == 0 {
            assert!(marker.exists());
            let _ = std::fs::remove_file(&marker);
        } else {
            assert!(!marker.exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn portfolio_winner_skips_delayed_solver() {
    let marker = portfolio_test_marker("delayed-solver");
    let marker_arg = marker.display().to_string();
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "printf started > \"$1\"; cat >/dev/null; printf 'sat\n'".to_string(),
                "sh".to_string(),
                marker_arg,
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, false);

    assert!(solver.is_sat(&[]).unwrap());
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn portfolio_delayed_solver_can_rescue_stalled_leader() {
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; sleep 0.3; printf 'unknown\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, false);
    let started_at = Instant::now();

    assert!(solver.is_sat(&[]).unwrap());
    assert!(started_at.elapsed() >= Duration::from_millis(100));
}

#[cfg(unix)]
#[test]
fn solver_capture_diagnostics_buffers_dump_smt_output() {
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, true);

    solver.capture_diagnostics();

    assert!(solver.is_sat(&[]).unwrap());
    let diagnostics = solver.take_diagnostics().unwrap();

    assert!(diagnostics.contains("--- symbolic SMT query 1 ---"));
    assert!(diagnostics.contains("(check-sat)"));
    assert!(diagnostics.contains("--- symbolic solver portfolio outcomes ---"));
    assert!(diagnostics.contains("winner"));
    assert!(solver.take_diagnostics().is_none());
}

#[cfg(unix)]
#[test]
fn solver_smt_dump_shares_repeated_subterms() {
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, true);
    let byte = SymExpr::var("calldata_0").extracted_byte(0);
    let shifted = SymExpr::op(SymExprOp::Shl, byte, SymExpr::constant(U256::from(248)));
    let constraints = vec![
        SymBoolExpr::eq(shifted.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ugt, shifted, SymExpr::constant(U256::ZERO)),
    ];

    solver.capture_diagnostics();

    assert!(solver.is_sat(&constraints).unwrap());
    let diagnostics = solver.take_diagnostics().unwrap();

    assert!(diagnostics.contains("(define-fun __sym_expr_"));
    assert!(diagnostics.contains("(assert (= __sym_expr_"));
    assert_eq!(diagnostics.matches("(bvlshr calldata_0 (_ bv248 256))").count(), 1);
}

#[test]
fn portfolio_diagnostics_counts_staged_outcomes() {
    let summaries = vec![
        SolverRunSummary::new(
            "primary".to_string(),
            Duration::from_millis(3),
            SolverOutcome::SatValid,
        )
        .with_schedule(0, Duration::ZERO, Some(Duration::ZERO))
        .winner(),
        SolverRunSummary::new("secondary".to_string(), Duration::ZERO, SolverOutcome::NotStarted)
            .with_schedule(1, Duration::from_millis(100), None),
        SolverRunSummary::new(
            "rescue".to_string(),
            Duration::from_millis(5),
            SolverOutcome::Cancelled,
        )
        .with_schedule(2, Duration::from_millis(500), Some(Duration::from_millis(500))),
        SolverRunSummary::new(
            "bad".to_string(),
            Duration::from_millis(1),
            SolverOutcome::SatInvalid,
        )
        .with_schedule(3, Duration::from_millis(1000), Some(Duration::from_millis(1000))),
        SolverRunSummary::new(
            "missing".to_string(),
            Duration::from_millis(1),
            SolverOutcome::Error,
        )
        .with_schedule(4, Duration::from_millis(1500), Some(Duration::from_millis(1500))),
    ];
    let mut diagnostics = PortfolioDiagnostics::default();

    diagnostics.record(&summaries);

    assert_eq!(
        diagnostics.to_string(),
        "\
--- symbolic solver portfolio summary ---
queries: 1
solver runs: 4
rescue solver runs: 3
not-started solver runs: 1
non-primary wins: 0
rescue wins: 0
cancelled after winner: 1
invalid models: 1
solver errors: 1
winner counts:
  primary: 1
launch counts:
  bad: 1
  missing: 1
  primary: 1
  rescue: 1
outcome counts:
  cancelled: 1
  error: 1
  not-started: 1
  sat-invalid: 1
  sat-valid: 1
"
    );
}

#[test]
fn assertion_revert_classifies_assert_panic_only() {
    let mut assert_payload = PANIC_SELECTOR.to_vec();
    assert_payload.extend_from_slice(&U256::from(1).to_be_bytes::<32>());

    let mut overflow_payload = PANIC_SELECTOR.to_vec();
    overflow_payload.extend_from_slice(&U256::from(0x11).to_be_bytes::<32>());

    assert!(is_assertion_revert(&assert_payload));
    assert!(!is_assertion_revert(&overflow_payload));
}

#[test]
fn assertion_revert_ignores_plain_require_reverts() {
    assert!(!is_assertion_revert(&error_payload("hit")));
}

#[test]
fn assertion_revert_accepts_forge_assertion_reverts() {
    assert!(is_assertion_revert(&error_payload("assertion failed: expected 1 to equal 2")));
}

fn error_payload(message: &str) -> Vec<u8> {
    let mut payload = ERROR_SELECTOR.to_vec();
    payload.extend_from_slice(&U256::from(32).to_be_bytes::<32>());
    payload.extend_from_slice(&U256::from(message.len()).to_be_bytes::<32>());
    payload.extend_from_slice(message.as_bytes());
    let padded_len = message.len().div_ceil(32) * 32;
    payload.resize(4 + 64 + padded_len, 0);
    payload
}
