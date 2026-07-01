use super::{abi::*, runtime::*, *};

fn empty_state() -> PathState {
    let mut cx = SymCx::new();
    let calldata =
        SymbolicCalldata::selector_only(&mut cx, &Function::parse("empty()").unwrap()).unwrap();
    PathState::new(&mut cx, Address::ZERO, Address::ZERO, U256::ZERO, calldata, false)
}

fn symbolic_calldata_variants(
    cx: &mut SymCx,
    function: &Function,
    config: &SymbolicConfig,
) -> Result<Vec<SymbolicCalldata>, SymbolicError> {
    SymbolicCalldata::variants(function, config, cx)
}

fn add_words(cx: &mut SymCx, left: SymExpr, right: SymExpr) -> SymExpr {
    cx.op(SymExprOp::Add, left, right)
}

macro_rules! sym_from_bytes {
    ($cx:ident, $bytes:expr) => {{
        let bytes = $bytes.collect::<Vec<_>>();
        $cx.from_bytes(bytes)
    }};
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
    let mut cx = SymCx::new();
    let mut state = empty_state();
    state.stack.push(cx.constant(U256::from(2))).unwrap();
    state.stack.push(cx.constant(U256::from(10))).unwrap();

    state.bin_word(&mut cx, SymExprOp::Sub).unwrap();

    assert_eq!(state.stack.pop().unwrap(), cx.constant(U256::from(8)));
}

#[test]
fn comparison_helpers_use_evm_operand_order() {
    let mut cx = SymCx::new();
    let mut state = empty_state();
    state.stack.push(cx.constant(U256::from(2))).unwrap();
    state.stack.push(cx.constant(U256::from(10))).unwrap();

    state.cmp_word(&mut cx, SymBoolExprOp::Ult).unwrap();

    assert_eq!(state.stack.pop().unwrap(), cx.constant(U256::ZERO));
}

#[test]
fn exp_helper_uses_evm_operand_order() {
    let mut cx = SymCx::new();
    let mut state = empty_state();
    state.stack.push(cx.constant(U256::ZERO)).unwrap();
    state.stack.push(cx.constant(U256::from(0x100))).unwrap();

    state.exp_word(&mut cx).unwrap();

    assert_eq!(state.stack.pop().unwrap(), cx.constant(U256::from(1)));
}

#[test]
fn exp_helper_expands_symbolic_base_for_bounded_concrete_exponent() {
    let mut cx = SymCx::new();
    let mut state = empty_state();
    state.stack.push(cx.constant(U256::from(16))).unwrap();
    state.stack.push(cx.var("base")).unwrap();

    state.exp_word(&mut cx).unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        result.eval_model(&BTreeMap::from([("base".to_string(), U256::from(2))])).unwrap(),
        U256::from(65536)
    );
}

#[test]
fn exp_helper_expands_bounded_symbolic_exponent() {
    let mut cx = SymCx::new();
    let mut state = empty_state();
    let exponent = cx.var("exponent");
    let five = cx.constant(U256::from(5));
    let bound = cx.cmp(SymBoolExprOp::Ule, exponent.clone(), five);
    state.constraints.push(bound);
    state.stack.push(exponent).unwrap();
    state.stack.push(cx.constant(U256::from(3))).unwrap();

    state.exp_word(&mut cx).unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        result.eval_model(&BTreeMap::from([("exponent".to_string(), U256::from(5))])).unwrap(),
        U256::from(243)
    );
}

#[test]
fn shift_helpers_accept_symbolic_amounts() {
    let mut cx = SymCx::new();
    let mut shl = empty_state();
    shl.stack.push(cx.constant(U256::from(1))).unwrap();
    shl.stack.push(cx.var("shift")).unwrap();
    shl.shift_word(&mut cx, ShiftKind::Shl).unwrap();
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
    shr.stack.push(cx.constant(U256::from(1) << 255)).unwrap();
    shr.stack.push(cx.var("shift")).unwrap();
    shr.shift_word(&mut cx, ShiftKind::Shr).unwrap();
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
    sar.stack.push(cx.constant(U256::MAX)).unwrap();
    sar.stack.push(cx.var("shift")).unwrap();
    sar.shift_word(&mut cx, ShiftKind::Sar).unwrap();
    let shifted = sar.stack.pop().unwrap();

    assert_eq!(
        shifted.eval_model(&BTreeMap::from([("shift".to_string(), U256::from(300))])).unwrap(),
        U256::MAX
    );
}

#[test]
fn symbolic_division_guards_zero_divisor() {
    let mut cx = SymCx::new();
    let mut state = empty_state();
    state.stack.push(cx.var("den")).unwrap();
    state.stack.push(cx.var("num")).unwrap();

    state.bin_word_div_zero_guard(&mut cx, SymExprOp::UDiv).unwrap();

    let den = cx.var("den");
    let zero = cx.zero();
    let den_is_zero = cx.eq(den.clone(), zero.clone());
    let num = cx.var("num");
    let quotient = cx.op(SymExprOp::UDiv, num, den);
    let expected = cx.ite(den_is_zero, zero, quotient);
    assert_eq!(state.stack.pop().unwrap(), expected);
}

#[test]
fn symbolic_byte_extracts_with_concrete_index() {
    let mut cx = SymCx::new();
    let word = cx.var("word");
    let actual = byte_word(&mut cx, U256::from(0), word.clone());
    let shift = cx.constant(U256::from(248));
    let shifted = cx.op(SymExprOp::Shr, word, shift);
    let mask = cx.constant(U256::from(0xff));
    let expected = cx.op(SymExprOp::And, shifted, mask);
    assert_eq!(actual, expected);
}

#[test]
fn symbolic_byte_extracts_with_symbolic_index() {
    let mut cx = SymCx::new();
    let word = U256::from_be_bytes(core::array::from_fn::<_, 32, _>(|idx| idx as u8));
    let index = cx.var("index");
    let word = cx.constant(word);
    let byte = byte_word_dynamic(&mut cx, index, word);

    let in_range = BTreeMap::from([("index".to_string(), U256::from(9))]);
    assert_eq!(byte.eval_model(&in_range).unwrap(), U256::from(9));

    let out_of_range = BTreeMap::from([("index".to_string(), U256::from(32))]);
    assert_eq!(byte.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn symbolic_signextend_accepts_symbolic_index() {
    let mut cx = SymCx::new();
    let value = cx.constant(U256::from(0x80));
    let index = cx.var("index");
    let extended = signextend_word_dynamic(&mut cx, index, value);

    let zero_index = BTreeMap::from([("index".to_string(), U256::ZERO)]);
    assert_eq!(extended.eval_model(&zero_index).unwrap(), U256::MAX - U256::from(0x7f));

    let one_index = BTreeMap::from([("index".to_string(), U256::from(1))]);
    assert_eq!(extended.eval_model(&one_index).unwrap(), U256::from(0x80));
}

#[test]
fn symbolic_byte_preserves_concrete_packed_selector_bytes() {
    let mut cx = SymCx::new();
    let selector = U256::from(0x12345678);
    let selector = cx.constant(selector);
    let shift_224 = cx.constant(U256::from(224));
    let selector = cx.op(SymExprOp::Shl, selector, shift_224);
    let arg = cx.var("arg");
    let shift_32 = cx.constant(U256::from(32));
    let shifted_arg = cx.op(SymExprOp::Shr, arg, shift_32);
    let packed = cx.op(SymExprOp::Or, selector, shifted_arg);

    assert_eq!(byte_word(&mut cx, U256::from(0), packed.clone()), cx.constant(U256::from(0x12)));
    assert_eq!(byte_word(&mut cx, U256::from(1), packed.clone()), cx.constant(U256::from(0x34)));
    assert_eq!(byte_word(&mut cx, U256::from(2), packed.clone()), cx.constant(U256::from(0x56)));
    assert_eq!(byte_word(&mut cx, U256::from(3), packed), cx.constant(U256::from(0x78)));
}

#[test]
fn word_reassembly_preserves_split_symbolic_word() {
    let mut cx = SymCx::new();
    let value = cx.var("value");
    let one = cx.one();
    let original = cx.op(SymExprOp::Add, value, one);
    let bytes = original.clone().into_byte_exprs(&mut cx);

    assert_eq!(cx.from_bytes(bytes), original);
}

#[test]
fn symbolic_address_aliases_match_abi_encoded_address_words() {
    let mut cx = SymCx::new();
    let source = cx.var("beneficiary");
    let address_mask = cx.constant((U256::from(1) << 160) - U256::from(1));
    let masked = cx.op(SymExprOp::And, source.clone(), address_mask);
    let mut encoded = vec![cx.zero(); 12];
    encoded.extend((12..32).map(|idx| byte_word(&mut cx, U256::from(idx), masked.clone())));
    let reassembled = cx.from_bytes(encoded);

    assert!(source.symbolic_address_equivalent(&reassembled));
    assert_eq!(source.symbolic_address_key(), reassembled.symbolic_address_key());
}

#[test]
fn selector_shift_simplifies_to_concrete_word() {
    let mut cx = SymCx::new();
    let selector = U256::from(0x12345678);
    let selector_word = cx.constant(selector);
    let shift_224 = cx.constant(U256::from(224));
    let shifted_selector = cx.op(SymExprOp::Shl, selector_word, shift_224.clone());
    let arg = cx.var("arg");
    let shift_32 = cx.constant(U256::from(32));
    let shifted_arg = cx.op(SymExprOp::Shr, arg, shift_32);
    let call_word = cx.op(SymExprOp::Or, shifted_selector, shifted_arg);
    let selector_expr = cx.op(SymExprOp::Shr, call_word, shift_224);

    assert_eq!(selector_expr.known_word(), Some(selector));
}

#[test]
fn selector_equality_folds_known_word_expressions() {
    let mut cx = SymCx::new();
    let selector = U256::from(0x12345678u32);
    let other = U256::from(0x9a8325a0u32);
    let selector_word = cx.constant(selector);
    let shift_224 = cx.constant(U256::from(224));
    let shifted_selector = cx.op(SymExprOp::Shl, selector_word, shift_224.clone());
    let arg = cx.var("arg");
    let shift_32 = cx.constant(U256::from(32));
    let shifted_arg = cx.op(SymExprOp::Shr, arg, shift_32);
    let call_word = cx.op(SymExprOp::Or, shifted_selector, shifted_arg);
    let selector_expr = cx.op(SymExprOp::Shr, call_word, shift_224);

    let selector_word = cx.constant(selector);
    let actual = cx.eq(selector_expr.clone(), selector_word);
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
    let other_word = cx.constant(other);
    let actual = cx.eq(selector_expr, other_word);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
}

#[test]
fn calldata_selector_load_simplifies_to_concrete_word() {
    let mut cx = SymCx::new();
    let function = Function::parse("check(bytes32)").unwrap();
    let calldata = symbolic_calldata_variants(&mut cx, &function, &SymbolicConfig::default())
        .unwrap()
        .remove(0);
    let selector = U256::from_be_slice(function.selector().as_slice());
    let call_data = calldata.call_data(&mut cx);
    let offset = cx.zero();
    let loaded = call_data.load_word(&mut cx, offset).unwrap();
    let shift = cx.constant(U256::from(224));
    let selector_expr = cx.op(SymExprOp::Shr, loaded, shift);

    assert_eq!(selector_expr.known_word(), Some(selector));
    let selector_word = cx.constant(selector);
    let actual = cx.eq(selector_expr, selector_word);
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
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
    let mut cx = SymCx::new();
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = symbolic_calldata_variants(&mut cx, &function, &config).unwrap().remove(0);
    let call_data = calldata.call_data(&mut cx);

    assert_eq!(call_data.size_word(), cx.constant(U256::from(100)));
    assert_eq!(call_data.load(&mut cx, 4).unwrap(), cx.constant(U256::from(32)));
    assert_eq!(call_data.load(&mut cx, 36).unwrap(), cx.constant(U256::from(3)));
    let offset = cx.constant(U256::from(71));
    let bytes = call_data.read_bytes_offset(&mut cx, offset, 1);
    assert_eq!(bytes.byte(&mut cx, 0), cx.zero());

    let model = BTreeMap::from([
        ("calldata_0_0".to_string(), U256::from(1)),
        ("calldata_0_1".to_string(), U256::from(2)),
        ("calldata_0_2".to_string(), U256::from(3)),
    ]);
    assert_eq!(
        calldata.model_to_args(&mut cx, &model).unwrap(),
        vec![DynSolValue::Bytes(vec![1, 2, 3])]
    );
}

#[test]
fn calldata_seed_model_round_trips_common_abi_values() {
    let mut cx = SymCx::new();
    let function = Function::parse(
        "check(bool,uint256,int128,address,bytes3,bytes,string,uint256[],(uint256,bool))",
    )
    .unwrap();
    let config = SymbolicConfig {
        default_array_lengths: vec![2],
        default_bytes_lengths: vec![2],
        ..Default::default()
    };
    let mut fixed = [0u8; 32];
    fixed[..3].copy_from_slice(&[0xaa, 0xbb, 0xcc]);
    let args = vec![
        DynSolValue::Bool(true),
        DynSolValue::Uint(U256::from(42), 256),
        DynSolValue::Int(I256::try_from(-5i64).unwrap(), 128),
        DynSolValue::Address(Address::from([0x11; 20])),
        DynSolValue::FixedBytes(B256::from(fixed), 3),
        DynSolValue::Bytes(vec![1, 2]),
        DynSolValue::String("ok".to_string()),
        DynSolValue::Array(vec![
            DynSolValue::Uint(U256::from(1), 256),
            DynSolValue::Uint(U256::from(2), 256),
        ]),
        DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(9), 256), DynSolValue::Bool(false)]),
    ];
    let seed = SymbolicConcreteInput {
        calldata: Bytes::from(function.abi_encode_input(&args).unwrap()),
        args: args.clone(),
    };
    let calldata = symbolic_calldata_variants(&mut cx, &function, &config).unwrap().remove(0);
    let model = calldata.seed_model(&mut cx, &seed).unwrap();

    assert_eq!(calldata.model_to_args(&mut cx, &model).unwrap(), args);
}

#[test]
fn calldata_word_loads_preserve_structural_words() {
    let mut cx = SymCx::new();
    let function = Function::parse("check(uint256,bool,address)").unwrap();
    let calldata = symbolic_calldata_variants(&mut cx, &function, &SymbolicConfig::default())
        .unwrap()
        .remove(0);
    let call_data = calldata.call_data(&mut cx);

    assert_eq!(call_data.load(&mut cx, 4).unwrap(), cx.var("calldata_0"));
    assert_eq!(call_data.load(&mut cx, 36).unwrap(), cx.var("calldata_1"));
    assert_eq!(call_data.load(&mut cx, 68).unwrap(), cx.var("calldata_2"));
}

#[test]
fn calldata_copy_to_memory_preserves_structural_words() {
    let mut cx = SymCx::new();
    let function = Function::parse("check(uint256,uint256)").unwrap();
    let calldata = symbolic_calldata_variants(&mut cx, &function, &SymbolicConfig::default())
        .unwrap()
        .remove(0);
    let mut memory = SymMemory::default();
    let dest = cx.constant(U256::from(0x80));
    let offset = cx.constant(U256::from(4));
    let call_data = calldata.call_data(&mut cx);

    memory.copy_calldata_to_offset(&mut cx, dest, offset, 64, &call_data).unwrap();

    assert_eq!(memory.load_word(&mut cx, 0x80).unwrap(), cx.var("calldata_0"));
    assert_eq!(memory.load_word(&mut cx, 0xa0).unwrap(), cx.var("calldata_1"));
}

#[test]
fn byte_concat_word_load_assembles_word_fragments() {
    let mut cx = SymCx::new();
    let selector = [0xde, 0xad, 0xbe, 0xef];
    let word = cx.var("calldata_0");
    let selector_bytes = SymBytes::concrete(&mut cx, selector.to_vec());
    let word_bytes = word.clone().into_bytes(&mut cx);
    let bytes = SymBytes::concat(&mut cx, [selector_bytes, word_bytes]);

    assert_eq!(bytes.word_at(&mut cx, 4), word);

    let mut word_value = [0u8; 32];
    for (idx, byte) in word_value.iter_mut().enumerate() {
        *byte = idx as u8 + 1;
    }
    let mut expected = [0u8; 32];
    expected[..4].copy_from_slice(&selector);
    expected[4..].copy_from_slice(&word_value[..28]);

    assert_eq!(
        bytes
            .word_at(&mut cx, 0)
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
    let mut cx = SymCx::new();
    let immediate = cx.var("push_data");
    let opcode = SymBytes::concrete(&mut cx, vec![opcode::PUSH32]);
    let immediate_bytes = immediate.clone().into_bytes(&mut cx);
    let bytes = SymBytes::concat(&mut cx, [opcode, immediate_bytes]);

    assert_eq!(bytes.right_aligned_word(&mut cx, 1, 32), immediate);
    assert_eq!(
        bytes
            .right_aligned_word(&mut cx, 29, 4)
            .eval_model(&BTreeMap::from([("push_data".to_string(), U256::from(0x12345678))]))
            .unwrap(),
        U256::from(0x12345678)
    );
}

#[test]
fn calldata_load_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let calldata_bytes = (0u8..40).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let calldata_bytes = SymBytes::exprs(&mut cx, calldata_bytes);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let offset = cx.var("offset");
    let loaded = calldata.load_word(&mut cx, offset).unwrap();
    let expected = sym_from_bytes!(cx, (1u8..33).map(|idx| cx.constant(U256::from(idx + 1))));

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let bytes = vec![
        cx.constant(U256::from(0xaa)),
        cx.constant(U256::from(0xbb)),
        cx.constant(U256::from(0xcc)),
        cx.constant(U256::from(0xdd)),
    ];
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.store_bytes(&mut cx, 0, bytes);
    let size = cx.var("size");
    let bounded_size = BoundedCopySize::Symbolic { size, max_size: 4 };
    let offset = cx.zero();
    let input = bounded_size.read_from_memory(&mut cx, &memory, offset);
    let calldata = bounded_size.calldata(&mut cx, input);
    let model = BTreeMap::from([("size".to_string(), U256::from(2))]);

    assert_eq!(calldata.size_word().eval_model(&model).unwrap(), U256::from(2));
    let offset = cx.zero();
    let bytes = calldata.read_bytes_offset(&mut cx, offset, 4);
    assert_eq!(bytes.byte(&mut cx, 0).eval_model(&model).unwrap(), U256::from(0xaa));
    assert_eq!(bytes.byte(&mut cx, 1).eval_model(&model).unwrap(), U256::from(0xbb));
    assert_eq!(bytes.byte(&mut cx, 2).eval_model(&model).unwrap(), U256::ZERO);
    assert_eq!(bytes.byte(&mut cx, 3).eval_model(&model).unwrap(), U256::ZERO);
}

#[test]
fn memory_copies_unaligned_symbolic_calldata_bytes() {
    let mut cx = SymCx::new();
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = symbolic_calldata_variants(&mut cx, &function, &config).unwrap().remove(0);
    let mut memory = SymMemory::default();
    let dest = cx.constant(U256::from(1));
    let offset = cx.constant(U256::from(68));
    let call_data = calldata.call_data(&mut cx);

    memory.copy_calldata_to_offset(&mut cx, dest, offset, 3, &call_data).unwrap();
    let word = memory.load_word(&mut cx, 0).unwrap();

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
    let mut cx = SymCx::new();
    let calldata_bytes = (0u8..40).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let calldata_bytes = SymBytes::exprs(&mut cx, calldata_bytes);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let mut memory = SymMemory::default();

    let dest = cx.zero();
    let offset = cx.var("offset");
    memory.copy_calldata_to_offset(&mut cx, dest, offset, 2, &calldata).unwrap();
    let word = memory.load_word(&mut cx, 0).unwrap();

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
    let mut cx = SymCx::new();
    let calldata_bytes = (0u8..8).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let calldata_bytes = SymBytes::exprs(&mut cx, calldata_bytes);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = cx.zero();
    let offset = cx.zero();
    let size = cx.var("size");

    memory.copy_calldata_symbolic_size(&mut cx, dest, offset, size, 4, &calldata).unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_copies_symbolic_bytecode_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = cx.zero();
    let size = cx.var("size");
    let bytes = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.copy_bytes_size_offset(&mut cx, dest, size, bytes).unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_copies_folded_symbolic_size_prefix_only() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = cx.zero();
    let size = cx.constant(U256::from(2));
    let bytes = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.copy_bytes_size_offset(&mut cx, dest, size, bytes).unwrap();

    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &BTreeMap::new()).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
}

#[test]
fn memory_skips_folded_zero_symbolic_size_copy() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let dest = cx.zero();
    let size = cx.zero();
    let bytes = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.copy_bytes_size_offset(&mut cx, dest, size, bytes).unwrap();

    assert_eq!(memory.size_word(&mut cx), cx.zero());
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &BTreeMap::new()).unwrap(),
        vec![0, 0, 0, 0]
    );
}

#[test]
fn memory_reads_symbolic_size_with_zero_guarded_tail() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let source = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let source = SymBytes::exprs(&mut cx, source);
    memory.store_bytes(&mut cx, 32, source);

    let offset = cx.constant(U256::from(32));
    let size = cx.var("size");
    let bytes = memory.read_bytes_symbolic_size(&mut cx, offset, size, 4);

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(bytes.eval_model(&mut cx, &size_two).unwrap(), vec![1, 2, 0, 0]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(bytes.eval_model(&mut cx, &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_reads_folded_symbolic_size_prefix_only() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let source = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let source = SymBytes::exprs(&mut cx, source);
    memory.store_bytes(&mut cx, 32, source);

    let offset = cx.constant(U256::from(32));
    let size = cx.constant(U256::from(2));
    let bytes = memory.read_bytes_symbolic_size(&mut cx, offset, size, 4);

    assert_eq!(bytes.eval_model(&mut cx, &BTreeMap::new()).unwrap(), vec![1, 2, 0, 0]);
}

#[test]
fn memory_copies_symbolic_memory_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let source_bytes = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let source_bytes = SymBytes::exprs(&mut cx, source_bytes);
    memory.store_bytes(&mut cx, 32, source_bytes);
    let dest = cx.zero();
    let source = cx.constant(U256::from(32));
    let size = cx.var("size");

    memory.copy_memory_symbolic_size(&mut cx, dest, source, size, 4).unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_copies_symbolic_size_to_symbolic_dest() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0x80, tail);
    let source_bytes = (0u8..4).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let source_bytes = SymBytes::exprs(&mut cx, source_bytes);
    memory.store_bytes(&mut cx, 0x20, source_bytes);
    let dest = cx.var("dest");
    let source = cx.constant(U256::from(0x20));
    let size = cx.var("size");

    memory.copy_memory_symbolic_size(&mut cx, dest, source, size, 4).unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("size".to_string(), U256::from(2)),
    ]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0x80, 4).eval_model(&mut cx, &model).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
}

#[test]
fn memory_copies_symbolic_returndata_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let return_data = SymReturnData::from_concrete_bytes(&mut cx, vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = cx.zero();
    let offset = cx.zero();
    let size = cx.var("size");

    memory.copy_return_data_symbolic_size(&mut cx, dest, offset, size, 4, &return_data).unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn returndata_reads_symbolic_offset() {
    let mut cx = SymCx::new();
    let return_data = SymReturnData::from_concrete_bytes(&mut cx, vec![1, 2, 3, 4]);
    let offset = cx.var("offset");
    let bytes = return_data.read_bytes_offset(&mut cx, offset, 2);

    let offset_one = BTreeMap::from([("offset".to_string(), U256::from(1))]);
    assert_eq!(bytes.eval_model(&mut cx, &offset_one).unwrap(), vec![2, 3]);

    let offset_four = BTreeMap::from([("offset".to_string(), U256::from(4))]);
    assert_eq!(bytes.eval_model(&mut cx, &offset_four).unwrap(), vec![0, 0]);
}

#[test]
fn memory_return_data_accepts_symbolic_size() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let bytes = vec![1, 2, 3, 4].into_iter().map(|byte| cx.constant(U256::from(byte))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.store_bytes(&mut cx, 0, bytes);
    let offset = cx.zero();
    let len = cx.var("len");

    let return_data = memory.return_data_symbolic_size(&mut cx, offset, len, 4).unwrap();

    let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
    assert_eq!(return_data.len_word().eval_model(&len_two).unwrap(), U256::from(2));
    assert_eq!(return_data.byte(&mut cx, 0).eval_model(&len_two).unwrap(), U256::from(1));
    assert_eq!(return_data.byte(&mut cx, 2).eval_model(&len_two).unwrap(), U256::ZERO);
}

#[test]
fn call_output_preserves_memory_beyond_symbolic_returndata_size() {
    let mut cx = SymCx::new();
    let bytes = vec![1, 2, 3, 4].into_iter().map(|byte| cx.constant(U256::from(byte))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    let len = cx.var("len");
    let return_data = SymReturnData::from_bytes_with_len(bytes, len);
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = cx.zero();

    memory
        .copy_call_output_offset(&mut cx, dest, &BoundedCopySize::Concrete(4), &return_data)
        .unwrap();

    let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &len_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let len_four = BTreeMap::from([("len".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &len_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn nested_dynamic_calldata_uses_preorder_lengths() {
    let mut cx = SymCx::new();
    let function = Function::parse("check((uint256[],bytes))").unwrap();
    let config = SymbolicConfig { array_lengths: vec![2, 3], ..Default::default() };
    let calldata = symbolic_calldata_variants(&mut cx, &function, &config).unwrap().remove(0);
    let call_data = calldata.call_data(&mut cx);

    assert_eq!(call_data.load(&mut cx, 4).unwrap(), cx.constant(U256::from(32)));
    assert_eq!(call_data.load(&mut cx, 36).unwrap(), cx.constant(U256::from(64)));
    assert_eq!(call_data.load(&mut cx, 68).unwrap(), cx.constant(U256::from(160)));
    assert_eq!(call_data.load(&mut cx, 100).unwrap(), cx.constant(U256::from(2)));
    assert_eq!(call_data.load(&mut cx, 196).unwrap(), cx.constant(U256::from(3)));
}

#[test]
fn memory_round_trips_symbolic_words_as_bytes() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    memory.store_word(&mut cx, 7, value.clone());

    let model = BTreeMap::from([("word".to_string(), U256::from(0x1234))]);
    assert_eq!(
        memory.load_word(&mut cx, 7).unwrap().eval_model(&model).unwrap(),
        value.eval_model(&model).unwrap()
    );
}

#[test]
fn memory_load_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    memory.store_word(&mut cx, 7, value);
    let offset = cx.var("offset");
    let loaded = memory.load_word_offset(&mut cx, offset).unwrap();

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    let offset = cx.var("offset");
    memory.store_word_offset(&mut cx, offset, value);
    let loaded = memory.load_word(&mut cx, 7).unwrap();

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let concrete = cx.constant(U256::from(0x55));
    memory.store_word(&mut cx, 7, concrete);
    assert_eq!(memory.size_word(&mut cx), cx.constant(U256::from(64)));

    let offset = cx.var("offset");
    let word = cx.var("word");
    memory.store_word_offset(&mut cx, offset, word);
    let size = memory.size_word(&mut cx);

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let offset = cx.var("offset");
    let word = cx.var("word");
    memory.store_word_offset(&mut cx, offset, word);
    let concrete = cx.constant(U256::from(0x55));
    memory.store_word(&mut cx, 7, concrete);
    let loaded = memory.load_word(&mut cx, 7).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x55));
}

#[test]
fn memory_dynamic_read_respects_concrete_overwrite_epoch() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let write_offset = cx.var("write_offset");
    let byte = cx.var("byte");
    memory.store_byte_offset(&mut cx, write_offset, byte);
    let concrete = cx.constant(U256::from(0x55));
    memory.store_byte(&mut cx, 5, concrete);
    let read_offset = cx.var("read_offset");
    let loaded = memory.byte_dynamic_with_delta(&mut cx, &read_offset, 0);

    let model = BTreeMap::from([
        ("write_offset".to_string(), U256::from(5)),
        ("read_offset".to_string(), U256::from(5)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x55));
}

#[test]
fn memory_symbolic_write_after_concrete_overwrite_still_applies() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let older_offset = cx.var("older_offset");
    let older_byte = cx.var("older_byte");
    memory.store_byte_offset(&mut cx, older_offset, older_byte);
    let concrete = cx.constant(U256::from(0x55));
    memory.store_byte(&mut cx, 5, concrete);
    let later_offset = cx.var("later_offset");
    let later_byte = cx.var("later_byte");
    memory.store_byte_offset(&mut cx, later_offset, later_byte);
    let loaded = memory.byte(&mut cx, 5);

    let concrete_wins = BTreeMap::from([
        ("older_offset".to_string(), U256::from(5)),
        ("older_byte".to_string(), U256::from(0xaa)),
        ("later_offset".to_string(), U256::from(6)),
        ("later_byte".to_string(), U256::from(0xbb)),
    ]);
    assert_eq!(loaded.eval_model(&concrete_wins).unwrap(), U256::from(0x55));

    let later_symbolic_wins = BTreeMap::from([
        ("older_offset".to_string(), U256::from(5)),
        ("older_byte".to_string(), U256::from(0xaa)),
        ("later_offset".to_string(), U256::from(5)),
        ("later_byte".to_string(), U256::from(0xbb)),
    ]);
    assert_eq!(loaded.eval_model(&later_symbolic_wins).unwrap(), U256::from(0xbb));
}

#[test]
fn memory_store_byte_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let offset = cx.var("offset");
    let byte = cx.var("byte");
    memory.store_byte_offset(&mut cx, offset, byte);
    let loaded = memory.byte(&mut cx, 0x80);

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    memory.store_word(&mut cx, 7, value);
    let offset = cx.var("offset");
    let bytes = memory.read_bytes_offset(&mut cx, offset, 32).materialize(&mut cx);
    let loaded = cx.from_bytes(bytes);

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    memory.store_word(&mut cx, 7, value);
    let offset = cx.var("offset");
    let return_data = memory.return_data(&mut cx, offset, 32).unwrap();
    let loaded = return_data.load_word(&mut cx, 0).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_copy_accepts_symbolic_source_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    memory.store_word(&mut cx, 7, value);
    let dest = cx.constant(U256::from(64));
    let src = cx.var("src");
    memory.copy_memory_to_offset(&mut cx, dest, src, 32).unwrap();
    let loaded = memory.load_word(&mut cx, 64).unwrap();

    let model = BTreeMap::from([
        ("src".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_copy_accepts_symbolic_destination_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = cx.var("word");

    memory.store_word(&mut cx, 7, value);
    let dest = cx.var("dest");
    let src = cx.constant(U256::from(7));
    memory.copy_memory_to_offset(&mut cx, dest, src, 32).unwrap();
    let loaded = memory.load_word(&mut cx, 64).unwrap();

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let word = cx.var("word");
    let bytes = word.into_bytes(&mut cx);
    let return_data = SymReturnData::from_bytes(&mut cx, bytes);
    let dest = cx.var("dest");

    memory
        .copy_call_output_offset(&mut cx, dest, &BoundedCopySize::Concrete(32), &return_data)
        .unwrap();
    let loaded = memory.load_word(&mut cx, 64).unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(64)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_call_output_accepts_symbolic_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let return_data = SymReturnData::from_concrete_bytes(&mut cx, vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = cx.zero();
    let size = cx.var("size");
    let copy_size = BoundedCopySize::Symbolic { size, max_size: 4 };

    memory.copy_call_output_offset(&mut cx, dest, &copy_size, &return_data).unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_call_output_accepts_symbolic_destination_and_size() {
    let mut cx = SymCx::new();
    let return_data = SymReturnData::from_concrete_bytes(&mut cx, vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0x80, tail);
    let dest = cx.var("dest");
    let size = cx.var("size");
    let copy_size = BoundedCopySize::Symbolic { size, max_size: 4 };

    memory.copy_call_output_offset(&mut cx, dest, &copy_size, &return_data).unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("size".to_string(), U256::from(2)),
    ]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0x80, 4).eval_model(&mut cx, &model).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
}

#[test]
fn memory_call_output_accepts_symbolic_destination_and_return_len() {
    let mut cx = SymCx::new();
    let bytes = vec![1, 2, 3, 4].into_iter().map(|byte| cx.constant(U256::from(byte))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    let len = cx.var("len");
    let return_data = SymReturnData::from_bytes_with_len(bytes, len);
    let mut memory = SymMemory::default();
    let tail = cx.constant(U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0x80, tail);
    let dest = cx.var("dest");

    memory
        .copy_call_output_offset(&mut cx, dest, &BoundedCopySize::Concrete(4), &return_data)
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("len".to_string(), U256::from(2)),
    ]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0x80, 4).eval_model(&mut cx, &model).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
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
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let salt = cx.var("salt");
    let initcode = vec![opcode::STOP];
    let initcode_hash = cx.constant(U256::from_be_bytes(keccak256(&initcode).0));
    let creator_word = cx.constant(address_word(creator));

    let cheatcode_word = compute_create2_address_word(
        &mut cx,
        &mut state,
        creator_word,
        salt.clone(),
        initcode_hash,
    )
    .unwrap();
    let initcode = SymCode::concrete(&mut cx, initcode);
    let opcode_word =
        create2_address_word(&mut cx, &mut state, creator, salt, &initcode).unwrap().0;

    assert_eq!(cheatcode_word, opcode_word);
}

#[test]
fn compute_create2_cheatcode_helper_accepts_symbolic_init_code_hash() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let salt = cx.var("salt");
    let initcode_hash = cx.var("initcode_hash");
    let creator_word = cx.constant(address_word(creator));

    let first = compute_create2_address_word(
        &mut cx,
        &mut state,
        creator_word.clone(),
        salt.clone(),
        initcode_hash.clone(),
    )
    .unwrap();
    let second =
        compute_create2_address_word(&mut cx, &mut state, creator_word, salt, initcode_hash)
            .unwrap();

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create2_address_")));
}

#[test]
fn compute_create_cheatcode_helper_accepts_symbolic_nonce() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let nonce = cx.var("nonce");
    let creator_word = cx.constant(address_word(creator));

    let first =
        compute_create_address_word(&mut cx, &mut state, creator_word.clone(), nonce.clone())
            .unwrap();
    let second = compute_create_address_word(&mut cx, &mut state, creator_word, nonce).unwrap();

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create_address_")));
}

#[test]
fn compute_create_cheatcode_helper_accepts_symbolic_deployer() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let deployer = cx.var("deployer");
    let nonce = cx.var("nonce");

    let first = compute_create_address_word(&mut cx, &mut state, deployer.clone(), nonce.clone())
        .expect("symbolic deployer is supported");
    let second = compute_create_address_word(&mut cx, &mut state, deployer, nonce)
        .expect("symbolic deployer is supported");

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create_address_")));
}

#[test]
fn compute_create2_cheatcode_helper_accepts_symbolic_deployer() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let deployer = cx.var("deployer");
    let salt = cx.var("salt");
    let initcode_hash = cx.var("initcode_hash");

    let first = compute_create2_address_word(
        &mut cx,
        &mut state,
        deployer.clone(),
        salt.clone(),
        initcode_hash.clone(),
    )
    .expect("symbolic deployer is supported");
    let second = compute_create2_address_word(&mut cx, &mut state, deployer, salt, initcode_hash)
        .expect("symbolic deployer is supported");

    assert_eq!(first, second);
    assert!(first.var_name().is_some_and(|name| name.starts_with("create2_address_")));
}

#[test]
fn recorded_logs_return_data_matches_abi_encoding() {
    let mut cx = SymCx::new();
    let emitter = Address::from([0x33; 20]);
    let topic = B256::from([0x11; 32]);
    let data = vec![cx.constant(U256::from(0x22)), cx.constant(U256::from(0x33))];
    let data = SymBytes::exprs(&mut cx, data);
    let log = SymbolicLog::new(
        vec![cx.constant(U256::from_be_bytes(topic.0))],
        cx.constant(U256::from(2)),
        data,
        emitter,
    );

    let encoded = recorded_logs_return_data(&mut cx, vec![log])
        .read_concrete(&mut cx, "recorded log return data")
        .unwrap();
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
    let mut cx = SymCx::new();
    let emitter = Address::from([0x33; 20]);
    let data = vec![cx.constant(U256::from(0x12)), cx.var("byte")];
    let data = SymBytes::exprs(&mut cx, data);
    let log = SymbolicLog::new(vec![cx.var("topic")], cx.constant(U256::from(2)), data, emitter);

    let return_data = recorded_logs_json_return_data(&mut cx, vec![log]).unwrap();
    let encoded = (0..return_data.len()).map(|idx| return_data.byte(&mut cx, idx)).collect();
    let encoded = SymBytes::exprs(&mut cx, encoded)
        .eval_model(
            &mut cx,
            &BTreeMap::from([
                ("topic".to_string(), U256::from(0xabcd)),
                ("byte".to_string(), U256::from(0xef)),
            ]),
        )
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
    let mut cx = SymCx::new();
    let bytes = vec![
        cx.constant(U256::from(0x22)),
        cx.constant(U256::from(0x33)),
        cx.constant(U256::from(0x44)),
    ];
    let bytes = SymBytes::exprs(&mut cx, bytes);
    let len = cx.var("len");
    let encoded = encode_packed_bytes_with_len(&mut cx, len, &bytes);

    assert_eq!(
        encoded
            .word_at(&mut cx, 0)
            .eval_model(&BTreeMap::from([("len".to_string(), U256::from(2))]))
            .unwrap(),
        U256::from(2)
    );
}

#[test]
fn symbolic_world_resolves_symbolic_create2_address_aliases() {
    let mut cx = SymCx::new();
    let mut world = SymbolicWorld::default();
    let word = cx.var("create2_address");
    let address = world.symbolic_address_slot(word.clone());
    let address_mask = cx.constant((U256::from(1) << 160) - U256::from(1));
    let masked = cx.op(SymExprOp::And, word.clone(), address_mask);

    assert_eq!(world.resolve_address(&word), Some(address));
    assert_eq!(world.resolve_address(&masked), Some(address));
    assert_eq!(world.symbolic_address_slot(word), address);
    assert_ne!(address, Address::ZERO);
}

#[test]
fn symbolic_create2_accepts_symbolic_salt() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let salt = cx.var("salt");
    let initcode = SymCode::concrete(&mut cx, vec![opcode::STOP]);

    let (word, address) =
        create2_address_word(&mut cx, &mut state, creator, salt, &initcode).unwrap();

    assert!(word.as_const().is_none());
    assert_eq!(state.world.resolve_address(&word), Some(address));
    assert_eq!(state.constraints.len(), 1);
    assert_ne!(address, Address::ZERO);
}

#[test]
fn symbolic_create2_initcode_identity_uses_byte_semantics() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let salt = cx.var("salt");
    let init_word = cx.var("init_word");
    let structural_bytes = init_word.clone().into_bytes(&mut cx);
    let structural = SymCode::from_bytes(&mut cx, structural_bytes);
    let init_bytes = init_word.into_byte_exprs(&mut cx);
    let materialized = SymCode::from_byte_exprs(&mut cx, init_bytes);

    let (structural_word, structural_address) =
        create2_address_word(&mut cx, &mut state, creator, salt.clone(), &structural).unwrap();
    let (materialized_word, materialized_address) =
        create2_address_word(&mut cx, &mut state, creator, salt, &materialized).unwrap();

    assert_eq!(structural_word, materialized_word);
    assert_eq!(structural_address, materialized_address);
}

#[test]
fn symbolic_return_data_can_be_installed_as_runtime_code() {
    let mut cx = SymCx::new();
    let runtime_byte = cx.var("runtime_byte");
    let bytes = SymBytes::exprs(&mut cx, vec![runtime_byte.clone()]);
    let data = SymReturnData::from_bytes(&mut cx, bytes);

    let code = data.to_code(&mut cx).unwrap();

    assert_eq!(code.read_bytes(&mut cx, 0, 1).materialize(&mut cx), vec![runtime_byte]);
}

#[test]
fn symbolic_runtime_size_is_not_installed_as_concrete_code() {
    let mut cx = SymCx::new();
    let stop = cx.constant(U256::from(opcode::STOP));
    let bytes = SymBytes::exprs(&mut cx, vec![stop]);
    let len = cx.var("runtime_len");
    let data = SymReturnData::from_bytes_with_len(bytes, len);

    assert!(matches!(
        data.to_code(&mut cx),
        Err(SymbolicError::Unsupported("CREATE with symbolic runtime size not modeled"))
    ));
}

#[test]
fn symbolic_world_tracks_created_code_and_nonce_overlay() {
    let mut cx = SymCx::new();
    let created = Address::from([0x22; 20]);
    let mut world = SymbolicWorld::default();

    let code = SymCode::concrete(&mut cx, vec![opcode::STOP]);
    world.install_code(created, code.clone());
    world.set_nonce(created, 1);

    assert_eq!(world.cached_code(created), Some(&code));
    assert_eq!(world.cached_nonce(created), Some(1));
}

#[test]
fn symbolic_codecopy_preserves_symbolic_constructor_bytes() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let stop = cx.constant(U256::from(opcode::STOP));
    let constructor_arg = cx.var("constructor_arg_byte");
    let bytes = SymBytes::exprs(&mut cx, vec![stop, constructor_arg]);
    let initcode = SymCode::from_bytes(&mut cx, bytes);

    let init_bytes = initcode.read_bytes(&mut cx, 0, 2);
    memory.store_bytes(&mut cx, 0, init_bytes);

    assert_eq!(memory.byte(&mut cx, 0), cx.constant(U256::from(opcode::STOP)));
    assert_eq!(memory.byte(&mut cx, 1), cx.var("constructor_arg_byte"));
}

#[test]
fn symbolic_codecopy_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let code_bytes = (0u8..40).map(|idx| cx.constant(U256::from(idx + 1))).collect();
    let code_bytes = SymBytes::exprs(&mut cx, code_bytes);
    let code = SymCode::from_bytes(&mut cx, code_bytes);
    let mut memory = SymMemory::default();

    let offset = cx.var("offset");
    let code_bytes = code.read_bytes_offset(&mut cx, offset, 2);
    memory.store_bytes(&mut cx, 0, code_bytes);
    let word = memory.load_word(&mut cx, 0).unwrap();

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
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let stop = cx.constant(U256::from(opcode::STOP));
    let arg = cx.var("arg");
    let bytes = SymBytes::exprs(&mut cx, vec![stop, arg]);
    memory.store_bytes(&mut cx, 7, bytes);
    let offset = cx.var("offset");
    let initcode = SymCode::from_memory_offset(&mut cx, &memory, offset, 2);
    let mut bytes = initcode.read_bytes(&mut cx, 0, 2).materialize(&mut cx);
    bytes.extend((0..30).map(|_| cx.zero()));
    let word = cx.from_bytes(bytes);

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
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let offset = cx.var("offset");

    let seven = cx.constant(U256::from(7));
    let constraint = cx.eq(offset.clone(), seven);
    state.constraints.push(constraint);

    assert_eq!(state.constrained_usize(&mut cx, &offset), Some(7));
}

#[test]
fn path_state_extracts_symbolic_usize_upper_bound() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let size = cx.var("size");

    let five = cx.constant(U256::from(5));
    let constraint = cx.cmp(SymBoolExprOp::Ult, size.clone(), five);
    state.constraints.push(constraint);

    assert_eq!(state.upper_bound_usize(&mut cx, &size), Some(4));
}

#[test]
fn path_state_extracts_constrained_symbolic_usize_from_encoded_bool_word() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let offset = cx.var("offset");
    let offset_expr = offset.clone();
    let mask = cx.constant(U256::from(0xffff));

    let masked_offset = cx.op(SymExprOp::And, offset_expr.clone(), mask.clone());
    let offset_constraint = cx.eq(masked_offset, offset_expr.clone());
    state.constraints.push(offset_constraint);
    let expected_offset = cx.constant(U256::from(0x80));
    let masked_offset = cx.op(SymExprOp::And, mask, offset_expr);
    let condition = cx.eq(expected_offset, masked_offset);
    let one = cx.one();
    let zero = cx.zero();
    let condition_word = cx.ite(condition, one, zero.clone());
    let shifted_condition = cx.op(SymExprOp::Shr, condition_word, zero.clone());
    let byte_mask = cx.constant(U256::from(0xff));
    let bool_byte = cx.op(SymExprOp::And, shifted_condition, byte_mask);
    let bool_word = cx.op(SymExprOp::Or, zero.clone(), bool_byte);
    let bool_word_zero = cx.eq(bool_word, zero);
    state.constraints.push(bool_word_zero.not(&mut cx));

    assert_eq!(state.constrained_usize(&mut cx, &offset), Some(0x80));
}

#[test]
fn path_state_evaluates_compound_constrained_symbolic_word() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let value = cx.var("value");
    let beef = cx.constant(U256::from(0xbeef));
    let value_constraint = cx.eq(value.clone(), beef);
    state.constraints.push(value_constraint);

    let zero = cx.zero();
    let u64_max = cx.constant(U256::from(u64::MAX));
    let masked_value = cx.op(SymExprOp::And, value, u64_max);
    let encoded_word = cx.op(SymExprOp::Or, zero, masked_value);

    assert_eq!(state.constrained_word(&mut cx, &encoded_word), Some(U256::from(0xbeef)));
}

#[test]
fn symbolic_push_data_reconstructs_symbolic_word() {
    let mut cx = SymCx::new();
    let push2 = cx.constant(U256::from(opcode::PUSH2));
    let hi = cx.var("immutable_hi");
    let lo = cx.var("immutable_lo");
    let bytes = SymBytes::exprs(&mut cx, vec![push2, hi, lo]);
    let code = SymCode::from_bytes(&mut cx, bytes);

    let word = code.push_data_word(&mut cx, 1, 2);

    assert!(word.as_const().is_none());
}

#[test]
fn symbolic_push32_preserves_structural_word() {
    let mut cx = SymCx::new();
    let immediate = cx.var("immutable");
    let opcode = SymBytes::concrete(&mut cx, vec![opcode::PUSH32]);
    let immediate_bytes = immediate.clone().into_bytes(&mut cx);
    let bytes = SymBytes::concat(&mut cx, [opcode, immediate_bytes]);
    let code = SymCode::from_bytes(&mut cx, bytes);

    assert_eq!(code.push_data_word(&mut cx, 1, 32), immediate);
}

#[test]
fn abi_bytes_return_encodes_symbolic_bytes() {
    let mut cx = SymCx::new();
    let byte_0 = cx.var("calldata_byte_0");
    let byte_1 = cx.constant(U256::from(0x42));
    let ret = abi_bytes_return(&mut cx, vec![byte_0.clone(), byte_1.clone()]);
    let offset_bytes = (0..32).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let offset_word = cx.from_bytes(offset_bytes);
    let offset = cx.constant(U256::from(32));

    assert_eq!(offset_word, offset);
    let len_bytes = (32..64).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let len_word = cx.from_bytes(len_bytes);
    let len = cx.constant(U256::from(2));
    assert_eq!(len_word, len);
    assert_eq!(ret.byte(&mut cx, 64), byte_0);
    assert_eq!(ret.byte(&mut cx, 65), byte_1);
}

#[test]
fn abi_bytes_return_can_encode_symbolic_length() {
    let mut cx = SymCx::new();
    let len = cx.var("len");
    let byte_0 = cx.var("byte_0");
    let byte_1 = cx.var("byte_1");
    let ret = abi_bytes_return_with_len(&mut cx, len.clone(), vec![byte_0.clone(), byte_1.clone()]);
    let offset_bytes = (0..32).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let offset_word = cx.from_bytes(offset_bytes);
    let offset = cx.constant(U256::from(32));

    assert_eq!(offset_word, offset);
    let len_bytes = (32..64).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let len_word = cx.from_bytes(len_bytes);
    assert_eq!(len_word, len);
    assert_eq!(ret.byte(&mut cx, 64), byte_0);
    assert_eq!(ret.byte(&mut cx, 65), byte_1);
}

#[test]
fn symbolic_keccak_is_deterministic_for_same_symbolic_bytes() {
    let mut cx = SymCx::new();
    let bytes = cx.var("slot_key").into_byte_exprs(&mut cx);

    let first = keccak_word(&mut cx, bytes.clone());
    let second = keccak_word(&mut cx, bytes);

    assert_eq!(first, second);
    assert!(first.is_keccak());
}

#[test]
fn symbolic_keccak_tracks_symbolic_length() {
    let mut cx = SymCx::new();
    let bytes = vec![cx.var("byte_0"), cx.var("byte_1"), cx.zero()];
    let len = cx.var("len");

    let word = keccak_word_with_len(&mut cx, bytes, len);

    let (hash_len, hash_bytes_len) =
        word.keccak_len_and_byte_count().expect("expected symbolic keccak term");
    assert_eq!(hash_len, &cx.var("len"));
    assert_eq!(hash_bytes_len, 3);
}

#[test]
fn sym_expr_eval_computes_symbolic_keccak_from_model() {
    let mut cx = SymCx::new();
    let owner = Address::from([0x11; 20]);
    let slot = U256::from(1);
    let mut bytes = cx.var("owner").into_byte_exprs(&mut cx);
    bytes.extend(cx.constant(slot).into_byte_exprs(&mut cx));
    let word = keccak_word(&mut cx, bytes);

    let mut input = Vec::new();
    input.extend(address_word(owner).to_be_bytes::<32>());
    input.extend(slot.to_be_bytes::<32>());
    let expected = U256::from_be_bytes(keccak256(input).0);
    let model = BTreeMap::from([("owner".to_string(), address_word(owner))]);

    assert_eq!(word.eval_model(&model).unwrap(), expected);
}

#[test]
fn symbolic_hash_precompiles_are_deterministic_for_same_symbolic_input() {
    let mut cx = SymCx::new();
    let input = vec![cx.var("input_0"), cx.var("input_1")];

    let input_len = cx.constant(U256::from(input.len()));
    let input_bytes = SymBytes::exprs(&mut cx, input.clone());
    let sha = execute_symbolic_precompile(
        &mut cx,
        precompile_address(2),
        input_bytes,
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let input_bytes = SymBytes::exprs(&mut cx, input.clone());
    let sha_again = execute_symbolic_precompile(
        &mut cx,
        precompile_address(2),
        input_bytes,
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let sha_bytes = (0..32).map(|idx| sha.byte(&mut cx, idx)).collect::<Vec<_>>();
    let sha_word = cx.from_bytes(sha_bytes);
    let sha_again_bytes = (0..32).map(|idx| sha_again.byte(&mut cx, idx)).collect::<Vec<_>>();
    let sha_again_word = cx.from_bytes(sha_again_bytes);

    assert_eq!(sha.len(), 32);
    assert_eq!(sha_word, sha_again_word);
    assert_eq!(sha_word.hash_algorithm(), Some("sha256"));

    let input_bytes = SymBytes::exprs(&mut cx, input.clone());
    let ecrecover = execute_symbolic_precompile(
        &mut cx,
        precompile_address(1),
        input_bytes,
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let input_bytes = SymBytes::exprs(&mut cx, input.clone());
    let ecrecover_again = execute_symbolic_precompile(
        &mut cx,
        precompile_address(1),
        input_bytes,
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(ecrecover.len(), 32);
    for idx in 0..12 {
        assert_eq!(ecrecover.byte(&mut cx, idx), cx.zero());
    }
    for idx in 0..32 {
        assert_eq!(ecrecover.byte(&mut cx, idx), ecrecover_again.byte(&mut cx, idx));
    }

    let input_bytes = SymBytes::exprs(&mut cx, input.clone());
    let ripemd = execute_symbolic_precompile(
        &mut cx,
        precompile_address(3),
        input_bytes,
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let input_bytes = SymBytes::exprs(&mut cx, input);
    let ripemd_again = execute_symbolic_precompile(
        &mut cx,
        precompile_address(3),
        input_bytes,
        input_len,
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(ripemd.len(), 32);
    for idx in 0..12 {
        assert_eq!(ripemd.byte(&mut cx, idx), cx.zero());
    }
    for idx in 0..32 {
        assert_eq!(ripemd.byte(&mut cx, idx), ripemd_again.byte(&mut cx, idx));
    }
}

#[test]
fn identity_precompile_preserves_symbolic_input_len() {
    let mut cx = SymCx::new();
    let input = vec![
        cx.constant(U256::from(1)),
        cx.constant(U256::from(2)),
        cx.constant(U256::from(3)),
        cx.constant(U256::from(4)),
    ];
    let input_len = cx.var("size");
    let input = SymBytes::exprs(&mut cx, input);
    let return_data = execute_symbolic_precompile(
        &mut cx,
        precompile_address(4),
        input,
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(return_data.len(), 4);
    assert_eq!(return_data.len_word(), input_len);
    assert_eq!(return_data.byte(&mut cx, 0), cx.constant(U256::from(1)));
    assert_eq!(return_data.byte(&mut cx, 3), cx.constant(U256::from(4)));
}

#[test]
fn advanced_precompiles_accept_symbolic_payloads() {
    let mut cx = SymCx::new();
    let mut modexp_input = vec![cx.zero(); 99];
    modexp_input[31] = cx.constant(U256::from(1));
    modexp_input[63] = cx.constant(U256::from(1));
    modexp_input[95] = cx.constant(U256::from(1));
    modexp_input[96] = cx.var("base");
    modexp_input[97] = cx.constant(U256::from(5));
    modexp_input[98] = cx.constant(U256::from(13));

    let modexp_len = cx.constant(U256::from(modexp_input.len()));
    let input = SymBytes::exprs(&mut cx, modexp_input.clone());
    let modexp = execute_symbolic_precompile(
        &mut cx,
        precompile_address(5),
        input,
        modexp_len,
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let modexp_len = cx.constant(U256::from(modexp_input.len()));
    let input = SymBytes::exprs(&mut cx, modexp_input.clone());
    let modexp_again = execute_symbolic_precompile(
        &mut cx,
        precompile_address(5),
        input,
        modexp_len,
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(modexp.len(), 1);
    assert_eq!(modexp.byte(&mut cx, 0), modexp_again.byte(&mut cx, 0));

    let mut blake_input = vec![cx.var("blake_input"); 213];
    blake_input[212] = cx.zero();
    let blake_len = cx.constant(U256::from(213));
    let blake_input = SymBytes::exprs(&mut cx, blake_input);
    let blake = execute_symbolic_precompile(
        &mut cx,
        precompile_address(9),
        blake_input,
        blake_len,
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(blake.len(), 64);
}

#[test]
fn validity_sensitive_symbolic_precompiles_report_incomplete() {
    let mut cx = SymCx::new();
    let bn_input = vec![cx.var("point"); 128];
    let bn_len = cx.constant(U256::from(128));
    let bn_input = SymBytes::exprs(&mut cx, bn_input);
    let err = execute_symbolic_precompile(
        &mut cx,
        precompile_address(6),
        bn_input,
        bn_len,
        SpecId::CANCUN,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SymbolicError::Unsupported("symbolic bn254 precompile validity not modeled")
    ));

    let blake_input = vec![cx.var("blake_input"); 213];
    let blake_len = cx.constant(U256::from(213));
    let blake_input = SymBytes::exprs(&mut cx, blake_input);
    let err = execute_symbolic_precompile(
        &mut cx,
        precompile_address(9),
        blake_input,
        blake_len,
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
    let mut cx = SymCx::new();
    let address = Address::from([0x11; 20]);
    let key = cx.var("slot");
    let value = cx.var("value");
    let writes = vec![StorageWrite::new(address, key.clone(), value.clone())];
    let base = cx.zero();

    assert_eq!(StorageWrite::select_from(&mut cx, &writes, address, key, base), value);
}

#[test]
fn symbolic_storage_uses_conditional_value_for_maybe_equal_key() {
    let mut cx = SymCx::new();
    let address = Address::from([0x11; 20]);
    let write_key = cx.var("write_slot");
    let read_key = cx.var("read_slot");
    let value = cx.var("value");
    let writes = vec![StorageWrite::new(address, write_key.clone(), value.clone())];
    let base = cx.zero();
    let selected = StorageWrite::select_from(&mut cx, &writes, address, read_key.clone(), base);
    let keys_match = cx.eq(read_key, write_key);
    let zero = cx.zero();
    let expected = cx.ite(keys_match, value, zero);

    assert_eq!(selected, expected);
}

#[test]
fn symbolic_storage_key_equality_decomposes_keccak_offsets() {
    let mut cx = SymCx::new();
    let owner = cx.var("owner");
    let owner_bytes = owner.into_byte_exprs(&mut cx);
    let base = keccak_word(&mut cx, owner_bytes);
    let left_index = cx.var("left_index");
    let right_index = cx.var("right_index");
    let left = add_words(&mut cx, base.clone(), left_index);
    let right = add_words(&mut cx, base, right_index);

    let actual = left.storage_key_eq(&mut cx, &right);
    let left_index = cx.var("left_index");
    let right_index = cx.var("right_index");
    let expected = cx.eq(left_index, right_index);
    assert_eq!(actual, expected);
}

#[test]
fn symbolic_storage_key_equality_expands_distinct_keccak_bases() {
    let mut cx = SymCx::new();
    let left_owner = cx.var("left_owner");
    let left_base = keccak_word(&mut cx, vec![left_owner]);
    let right_owner = cx.var("right_owner");
    let right_base = keccak_word(&mut cx, vec![right_owner]);
    let index = cx.var("index");

    let left = add_words(&mut cx, left_base, index.clone());
    let right = add_words(&mut cx, right_base, index);
    let condition = left.storage_key_eq(&mut cx, &right);

    let left_owner = cx.var("left_owner");
    let right_owner = cx.var("right_owner");
    let expected = cx.eq(left_owner, right_owner);
    assert_eq!(condition, expected);
}

#[test]
fn symbolic_storage_key_equality_rejects_concrete_plain_slot_alias() {
    let mut cx = SymCx::new();
    let owner = cx.var("owner");
    let owner = owner.into_byte_exprs(&mut cx);
    let owner = keccak_word(&mut cx, owner);
    let zero = cx.zero();
    let layout_key = add_words(&mut cx, owner, zero);
    let plain_slot = cx.zero();
    let actual = layout_key.storage_key_eq(&mut cx, &plain_slot);
    let expected = cx.bool_constant(false);

    assert_eq!(actual, expected);
}

#[test]
fn nested_mapping_key_does_not_alias_plain_mapping_key_under_model() {
    let mut cx = SymCx::new();
    let owner = cx.var("owner");
    let spender = cx.var("spender");
    let recipient = cx.var("recipient");

    let mut balance_key_bytes = recipient.into_byte_exprs(&mut cx);
    balance_key_bytes.extend(cx.constant(U256::ZERO).into_byte_exprs(&mut cx));
    let balance_key = keccak_word(&mut cx, balance_key_bytes);

    let mut inner_key_bytes = owner.into_byte_exprs(&mut cx);
    inner_key_bytes.extend(cx.constant(U256::from(1)).into_byte_exprs(&mut cx));
    let inner_key = keccak_word(&mut cx, inner_key_bytes);

    let mut allowance_key_bytes = spender.into_byte_exprs(&mut cx);
    allowance_key_bytes.extend(inner_key.into_byte_exprs(&mut cx));
    let allowance_key = keccak_word(&mut cx, allowance_key_bytes);

    let same_address = precompile_address(0x60);
    let model = BTreeMap::from([
        ("owner".to_string(), address_word(Address::from([0x11; 20]))),
        ("spender".to_string(), address_word(same_address)),
        ("recipient".to_string(), address_word(same_address)),
    ]);
    let condition = balance_key.storage_key_eq(&mut cx, &allowance_key);

    assert_eq!(condition, cx.bool_constant(false));
    assert!(!condition.eval_model(&model).unwrap());
}

#[test]
fn symbolic_world_snapshot_restores_overlay_state() {
    let mut cx = SymCx::new();
    let address = Address::from([0x11; 20]);
    let mut world = SymbolicWorld::default();
    world.sstore(address, cx.constant(U256::from(1)), cx.constant(U256::from(2)));

    let snapshot = world.snapshot_state();
    world.sstore(address, cx.constant(U256::from(1)), cx.constant(U256::from(3)));

    assert!(world.restore_snapshot(snapshot));
    assert_eq!(world.storage_len(), 1);
    assert_eq!(world.storage_value(0), Some(&cx.constant(U256::from(2))));
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

    let mut cx = SymCx::new();
    let err = symbolic_calldata_variants(&mut cx, &function, &config).unwrap_err();

    assert!(err.to_string().contains("symbolic.array_lengths has 2 entries"));
}

#[test]
fn positional_dynamic_lengths_allow_shorter_expanded_variants() {
    let mut cx = SymCx::new();
    let function = Function::parse("check(bytes[])").unwrap();
    let config = SymbolicConfig {
        default_array_lengths: vec![1, 2],
        array_lengths: vec![4, 4],
        ..Default::default()
    };

    let variants = symbolic_calldata_variants(&mut cx, &function, &config).unwrap();

    assert_eq!(variants.len(), 2);
    let element_counts = variants
        .iter()
        .map(|calldata| {
            calldata
                .call_data(&mut cx)
                .load(&mut cx, 36)
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

    let mut cx = SymCx::new();
    let err = symbolic_calldata_variants(&mut cx, &function, &config).unwrap_err();

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

    let mut cx = SymCx::new();
    let err = symbolic_calldata_variants(&mut cx, &function, &config).unwrap_err();

    assert!(matches!(err, SymbolicError::CalldataVariantLimit(2)));
}

#[test]
fn symbolic_signextend_uses_sign_bit_ite() {
    let mut cx = SymCx::new();
    let word = cx.var("word");
    let actual = signextend_word(&mut cx, U256::ZERO, word.clone());
    let sign_mask = cx.constant(U256::from(0x80));
    let sign_bit = cx.op(SymExprOp::And, word.clone(), sign_mask);
    let zero = cx.zero();
    let sign_bit_zero = cx.eq(sign_bit, zero);
    let positive_mask = cx.constant(U256::from(0x7f));
    let positive = cx.op(SymExprOp::And, word.clone(), positive_mask);
    let negative_mask = cx.constant(!U256::from(0x7f));
    let negative = cx.op(SymExprOp::Or, word, negative_mask);
    let expected = cx.ite(sign_bit_zero, positive, negative);
    assert_eq!(actual, expected);
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
    let mut cx = SymCx::new();
    let output = "\
sat
(
  (define-fun calldata_0 () (_ BitVec 8)
    #x00)
)
";
    let calldata = cx.var("calldata_0");
    let one = cx.constant(U256::from(1));
    let constraints = vec![cx.eq(calldata, one)];

    let err = validate_solver_model_output(output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
fn fallback_model_finds_wrapping_arithmetic_riddle_candidate() {
    let mut cx = SymCx::new();
    let var = cx.var("calldata_0");
    let msg_sender = U256::from_str_radix("1804c8ab1f12e6bbf3894d4083f33e07309d1f38", 16).unwrap();
    let product = cx.op(SymExprOp::Mul, var.clone(), var.clone());
    let sender_word = cx.constant(msg_sender);
    let product_below_sender = cx.cmp(SymBoolExprOp::Ult, product, sender_word.clone());
    let above_sender = cx.cmp(SymBoolExprOp::Ugt, var.clone(), sender_word);
    let mask_800 = cx.constant(U256::from(0x800));
    let masked_800 = cx.op(SymExprOp::And, var.clone(), mask_800);
    let zero = cx.zero();
    let has_800 = cx.eq(masked_800, zero.clone()).not(&mut cx);
    let mask_10000 = cx.constant(U256::from(0x10000));
    let masked_10000 = cx.op(SymExprOp::And, var, mask_10000);
    let lacks_10000 = cx.eq(masked_10000, zero);
    let constraints = vec![product_below_sender, above_sender, has_800, lacks_10000];

    let model = fallback_single_var_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn expression_op_simplifies_exact_arithmetic_identities() {
    let mut cx = SymCx::new();
    let x = cx.var("x");

    let zero = cx.zero();
    let actual = cx.op(SymExprOp::Mul, x.clone(), zero.clone());
    assert_eq!(actual, zero);
    let one = cx.one();
    let actual = cx.op(SymExprOp::Mul, one, x.clone());
    assert_eq!(actual, x);
    let x = cx.var("x");
    let one = cx.one();
    let actual = cx.op(SymExprOp::UDiv, x.clone(), one);
    assert_eq!(actual, x);
    let x = cx.var("x");
    let one = cx.one();
    let actual = cx.op(SymExprOp::URem, x, one);
    let zero = cx.zero();
    assert_eq!(actual, zero);
    let x = cx.var("x");
    let x_again = cx.var("x");
    let actual = cx.op(SymExprOp::Sub, x, x_again);
    let zero = cx.zero();
    assert_eq!(actual, zero);
    let x = cx.var("x");
    let max = cx.constant(U256::MAX);
    let actual = cx.op(SymExprOp::And, x.clone(), max);
    assert_eq!(actual, x);
    let x = cx.var("x");
    let actual = cx.op(SymExprOp::And, x.clone(), x.clone());
    assert_eq!(actual, x);
    let mask = cx.constant((U256::from(1) << 160) - U256::from(1));
    let x = cx.var("x");
    let masked = cx.op(SymExprOp::And, x, mask.clone());
    let actual = cx.op(SymExprOp::And, masked.clone(), mask);
    assert_eq!(actual, masked);
    let six = cx.constant(U256::from(6));
    let seven = cx.constant(U256::from(7));
    let actual = cx.op(SymExprOp::Mul, six, seven);
    let forty_two = cx.constant(U256::from(42));
    assert_eq!(actual, forty_two);
}

#[test]
fn bool_word_equality_simplifies_to_condition() {
    let mut cx = SymCx::new();
    let x = cx.var("x");
    let y = cx.var("y");
    let condition = cx.cmp(SymBoolExprOp::Ult, x, y);
    let word = cx.bool_word(condition.clone());

    let one = cx.one();
    assert_eq!(cx.eq(word.clone(), one), condition);
    let one = cx.one();
    assert_eq!(cx.eq(one, word.clone()), condition);
    let expected = condition.not(&mut cx);
    let zero = cx.zero();
    assert_eq!(cx.eq(word.clone(), zero), expected);
    let two = cx.constant(U256::from(2));
    let actual = cx.eq(word, two);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
}

#[test]
fn inverted_bool_word_equality_simplifies_to_condition() {
    let mut cx = SymCx::new();
    let x = cx.var("x");
    let y = cx.var("y");
    let condition = cx.cmp(SymBoolExprOp::Ult, x, y);
    let zero = cx.zero();
    let one = cx.one();
    let word = cx.ite(condition.clone(), zero, one);

    let one = cx.one();
    let actual = cx.eq(word.clone(), one);
    let expected = condition.clone().not(&mut cx);
    assert_eq!(actual, expected);
    let zero = cx.zero();
    assert_eq!(cx.eq(word, zero), condition);
}

#[test]
fn bool_comparison_folds_reflexive_operands() {
    let mut cx = SymCx::new();

    let x = cx.var("x");
    let actual = cx.cmp(SymBoolExprOp::Ult, x.clone(), x.clone());
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let actual = cx.cmp(SymBoolExprOp::Ugt, x.clone(), x.clone());
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let actual = cx.cmp(SymBoolExprOp::Ule, x.clone(), x.clone());
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
    let actual = cx.cmp(SymBoolExprOp::Uge, x.clone(), x.clone());
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
    let actual = cx.cmp(SymBoolExprOp::Slt, x.clone(), x.clone());
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let actual = cx.cmp(SymBoolExprOp::Sgt, x.clone(), x);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
}

#[test]
fn bool_comparison_folds_unsigned_boundaries() {
    let mut cx = SymCx::new();

    let zero = cx.zero();
    let x = cx.var("x");
    let actual = cx.cmp(SymBoolExprOp::Ugt, zero, x);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let zero = cx.zero();
    let x = cx.var("x");
    let actual = cx.cmp(SymBoolExprOp::Ule, zero, x);
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
    let x = cx.var("x");
    let zero = cx.zero();
    let actual = cx.cmp(SymBoolExprOp::Ult, x, zero);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let x = cx.var("x");
    let zero = cx.zero();
    let actual = cx.cmp(SymBoolExprOp::Uge, x, zero);
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
    let max = cx.constant(U256::MAX);
    let x = cx.var("x");
    let actual = cx.cmp(SymBoolExprOp::Ult, max, x);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let max = cx.constant(U256::MAX);
    let x = cx.var("x");
    let actual = cx.cmp(SymBoolExprOp::Uge, max, x);
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
    let x = cx.var("x");
    let max = cx.constant(U256::MAX);
    let actual = cx.cmp(SymBoolExprOp::Ugt, x, max);
    let expected = cx.bool_constant(false);
    assert_eq!(actual, expected);
    let x = cx.var("x");
    let max = cx.constant(U256::MAX);
    let actual = cx.cmp(SymBoolExprOp::Ule, x, max);
    let expected = cx.bool_constant(true);
    assert_eq!(actual, expected);
}

#[test]
fn exact_modular_arithmetic_handles_zero_modulus_and_wide_intermediates() {
    let mut cx = SymCx::new();
    assert_eq!(U256::MAX.add_mod(U256::from(2), U256::ZERO), U256::ZERO);
    assert_eq!(U256::MAX.mul_mod(U256::MAX, U256::ZERO), U256::ZERO);

    assert_eq!(U256::MAX.add_mod(U256::from(2), U256::MAX), U256::from(2));
    assert_eq!(U256::MAX.mul_mod(U256::MAX, U256::MAX), U256::ZERO);

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    let a = cx.var("a");
    let two = cx.constant(U256::from(2));
    let max = cx.constant(U256::MAX);
    let addmod = cx.addmod(a, two, max);
    assert_eq!(addmod.eval_model(&model).unwrap(), U256::from(2));
    let a = cx.var("a");
    let a_again = cx.var("a");
    let max = cx.constant(U256::MAX);
    let mulmod = cx.mulmod(a, a_again, max);
    assert_eq!(mulmod.eval_model(&model).unwrap(), U256::ZERO);
}

#[test]
fn exact_modular_arithmetic_smt_widens_before_modulo() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let two = cx.constant(U256::from(2));
    let m = cx.var("m");
    let addmod = cx.addmod(a, two, m).smt();
    let a = cx.var("a");
    let b = cx.var("b");
    let m = cx.var("m");
    let mulmod = cx.mulmod(a, b, m).smt();

    assert!(addmod.contains("((_ zero_extend 256) a)"));
    assert!(addmod.contains("bvadd"));
    assert!(addmod.contains("bvurem"));
    assert!(mulmod.contains("((_ zero_extend 256) b)"));
    assert!(mulmod.contains("bvmul"));
    assert!(mulmod.contains("((_ extract 255 0)"));
}

#[test]
fn exact_addmod_model_validation_rejects_wrapping_false_pass() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let two = cx.constant(U256::from(2));
    let max = cx.constant(U256::MAX);
    let addmod = cx.addmod(a, two, max);
    let one = cx.constant(U256::from(1));
    let constraints = vec![cx.eq(addmod, one)];
    let output = format!("sat\n((define-fun a () (_ BitVec 256) #x{}))\n", "f".repeat(64));

    let err = validate_solver_model_output(&output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
fn solver_normalizes_udiv_zero_predicates_without_bvudiv() {
    let mut cx = SymCx::new();
    let numerator = cx.var("numerator");
    let denominator = cx.var("denominator");
    let div = cx.op(SymExprOp::UDiv, numerator, denominator);
    let zero = cx.zero();
    let original = cx.eq(div, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

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
    let mut cx = SymCx::new();
    let numerator = cx.var("numerator");
    let denominator = cx.var("denominator");
    let div = cx.op(SymExprOp::UDiv, numerator, denominator);
    let zero = cx.zero();
    let original = cx.cmp(SymBoolExprOp::Ugt, div, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

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
    let mut cx = SymCx::new();
    let x = cx.var("x");
    let y = cx.var("y");
    let ten = cx.constant(U256::from(10));
    let a = cx.cmp(SymBoolExprOp::Ult, x.clone(), ten);
    let three = cx.constant(U256::from(3));
    let b = cx.eq(y.clone(), three);
    let grouped = vec![
        {
            let value = cx.bool_constant(true);
            SymBoolExpr::raw_and(&mut cx, vec![b.clone(), value, a.clone()])
        },
        a.clone(),
        SymBoolExpr::raw_and(&mut cx, vec![b.clone()]),
    ];

    let normalized = normalize_constraints_for_solver(&mut cx, &grouped);

    assert_eq!(normalized, vec![b, a]);

    let x_eq_y = cx.eq(x, y);
    let false_expr = cx.bool_constant(false);
    let true_expr = cx.bool_constant(true);
    let unsat = normalize_constraints_for_solver(&mut cx, &[x_eq_y, false_expr, true_expr]);
    assert_eq!(unsat, vec![cx.bool_constant(false)]);
}

#[test]
fn solver_normalizes_erc4626_style_share_zero_predicate() {
    let mut cx = SymCx::new();
    let assets = cx.var("assets");
    let supply = cx.var("supply");
    let total_assets = cx.var("total_assets");
    let numerator = cx.op(SymExprOp::Mul, assets, supply);
    let shares = cx.op(SymExprOp::UDiv, numerator, total_assets);
    let zero = cx.zero();
    let constraints = vec![cx.eq(shares, zero)];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    assert_eq!(normalized.len(), 1);
    assert!(!normalized[0].smt().contains("bvudiv"));
    assert!(normalized[0].smt().contains("bvmul"));
}

#[test]
fn solver_rebuilds_word_from_extracted_byte_terms() {
    let mut cx = SymCx::new();
    let word = cx.var("word");
    let mask = cx.constant(U256::from(u64::MAX));
    let masked = cx.op(SymExprOp::And, word, mask);
    let bytes = masked.clone().into_byte_exprs(&mut cx);
    let rebuilt_word = cx.from_bytes(bytes);
    let rebuilt = normalize_expr_for_solver(&mut cx, rebuilt_word);

    assert_eq!(rebuilt, normalize_expr_for_solver(&mut cx, masked));
}

#[test]
fn solver_rebuilds_word_from_shifted_fragments() {
    let mut cx = SymCx::new();
    let word = cx.var("word");
    let low_mask = cx.constant(mask_bits(U256::MAX, 32));
    let low_bits = cx.op(SymExprOp::And, word.clone(), low_mask);
    let shift = cx.constant(U256::from(32));
    let shifted = cx.op(SymExprOp::Shr, word.clone(), shift.clone());
    let high_mask = cx.constant(mask_bits(U256::MAX, 224));
    let masked_high = cx.op(SymExprOp::And, shifted, high_mask);
    let high_bits = cx.op(SymExprOp::Shl, masked_high, shift);
    let rebuilt_expr = cx.op(SymExprOp::Or, low_bits, high_bits);
    let rebuilt = normalize_expr_for_solver(&mut cx, rebuilt_expr);

    assert_eq!(rebuilt, word);
}

#[test]
fn solver_simplifies_selector_prefixed_word_fragment() {
    let mut cx = SymCx::new();
    let x = cx.var("calldata_0");
    let y = cx.var("calldata_1");
    let low_32 = cx.constant(mask_bits(U256::MAX, 32));
    let low_224 = cx.constant(mask_bits(U256::MAX, 224));
    let selector = cx.constant(U256::from(0x771602f7u64) << 224);

    let shift_32 = cx.constant(U256::from(32));
    let shifted_x = cx.op(SymExprOp::Shr, x.clone(), shift_32.clone());
    let x_high = cx.op(SymExprOp::And, shifted_x, low_224.clone());
    let first_word = cx.op(SymExprOp::Or, selector, x_high);
    let shifted_y = cx.op(SymExprOp::Shr, y.clone(), shift_32.clone());
    let y_high = cx.op(SymExprOp::And, shifted_y, low_224.clone());
    let x_low = cx.op(SymExprOp::And, x.clone(), low_32.clone());
    let shift_224 = cx.constant(U256::from(224));
    let x_low_shifted = cx.op(SymExprOp::Shl, x_low, shift_224.clone());
    let second_word = cx.op(SymExprOp::Or, y_high, x_low_shifted);
    let second_word_shifted = cx.op(SymExprOp::Shr, second_word.clone(), shift_224);
    let rebuilt_low = cx.op(SymExprOp::And, second_word_shifted, low_32);
    let first_word_masked = cx.op(SymExprOp::And, first_word, low_224);
    let rebuilt_high = cx.op(SymExprOp::Shl, first_word_masked, shift_32.clone());
    let rebuilt = cx.op(SymExprOp::Or, rebuilt_low, rebuilt_high);

    assert_eq!(normalize_expr_for_solver(&mut cx, rebuilt), x);

    let next_low_mask = cx.constant(mask_bits(U256::MAX, 32));
    let next_low = cx.op(SymExprOp::And, y.clone(), next_low_mask);
    let next_high_mask = cx.constant(mask_bits(U256::MAX, 224));
    let next_high = cx.op(SymExprOp::And, second_word, next_high_mask);
    let next_high_shifted = cx.op(SymExprOp::Shl, next_high, shift_32);
    let rebuilt_next = cx.op(SymExprOp::Or, next_low, next_high_shifted);

    assert_eq!(normalize_expr_for_solver(&mut cx, rebuilt_next), y);
}

fn checked_mul_guard_word(cx: &mut SymCx, zero_operand: &SymExpr, expected: &SymExpr) -> SymExpr {
    let zero = cx.zero();
    let operand_is_zero = cx.eq(zero_operand.clone(), zero.clone());
    let product = cx.op(SymExprOp::Mul, zero_operand.clone(), expected.clone());
    let quotient = cx.op(SymExprOp::UDiv, product, zero_operand.clone());
    let checked_product = cx.ite(operand_is_zero.clone(), zero, quotient);
    let operand_is_zero_word = cx.bool_word(operand_is_zero);
    let product_matches_expected = cx.eq(checked_product, expected.clone());
    let product_matches_expected_word = cx.bool_word(product_matches_expected);

    cx.op(SymExprOp::Or, operand_is_zero_word, product_matches_expected_word)
}

#[test]
fn solver_normalizes_checked_mul_guard_for_bounded_operands() {
    let mut cx = SymCx::new();
    let a_word = cx.var("a");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let a = cx.op(SymExprOp::And, a_word, u64_max);
    let b_word = cx.var("b");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let b = cx.op(SymExprOp::And, b_word, u64_max);
    let guard = checked_mul_guard_word(&mut cx, &a, &b);
    let zero = cx.zero();
    let guard_is_zero = cx.eq(guard, zero);

    assert_eq!(normalize_bool_for_solver(&mut cx, guard_is_zero), cx.bool_constant(false));
}

#[test]
fn solver_normalizes_checked_mul_guard_from_path_upper_bound() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let factor = cx.constant(U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&mut cx, &a, &factor);
    let zero = cx.zero();
    let guard_is_false = cx.eq(guard, zero);
    let thousand = cx.constant(U256::from(1000));
    let upper_bound = cx.cmp(SymBoolExprOp::Ule, a, thousand);
    let constraints = vec![upper_bound, guard_is_false.clone()];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    assert_eq!(normalized, vec![cx.bool_constant(false)]);

    for value in [U256::ZERO, U256::from(1), U256::from(1000)] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert!(!guard_is_false.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_guard() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let zero = cx.zero();
    let a_is_zero = cx.eq(a.clone(), zero.clone());
    let quotient = cx.op(SymExprOp::UDiv, a.clone(), a);
    let checked_quotient = cx.ite(a_is_zero.clone(), zero.clone(), quotient);
    let a_is_zero_word = cx.bool_word(a_is_zero);
    let one = cx.one();
    let quotient_is_one = cx.eq(checked_quotient, one);
    let quotient_is_one_word = cx.bool_word(quotient_is_one);
    let guard = cx.op(SymExprOp::Or, a_is_zero_word, quotient_is_one_word);
    let original = cx.eq(guard, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert_eq!(normalized, cx.bool_constant(false));

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert!(!original.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_word() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let zero = cx.zero();
    let a_is_zero = cx.eq(a.clone(), zero.clone());
    let quotient = cx.op(SymExprOp::UDiv, a.clone(), a);
    let checked_quotient = cx.ite(a_is_zero.clone(), zero, quotient);
    let normalized = normalize_expr_for_solver(&mut cx, checked_quotient.clone());

    assert!(!normalized.smt().contains("bvudiv"));
    let a_is_nonzero = a_is_zero.not(&mut cx);
    let one = cx.one();
    let zero = cx.zero();
    let expected = cx.ite(a_is_nonzero, one, zero);
    assert_eq!(normalized, expected);

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
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let zero = cx.zero();
    let a_is_zero = cx.eq(a.clone(), zero.clone());
    let quotient = cx.op(SymExprOp::UDiv, a.clone(), a);
    let mirrored = cx.ite(a_is_zero, quotient, zero);
    let normalized = normalize_expr_for_solver(&mut cx, mirrored.clone());

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert_eq!(mirrored.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_add_overflow_guard() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let zero = cx.zero();
    let a_is_zero = cx.eq(a.clone(), zero.clone());
    let quotient = cx.op(SymExprOp::UDiv, a.clone(), a);
    let checked_quotient = cx.ite(a_is_zero, zero, quotient);
    let one = cx.one();
    let sum = cx.op(SymExprOp::Add, one, checked_quotient.clone());
    let condition = cx.cmp(SymBoolExprOp::Ugt, checked_quotient, sum);

    assert_eq!(normalize_bool_for_solver(&mut cx, condition), cx.bool_constant(false));
}

#[test]
fn solver_normalizes_checked_mul_guard_with_context_bound() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let scale = cx.constant(U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&mut cx, &a, &scale);
    let zero = cx.zero();
    let a_positive = cx.cmp(SymBoolExprOp::Ugt, a.clone(), zero.clone());
    let thousand = cx.constant(U256::from(1000));
    let above_thousand = cx.cmp(SymBoolExprOp::Ugt, a, thousand).not(&mut cx);
    let guard_is_zero = cx.eq(guard, zero);
    let constraints = vec![a_positive, above_thousand, guard_is_zero];

    assert_eq!(
        normalize_constraints_for_solver(&mut cx, &constraints),
        vec![cx.bool_constant(false)]
    );
}

#[test]
fn solver_does_not_context_normalize_checked_mul_guard_without_tight_bound() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let scale = cx.constant(U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&mut cx, &a, &scale);
    let zero = cx.zero();
    let guard_is_zero = cx.eq(guard, zero);

    assert_ne!(
        normalize_constraints_for_solver(&mut cx, std::slice::from_ref(&guard_is_zero)),
        vec![cx.bool_constant(false)]
    );
    let max = cx.constant(U256::MAX);
    let bound = cx.cmp(SymBoolExprOp::Ule, a, max);
    assert_ne!(
        normalize_constraints_for_solver(&mut cx, &[bound, guard_is_zero.clone()]),
        vec![cx.bool_constant(false)]
    );

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert!(guard_is_zero.eval_model(&model).unwrap());
}

#[test]
fn solver_does_not_normalize_unbounded_checked_mul_guard_to_tautology() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let b = cx.var("b");
    let guard = checked_mul_guard_word(&mut cx, &a, &b);
    let zero = cx.zero();
    let original = cx.eq(guard, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert_ne!(normalized, cx.bool_constant(false));

    let model = BTreeMap::from([("a".to_string(), U256::MAX), ("b".to_string(), U256::from(2))]);
    assert!(original.eval_model(&model).unwrap());
    assert_eq!(original.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
}

#[test]
fn solver_does_not_normalize_checked_mul_guard_when_path_bound_can_overflow() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let factor = cx.constant(U256::from(2));
    let guard = checked_mul_guard_word(&mut cx, &a, &factor);
    let zero = cx.zero();
    let original = cx.eq(guard, zero);
    let max = cx.constant(U256::MAX);
    let bound = cx.cmp(SymBoolExprOp::Ule, a, max);
    let normalized = normalize_constraints_for_solver(&mut cx, &[bound, original.clone()]);

    assert!(!matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)));

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert!(original.eval_model(&model).unwrap());
    assert!(normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn solver_normalizes_checked_add_overflow_guard_for_bounded_operands() {
    let mut cx = SymCx::new();
    let a_raw = cx.var("a");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let a = cx.op(SymExprOp::And, a_raw, u64_max);
    let b_raw = cx.var("b");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let b_masked = cx.op(SymExprOp::And, b_raw, u64_max);
    let numerator = cx.op(SymExprOp::Mul, b_masked, a.clone());
    let denominator_raw = cx.var("denominator");
    let high_mask = cx.constant(U256::MAX >> 64);
    let denominator = cx.op(SymExprOp::And, denominator_raw, high_mask);
    let b = cx.op(SymExprOp::UDiv, numerator, denominator);
    let sum = cx.op(SymExprOp::Add, a.clone(), b);
    let condition = cx.cmp(SymBoolExprOp::Ugt, a, sum);

    assert_eq!(normalize_bool_for_solver(&mut cx, condition), cx.bool_constant(false));
}

#[test]
fn solver_does_not_normalize_unbounded_checked_add_overflow_guard() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let b = cx.var("b");
    let sum = cx.op(SymExprOp::Add, a.clone(), b);
    let original = cx.cmp(SymBoolExprOp::Ugt, a, sum);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert_ne!(normalized, cx.bool_constant(false));

    let model = BTreeMap::from([("a".to_string(), U256::MAX), ("b".to_string(), U256::from(1))]);
    assert!(original.eval_model(&model).unwrap());
    assert_eq!(original.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
}

#[test]
fn solver_detects_monotonic_product_contradiction() {
    let mut cx = SymCx::new();
    let ink_word = cx.var("ink");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let ink = cx.op(SymExprOp::And, ink_word, u64_max);
    let art_word = cx.var("art");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let art = cx.op(SymExprOp::And, art_word, u64_max);
    let spot_word = cx.var("spot");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let spot = cx.op(SymExprOp::And, spot_word, u64_max);
    let rate_word = cx.var("rate");
    let u64_max = cx.constant(U256::from(u64::MAX));
    let rate = cx.op(SymExprOp::And, rate_word, u64_max);
    let zero = cx.zero();
    let ink_positive = cx.cmp(SymBoolExprOp::Ugt, ink.clone(), zero.clone());
    let art_above_ink = cx.cmp(SymBoolExprOp::Ugt, art.clone(), ink.clone());
    let spot_positive = cx.cmp(SymBoolExprOp::Ugt, spot.clone(), zero);
    let rate_above_spot = cx.cmp(SymBoolExprOp::Ugt, rate.clone(), spot.clone());
    let left_product = cx.op(SymExprOp::Mul, ink, spot);
    let right_product = cx.op(SymExprOp::Mul, art, rate);
    let wrapping = cx.cmp(SymBoolExprOp::Ult, left_product, right_product).not(&mut cx);
    let constraints = vec![ink_positive, art_above_ink, spot_positive, rate_above_spot, wrapping];

    assert!(product_monotonic_unsat(&mut cx, &constraints));
}

#[test]
fn solver_does_not_prune_wrapping_product_inequality() {
    let mut cx = SymCx::new();
    let ink = cx.var("ink");
    let art = cx.var("art");
    let spot = cx.var("spot");
    let rate = cx.var("rate");
    let zero = cx.zero();
    let ink_positive = cx.cmp(SymBoolExprOp::Ugt, ink.clone(), zero.clone());
    let art_above_ink = cx.cmp(SymBoolExprOp::Ugt, art.clone(), ink.clone());
    let spot_positive = cx.cmp(SymBoolExprOp::Ugt, spot.clone(), zero);
    let rate_above_spot = cx.cmp(SymBoolExprOp::Ugt, rate.clone(), spot.clone());
    let left_product = cx.op(SymExprOp::Mul, ink, spot);
    let right_product = cx.op(SymExprOp::Mul, art, rate);
    let wrapping = cx.cmp(SymBoolExprOp::Ult, left_product, right_product).not(&mut cx);
    let constraints = vec![ink_positive, art_above_ink, spot_positive, rate_above_spot, wrapping];

    assert!(!product_monotonic_unsat(&mut cx, &constraints));

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
    let mut cx = SymCx::new();
    let x = cx.var("x");
    let y = cx.var("y");

    assert!(cx.op(SymExprOp::Mul, x.clone(), y.clone()).contains_hard_arith());
    assert!(cx.op(SymExprOp::UDiv, x.clone(), y.clone()).contains_hard_arith());
    assert!(cx.op(SymExprOp::URem, x.clone(), y).contains_hard_arith());
    let one = cx.constant(U256::from(1));
    let mul = cx.op(SymExprOp::Mul, x, one);
    assert!(!mul.contains_hard_arith());
}

#[test]
fn hard_arithmetic_fallback_finds_multi_variable_candidate() {
    let mut cx = SymCx::new();
    let first = cx.var("first");
    let donation = cx.var("donation");
    let second = cx.var("second");
    let denominator = cx.op(SymExprOp::Add, first.clone(), donation.clone());
    let numerator = cx.op(SymExprOp::Mul, second.clone(), first.clone());
    let shares = cx.op(SymExprOp::UDiv, numerator, denominator);
    let zero = cx.zero();
    let first_nonzero = cx.cmp(SymBoolExprOp::Ugt, first, zero.clone());
    let donation_nonzero = cx.cmp(SymBoolExprOp::Ugt, donation, zero.clone());
    let second_nonzero = cx.cmp(SymBoolExprOp::Ugt, second, zero.clone());
    let shares_zero = cx.eq(shares, zero);
    let constraints = vec![first_nonzero, donation_nonzero, second_nonzero, shares_zero];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_finds_wrapping_product_inequality_candidate() {
    let mut cx = SymCx::new();
    let ink = cx.var("ink");
    let art = cx.var("art");
    let spot = cx.var("spot");
    let rate = cx.var("rate");
    let zero = cx.zero();
    let ink_positive = cx.cmp(SymBoolExprOp::Ugt, ink.clone(), zero.clone());
    let art_above_ink = cx.cmp(SymBoolExprOp::Ugt, art.clone(), ink.clone());
    let spot_positive = cx.cmp(SymBoolExprOp::Ugt, spot.clone(), zero);
    let rate_above_spot = cx.cmp(SymBoolExprOp::Ugt, rate.clone(), spot.clone());
    let left_product = cx.op(SymExprOp::Mul, ink, spot);
    let right_product = cx.op(SymExprOp::Mul, art, rate);
    let wrapping = cx.cmp(SymBoolExprOp::Ult, left_product, right_product).not(&mut cx);
    let constraints = vec![ink_positive, art_above_ink, spot_positive, rate_above_spot, wrapping];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_finds_exact_mulmod_wide_intermediate_candidate() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let zero = cx.zero();
    let positive = cx.cmp(SymBoolExprOp::Ugt, a.clone(), zero.clone());
    let max = cx.constant(U256::MAX);
    let product = cx.mulmod(a.clone(), a, max);
    let exact = cx.eq(product, zero);
    let constraints = vec![positive, exact];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    assert_eq!(model.get(&Symbol::intern("a")), Some(&U256::MAX));
}

#[test]
fn hard_arithmetic_fallback_rejects_unvalidated_partial_model() {
    let mut cx = SymCx::new();
    let a = cx.var("a");
    let b = cx.var("b");
    let c = cx.var("c");
    let d = cx.var("d");
    let e = cx.var("e");
    let f = cx.var("f");
    let product = cx.op(SymExprOp::Mul, a, b);
    let one = cx.constant(U256::from(1));
    let product_eq_one = cx.eq(product, one);
    let three = cx.constant(U256::from(3));
    let c_eq_three = cx.eq(c, three);
    let four = cx.constant(U256::from(4));
    let d_eq_four = cx.eq(d, four);
    let five = cx.constant(U256::from(5));
    let e_eq_five = cx.eq(e, five);
    let six = cx.constant(U256::from(6));
    let f_eq_six = cx.eq(f, six);
    let constraints = vec![product_eq_one, c_eq_three, d_eq_four, e_eq_five, f_eq_six];

    let Some(model) = hard_arith_fallback_model(&constraints) else { return };

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_skips_symbolic_hashes() {
    let mut cx = SymCx::new();
    let x = cx.var("x");
    let hash = cx.hash_symbol(Symbol::intern("hash"), "sha256", vec![x.clone()]);
    let product = cx.op(SymExprOp::Mul, x, hash);
    let one = cx.constant(U256::from(1));
    let constraints = vec![cx.eq(product, one)];

    assert!(hard_arith_fallback_model(&constraints).is_none());
}

#[test]
fn concrete_dynamic_array_return_uses_raw_abi_encoding() {
    let mut cx = SymCx::new();
    let return_data = abi_concrete_value_return(
        &mut cx,
        DynSolValue::Array(vec![
            DynSolValue::Uint(U256::from(1), 256),
            DynSolValue::Uint(U256::from(2), 256),
        ]),
    );
    let encoded = return_data.read_concrete(&mut cx, "test return data").unwrap();
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
    let mut cx = SymCx::new();
    let mut solver = SmtLibSubprocessSolver::new(
        Ok(vec![SolverCommand::new(vec!["missing-z3".to_string()], false).unwrap()]),
        None,
        0,
        false,
    );

    let err = solver.is_sat(&mut cx, &[]).unwrap_err();

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
fn split_solver_command_rejects_invalid_double_quote() {
    let err = split_solver_command(r#"z3 "unterm"#).unwrap_err();

    assert!(
        matches!(err, SolverConfigError::InvalidShellQuoting),
        "expected InvalidShellQuoting, got {err:?}"
    );
    assert_eq!(err.to_string(), "invalid shell quoting in symbolic solver command");
}

#[test]
fn split_solver_command_rejects_invalid_single_quote() {
    let err = split_solver_command("z3 'unterm").unwrap_err();

    assert!(
        matches!(err, SolverConfigError::InvalidShellQuoting),
        "expected InvalidShellQuoting, got {err:?}"
    );
    assert_eq!(err.to_string(), "invalid shell quoting in symbolic solver command");
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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("hard-arith-is-sat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let zero = cx.zero();
    let x_positive = cx.cmp(SymBoolExprOp::Ugt, x.clone(), zero.clone());
    let y_positive = cx.cmp(SymBoolExprOp::Ugt, y.clone(), zero);
    let product = cx.op(SymExprOp::Mul, x, y);
    let four = cx.constant(U256::from(4));
    let product_eq_four = cx.eq(product, four);
    let constraints = vec![x_positive, y_positive, product_eq_four];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    let model = hard_arith_fallback_model(&normalized).unwrap();

    assert!(normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    assert!(solver.is_sat(&mut cx, &constraints).unwrap());
    assert!(solver.is_sat(&mut cx, &constraints).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("hard-arith-model-cache");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let zero = cx.zero();
    let x_positive = cx.cmp(SymBoolExprOp::Ugt, x.clone(), zero.clone());
    let y_positive = cx.cmp(SymBoolExprOp::Ugt, y.clone(), zero);
    let product = cx.op(SymExprOp::Mul, x, y);
    let four = cx.constant(U256::from(4));
    let product_eq_four = cx.eq(product, four);
    let constraints = vec![x_positive, y_positive, product_eq_four];

    let first = solver.model(&mut cx, &constraints).unwrap();
    assert!(constraints.iter().all(|constraint| constraint.eval_model(&first).unwrap()));
    let second = solver.model(&mut cx, &constraints).unwrap();
    assert_eq!(first, second);
    assert!(solver.is_sat(&mut cx, &constraints).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("hard-arith-is-sat-unsat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let zero = cx.zero();
    let x_is_zero = cx.eq(x.clone(), zero);
    let product = cx.op(SymExprOp::Mul, x, y);
    let one = cx.one();
    let product_eq_one = cx.eq(product, one);
    let constraints = vec![x_is_zero, product_eq_one];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    assert!(hard_arith_fallback_model(&normalized).is_none());
    assert!(!solver.is_sat(&mut cx, &constraints).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let one = cx.constant(U256::from(1));
    let two = cx.constant(U256::from(2));
    let constraints =
        vec![cx.bool_constant(true), cx.eq(x.clone(), one.clone()), cx.eq(y.clone(), two.clone())];
    let y_eq_two = cx.eq(y, two);
    let x_eq_one = cx.eq(x, one);
    let reordered_constraints = vec![y_eq_two, x_eq_one];

    assert!(solver.is_sat(&mut cx, &constraints).unwrap());
    assert!(solver.is_sat(&mut cx, &reordered_constraints).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-canonical");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let three = cx.constant(U256::from(3));
    let one = cx.constant(U256::from(1));
    let sum = cx.op(SymExprOp::Add, x.clone(), y.clone());
    let sum_eq_three = cx.eq(sum, three.clone());
    let x_eq_one = cx.eq(x.clone(), one.clone());
    let constraints = vec![cx.and(vec![sum_eq_three, x_eq_one])];
    let one_eq_x = cx.eq(one, x);
    let x_again = cx.var("x");
    let reordered_sum = cx.op(SymExprOp::Add, y, x_again);
    let three_eq_sum = cx.eq(three, reordered_sum);
    let reordered_constraints = vec![cx.and(vec![one_eq_x, three_eq_sum])];

    assert!(solver.is_sat(&mut cx, &constraints).unwrap());
    assert!(solver.is_sat(&mut cx, &reordered_constraints).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-dedup");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let constraint = cx.eq(x, one);
    let duplicated = vec![constraint.clone(), constraint.clone()];
    let deduplicated = vec![constraint];

    assert!(solver.is_sat(&mut cx, &duplicated).unwrap());
    assert!(solver.is_sat(&mut cx, &deduplicated).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-nested-dedup");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let constraint = cx.eq(x, one);
    let duplicated =
        vec![SymBoolExpr::raw_and(&mut cx, vec![constraint.clone(), constraint.clone()])];
    let deduplicated = vec![constraint];

    assert!(solver.is_sat(&mut cx, &duplicated).unwrap());
    assert!(solver.is_sat(&mut cx, &deduplicated).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-and-flatten");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let z = cx.var("z");
    let one = cx.constant(U256::from(1));
    let a = cx.eq(x, one);
    let two = cx.constant(U256::from(2));
    let b = cx.eq(y, two);
    let three = cx.constant(U256::from(3));
    let c = cx.eq(z, three);
    let child = SymBoolExpr::raw_and(&mut cx, vec![b.clone(), c.clone()]);
    let grouped = vec![SymBoolExpr::raw_and(&mut cx, vec![child, a.clone()])];
    let split = vec![c, a, b];

    assert!(solver.is_sat(&mut cx, &grouped).unwrap());
    assert!(solver.is_sat(&mut cx, &split).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-reversed-cmp");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 3, false);
    let x = cx.var("x");
    let y = cx.var("y");

    let ugt = cx.cmp(SymBoolExprOp::Ugt, x.clone(), y.clone());
    assert!(solver.is_sat(&mut cx, &[ugt]).unwrap());
    let ult = cx.cmp(SymBoolExprOp::Ult, y.clone(), x.clone());
    assert!(solver.is_sat(&mut cx, &[ult]).unwrap());
    let uge = cx.cmp(SymBoolExprOp::Uge, x.clone(), y.clone());
    assert!(solver.is_sat(&mut cx, &[uge]).unwrap());
    let ule = cx.cmp(SymBoolExprOp::Ule, y.clone(), x.clone());
    assert!(solver.is_sat(&mut cx, &[ule]).unwrap());
    let sgt = cx.cmp(SymBoolExprOp::Sgt, x.clone(), y.clone());
    assert!(solver.is_sat(&mut cx, &[sgt]).unwrap());
    let slt = cx.cmp(SymBoolExprOp::Slt, y, x);
    assert!(solver.is_sat(&mut cx, &[slt]).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-branch-complement");
    let commands = vec![counted_solver_command_sequence(&marker, &["sat", "unsat"])];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = cx.var("x");
    let ten = cx.constant(U256::from(10));
    let base = cx.cmp(SymBoolExprOp::Ult, x.clone(), ten);
    let one = cx.constant(U256::from(1));
    let condition = cx.eq(x, one);
    let not_condition = condition.clone().not(&mut cx);

    assert!(solver.is_sat(&mut cx, std::slice::from_ref(&base)).unwrap());
    assert!(!solver.is_sat_branch(&mut cx, &[base.clone(), condition]).unwrap());
    assert!(solver.is_sat_branch(&mut cx, &[base, not_condition]).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-unsat-base-branch-complement");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = cx.var("x");
    let two = cx.constant(U256::from(2));
    let one = cx.constant(U256::from(1));
    let lower = cx.cmp(SymBoolExprOp::Uge, x.clone(), two);
    let upper = cx.cmp(SymBoolExprOp::Ule, x.clone(), one.clone());
    let base = cx.and(vec![lower, upper]);
    let condition = cx.eq(x, one);

    assert!(!solver.is_sat_branch(&mut cx, &[base.clone(), condition.clone()]).unwrap());
    let not_condition = condition.not(&mut cx);
    assert!(!solver.is_sat_branch(&mut cx, &[base, not_condition]).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-cache-unknown");
    let commands = vec![counted_solver_command(&marker, "unknown")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 4, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let constraints = vec![cx.eq(x, one)];

    assert!(matches!(solver.is_sat(&mut cx, &constraints), Err(SymbolicError::SolverUnknown)));
    assert!(matches!(solver.is_sat(&mut cx, &constraints), Err(SymbolicError::SolverUnknown)));

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("model-cache");
    let model_output = "sat\n\
        ((define-fun x () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000001)\n\
        (define-fun y () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000002))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let y = cx.var("y");
    let one = cx.constant(U256::from(1));
    let two = cx.constant(U256::from(2));
    let constraints =
        vec![cx.bool_constant(true), cx.eq(x.clone(), one.clone()), cx.eq(y.clone(), two.clone())];
    let y_eq_two = cx.eq(y, two);
    let x_eq_one = cx.eq(x, one);
    let reordered_constraints = vec![y_eq_two, x_eq_one];

    assert_eq!(
        solver.model(&mut cx, &constraints).unwrap().get(&Symbol::intern("x")),
        Some(&U256::from(1))
    );
    assert_eq!(
        solver.model(&mut cx, &reordered_constraints).unwrap().get(&Symbol::intern("y")),
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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("model-cache-sat");
    let model_output = "sat\n\
        ((define-fun x () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000001))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let constraints = vec![cx.eq(x, one)];

    assert_eq!(
        solver.model(&mut cx, &constraints).unwrap().get(&Symbol::intern("x")),
        Some(&U256::from(1))
    );
    assert!(solver.is_sat(&mut cx, &constraints).unwrap());

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
    let mut cx = SymCx::new();
    let mut solver = SmtLibSubprocessSolver::new(Ok(Vec::new()), None, 1, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let constraint = cx.eq(x, one);
    let constraints = vec![constraint.clone(), constraint.not(&mut cx)];

    assert!(!solver.is_sat(&mut cx, &constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(stats.sat_queries, 1);
}

#[cfg(unix)]
#[test]
fn is_sat_reuses_cached_unsat_subset() {
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("unsat-subset-cache");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let x_eq_one = cx.eq(x, one);
    let y = cx.var("y");
    let two = cx.constant(U256::from(2));
    let y_eq_two = cx.eq(y, two);

    assert!(!solver.is_sat(&mut cx, std::slice::from_ref(&x_eq_one)).unwrap());
    assert!(!solver.is_sat(&mut cx, &[x_eq_one, y_eq_two]).unwrap());

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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("sat-subset-cache");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let x_eq_one = cx.eq(x, one);
    let y = cx.var("y");
    let two = cx.constant(U256::from(2));
    let y_eq_two = cx.eq(y, two);

    assert!(solver.is_sat(&mut cx, std::slice::from_ref(&x_eq_one)).unwrap());
    assert!(matches!(
        solver.is_sat(&mut cx, &[x_eq_one, y_eq_two]),
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
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("model-cache-unsat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let constraints = vec![cx.eq(x, one)];

    assert!(!solver.is_sat(&mut cx, &constraints).unwrap());
    assert!(
        matches!(solver.model(&mut cx, &constraints), Err(SymbolicError::Solver(message)) if message.contains("unsat"))
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
    let mut cx = SymCx::new();
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

    assert!(solver.is_sat(&mut cx, &[]).unwrap());
}

#[cfg(unix)]
#[test]
fn solver_process_timeout_returns_unknown() {
    let mut cx = SymCx::new();
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "cat >/dev/null; sleep 2; printf 'sat\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(1), 1, false);

    assert!(matches!(solver.is_sat(&mut cx, &[]), Err(SymbolicError::SolverUnknown)));
}

#[cfg(unix)]
#[test]
fn portfolio_scheduler_promotes_recent_sat_winner() {
    let mut cx = SymCx::new();
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

    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let first = vec![cx.eq(x.clone(), one)];
    let two = cx.constant(U256::from(2));
    let second = vec![cx.eq(x, two)];

    assert!(solver.is_sat(&mut cx, &first).unwrap());
    assert!(marker.exists());
    let _ = std::fs::remove_file(&marker);

    assert!(solver.is_sat(&mut cx, &second).unwrap());
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn portfolio_scheduler_promotes_recent_unsat_winner() {
    let mut cx = SymCx::new();
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

    let x = cx.var("x");
    let one = cx.constant(U256::from(1));
    let first = vec![cx.eq(x.clone(), one)];
    let two = cx.constant(U256::from(2));
    let second = vec![cx.eq(x, two)];

    assert!(!solver.is_sat(&mut cx, &first).unwrap());
    assert!(!solver.is_sat(&mut cx, &second).unwrap());

    let diagnostics = solver.take_diagnostics().unwrap();
    let last_outcomes =
        diagnostics.rsplit("--- symbolic solver portfolio outcomes ---").next().unwrap_or_default();
    assert!(last_outcomes.contains("#1 scheduled +0.000ns"));
    assert!(last_outcomes.contains("printf 'unsat"));
}

#[cfg(unix)]
#[test]
fn portfolio_scheduler_penalizes_invalid_models() {
    let mut cx = SymCx::new();
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
        let calldata = cx.var("calldata_0");
        let one = cx.constant(U256::from(1));
        let calldata_constraint = cx.eq(calldata, one);
        let portfolio_query = cx.var(&format!("portfolio_query_{query}"));
        let zero = cx.zero();
        let query_constraint = cx.eq(portfolio_query, zero);
        let constraints = vec![calldata_constraint, query_constraint];
        assert_eq!(
            solver.model(&mut cx, &constraints).unwrap().get(&Symbol::intern("calldata_0")),
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
    let mut cx = SymCx::new();
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

    assert!(solver.is_sat(&mut cx, &[]).unwrap());
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn portfolio_delayed_solver_can_rescue_stalled_leader() {
    let mut cx = SymCx::new();
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

    assert!(solver.is_sat(&mut cx, &[]).unwrap());
    assert!(started_at.elapsed() >= Duration::from_millis(100));
}

#[cfg(unix)]
#[test]
fn solver_capture_diagnostics_buffers_dump_smt_output() {
    let mut cx = SymCx::new();
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

    assert!(solver.is_sat(&mut cx, &[]).unwrap());
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
    let mut cx = SymCx::new();
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
    let calldata = cx.var("calldata_0");
    let byte = calldata.extracted_byte(&mut cx, 0);
    let shift = cx.constant(U256::from(248));
    let shifted = cx.op(SymExprOp::Shl, byte, shift);
    let zero = cx.zero();
    let shifted_zero = cx.eq(shifted.clone(), zero.clone());
    let shifted_positive = cx.cmp(SymBoolExprOp::Ugt, shifted, zero);
    let constraints = vec![shifted_zero, shifted_positive];

    solver.capture_diagnostics();

    assert!(solver.is_sat(&mut cx, &constraints).unwrap());
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
