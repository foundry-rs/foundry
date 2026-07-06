use super::{abi::*, runtime::*, *};

fn empty_state(cx: &mut SymCx) -> PathState {
    let calldata =
        SymbolicCalldata::selector_only(cx, &Function::parse("empty()").unwrap()).unwrap();
    PathState::new(cx, Address::ZERO, Address::ZERO, U256::ZERO, calldata, false)
}

fn symbolic_calldata_variants(
    cx: &mut SymCx,
    function: &Function,
    config: &SymbolicConfig,
) -> Result<Vec<SymbolicCalldata>, SymbolicError> {
    SymbolicCalldata::variants(function, config, cx)
}

fn add_words(cx: &mut SymCx, left: SymExpr, right: SymExpr) -> SymExpr {
    SymExpr::binop(cx, SymBinOp::Add, left, right)
}

macro_rules! sym_from_bytes {
    ($cx:ident, $bytes:expr) => {{
        let bytes = $bytes.collect::<Vec<_>>();
        SymExpr::from_bytes(&mut $cx, bytes)
    }};
}

fn precompile_address(index: u8) -> Address {
    let mut bytes = [0u8; 20];
    bytes[19] = index;
    Address::from(bytes)
}

fn symbolic_model<S: AsRef<str>>(
    cx: &mut SymCx,
    values: impl IntoIterator<Item = (S, U256)>,
) -> SymbolicModel {
    values.into_iter().map(|(name, value)| (cx.intern(name.as_ref()), value)).collect()
}

fn model_value(cx: &SymCx, model: &SymbolicModel, name: &str) -> Option<U256> {
    model.get(&cx.symbol(name)).copied()
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
fn binary_helpers_use_evm_operand_order() {
    let mut cx = SymCx::new();
    let mut state = empty_state(&mut cx);
    state.stack.push(SymExpr::constant(&mut cx, U256::from(2))).unwrap();
    state.stack.push(SymExpr::constant(&mut cx, U256::from(10))).unwrap();

    state.bin_word(&mut cx, SymBinOp::Sub).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymExpr::constant(&mut cx, U256::from(8)));
}

#[test]
fn comparison_helpers_use_evm_operand_order() {
    let mut cx = SymCx::new();
    let mut state = empty_state(&mut cx);
    state.stack.push(SymExpr::constant(&mut cx, U256::from(2))).unwrap();
    state.stack.push(SymExpr::constant(&mut cx, U256::from(10))).unwrap();

    state.cmp_word(&mut cx, SymCmpOp::Ult).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymExpr::constant(&mut cx, U256::ZERO));
}

#[test]
fn exp_helper_uses_evm_operand_order() {
    let mut cx = SymCx::new();
    let mut state = empty_state(&mut cx);
    state.stack.push(SymExpr::constant(&mut cx, U256::ZERO)).unwrap();
    state.stack.push(SymExpr::constant(&mut cx, U256::from(0x100))).unwrap();

    state.exp_word(&mut cx).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymExpr::constant(&mut cx, U256::from(1)));
}

#[test]
fn exp_helper_expands_symbolic_base_for_bounded_concrete_exponent() {
    let mut cx = SymCx::new();
    let mut state = empty_state(&mut cx);
    state.stack.push(SymExpr::constant(&mut cx, U256::from(16))).unwrap();
    state.stack.push(SymExpr::var(&mut cx, "base")).unwrap();

    state.exp_word(&mut cx).unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        result.eval_model(&symbolic_model(&mut cx, [("base".to_string(), U256::from(2))])).unwrap(),
        U256::from(65536)
    );
}

#[test]
fn exp_helper_expands_bounded_symbolic_exponent() {
    let mut cx = SymCx::new();
    let mut state = empty_state(&mut cx);
    let exponent = SymExpr::var(&mut cx, "exponent");
    let five = SymExpr::constant(&mut cx, U256::from(5));
    let bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, exponent.clone(), five);
    state.constraints.push(bound);
    state.stack.push(exponent).unwrap();
    state.stack.push(SymExpr::constant(&mut cx, U256::from(3))).unwrap();

    state.exp_word(&mut cx).unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        result
            .eval_model(&symbolic_model(&mut cx, [("exponent".to_string(), U256::from(5))]))
            .unwrap(),
        U256::from(243)
    );
}

#[test]
fn shift_helpers_accept_symbolic_amounts() {
    let mut cx = SymCx::new();
    let mut shl = empty_state(&mut cx);
    shl.stack.push(SymExpr::constant(&mut cx, U256::from(1))).unwrap();
    shl.stack.push(SymExpr::var(&mut cx, "shift")).unwrap();
    shl.shift_word(&mut cx, ShiftKind::Shl).unwrap();
    let shifted = shl.stack.pop().unwrap();

    assert_eq!(
        shifted
            .eval_model(&symbolic_model(&mut cx, [("shift".to_string(), U256::from(5))]))
            .unwrap(),
        U256::from(32)
    );
    assert_eq!(
        shifted
            .eval_model(&symbolic_model(&mut cx, [("shift".to_string(), U256::from(256))]))
            .unwrap(),
        U256::ZERO
    );

    let mut shr = empty_state(&mut cx);
    shr.stack.push(SymExpr::constant(&mut cx, U256::from(1) << 255)).unwrap();
    shr.stack.push(SymExpr::var(&mut cx, "shift")).unwrap();
    shr.shift_word(&mut cx, ShiftKind::Shr).unwrap();
    let shifted = shr.stack.pop().unwrap();

    assert_eq!(
        shifted
            .eval_model(&symbolic_model(&mut cx, [("shift".to_string(), U256::from(255))]))
            .unwrap(),
        U256::from(1)
    );
    assert_eq!(
        shifted
            .eval_model(&symbolic_model(&mut cx, [("shift".to_string(), U256::from(256))]))
            .unwrap(),
        U256::ZERO
    );

    let mut sar = empty_state(&mut cx);
    sar.stack.push(SymExpr::constant(&mut cx, U256::MAX)).unwrap();
    sar.stack.push(SymExpr::var(&mut cx, "shift")).unwrap();
    sar.shift_word(&mut cx, ShiftKind::Sar).unwrap();
    let shifted = sar.stack.pop().unwrap();

    assert_eq!(
        shifted
            .eval_model(&symbolic_model(&mut cx, [("shift".to_string(), U256::from(300))]))
            .unwrap(),
        U256::MAX
    );
}

#[test]
fn symbolic_division_guards_zero_divisor() {
    let mut cx = SymCx::new();
    let mut state = empty_state(&mut cx);
    state.stack.push(SymExpr::var(&mut cx, "den")).unwrap();
    state.stack.push(SymExpr::var(&mut cx, "num")).unwrap();

    state.bin_word_div_zero_guard(&mut cx, SymBinOp::UDiv).unwrap();

    let den = SymExpr::var(&mut cx, "den");
    let zero = SymExpr::zero(&mut cx);
    let den_is_zero = SymBoolExpr::eq(&mut cx, den.clone(), zero.clone());
    let num = SymExpr::var(&mut cx, "num");
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, num, den);
    let expected = SymExpr::ite(&mut cx, den_is_zero, zero, quotient);
    assert_eq!(state.stack.pop().unwrap(), expected);
}

#[test]
fn symbolic_byte_extracts_with_concrete_index() {
    let mut cx = SymCx::new();
    let word = SymExpr::var(&mut cx, "word");
    let actual = byte_word(&mut cx, U256::from(0), word.clone());
    let shift = SymExpr::constant(&mut cx, U256::from(248));
    let shifted = SymExpr::binop(&mut cx, SymBinOp::Shr, word, shift);
    let mask = SymExpr::constant(&mut cx, U256::from(0xff));
    let expected = SymExpr::binop(&mut cx, SymBinOp::And, shifted, mask);
    assert_eq!(actual, expected);
}

#[test]
fn symbolic_byte_extracts_with_symbolic_index() {
    let mut cx = SymCx::new();
    let word = U256::from_be_bytes(core::array::from_fn::<_, 32, _>(|idx| idx as u8));
    let index = SymExpr::var(&mut cx, "index");
    let word = SymExpr::constant(&mut cx, word);
    let byte = byte_word_dynamic(&mut cx, index, word);

    let in_range = symbolic_model(&mut cx, [("index".to_string(), U256::from(9))]);
    assert_eq!(byte.eval_model(&in_range).unwrap(), U256::from(9));

    let out_of_range = symbolic_model(&mut cx, [("index".to_string(), U256::from(32))]);
    assert_eq!(byte.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn symbolic_signextend_accepts_symbolic_index() {
    let mut cx = SymCx::new();
    let value = SymExpr::constant(&mut cx, U256::from(0x80));
    let index = SymExpr::var(&mut cx, "index");
    let extended = signextend_word_dynamic(&mut cx, index, value);

    let zero_index = symbolic_model(&mut cx, [("index".to_string(), U256::ZERO)]);
    assert_eq!(extended.eval_model(&zero_index).unwrap(), U256::MAX - U256::from(0x7f));

    let one_index = symbolic_model(&mut cx, [("index".to_string(), U256::from(1))]);
    assert_eq!(extended.eval_model(&one_index).unwrap(), U256::from(0x80));
}

#[test]
fn symbolic_byte_preserves_concrete_packed_selector_bytes() {
    let mut cx = SymCx::new();
    let selector = U256::from(0x12345678);
    let selector = SymExpr::constant(&mut cx, selector);
    let shift_224 = SymExpr::constant(&mut cx, U256::from(224));
    let selector = SymExpr::binop(&mut cx, SymBinOp::Shl, selector, shift_224);
    let arg = SymExpr::var(&mut cx, "arg");
    let shift_32 = SymExpr::constant(&mut cx, U256::from(32));
    let shifted_arg = SymExpr::binop(&mut cx, SymBinOp::Shr, arg, shift_32);
    let packed = SymExpr::binop(&mut cx, SymBinOp::Or, selector, shifted_arg);

    assert_eq!(
        byte_word(&mut cx, U256::from(0), packed.clone()),
        SymExpr::constant(&mut cx, U256::from(0x12))
    );
    assert_eq!(
        byte_word(&mut cx, U256::from(1), packed.clone()),
        SymExpr::constant(&mut cx, U256::from(0x34))
    );
    assert_eq!(
        byte_word(&mut cx, U256::from(2), packed.clone()),
        SymExpr::constant(&mut cx, U256::from(0x56))
    );
    assert_eq!(
        byte_word(&mut cx, U256::from(3), packed),
        SymExpr::constant(&mut cx, U256::from(0x78))
    );
}

#[test]
fn word_reassembly_preserves_split_symbolic_word() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let one = SymExpr::one(&mut cx);
    let original = SymExpr::binop(&mut cx, SymBinOp::Add, value, one);
    let bytes = original.clone().into_byte_exprs(&mut cx);

    assert_eq!(SymExpr::from_bytes(&mut cx, bytes), original);
}

#[test]
fn symbolic_address_aliases_match_abi_encoded_address_words() {
    let mut cx = SymCx::new();
    let source = SymExpr::var(&mut cx, "beneficiary");
    let address_mask = SymExpr::constant(&mut cx, (U256::from(1) << 160) - U256::from(1));
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, source.clone(), address_mask);
    let mut encoded = vec![SymExpr::zero(&mut cx); 12];
    encoded.extend((12..32).map(|idx| byte_word(&mut cx, U256::from(idx), masked.clone())));
    let reassembled = SymExpr::from_bytes(&mut cx, encoded);

    assert!(source.symbolic_address_equivalent(&reassembled));
    assert_eq!(source.symbolic_address_key(), reassembled.symbolic_address_key());
}

#[test]
fn selector_shift_simplifies_to_concrete_word() {
    let mut cx = SymCx::new();
    let selector = U256::from(0x12345678);
    let selector_word = SymExpr::constant(&mut cx, selector);
    let shift_224 = SymExpr::constant(&mut cx, U256::from(224));
    let shifted_selector = SymExpr::binop(&mut cx, SymBinOp::Shl, selector_word, shift_224.clone());
    let arg = SymExpr::var(&mut cx, "arg");
    let shift_32 = SymExpr::constant(&mut cx, U256::from(32));
    let shifted_arg = SymExpr::binop(&mut cx, SymBinOp::Shr, arg, shift_32);
    let call_word = SymExpr::binop(&mut cx, SymBinOp::Or, shifted_selector, shifted_arg);
    let selector_expr = SymExpr::binop(&mut cx, SymBinOp::Shr, call_word, shift_224);

    assert_eq!(selector_expr.known_word(), Some(selector));
}

#[test]
fn selector_equality_folds_known_word_expressions() {
    let mut cx = SymCx::new();
    let selector = U256::from(0x12345678u32);
    let other = U256::from(0x9a8325a0u32);
    let selector_word = SymExpr::constant(&mut cx, selector);
    let shift_224 = SymExpr::constant(&mut cx, U256::from(224));
    let shifted_selector = SymExpr::binop(&mut cx, SymBinOp::Shl, selector_word, shift_224.clone());
    let arg = SymExpr::var(&mut cx, "arg");
    let shift_32 = SymExpr::constant(&mut cx, U256::from(32));
    let shifted_arg = SymExpr::binop(&mut cx, SymBinOp::Shr, arg, shift_32);
    let call_word = SymExpr::binop(&mut cx, SymBinOp::Or, shifted_selector, shifted_arg);
    let selector_expr = SymExpr::binop(&mut cx, SymBinOp::Shr, call_word, shift_224);

    let selector_word = SymExpr::constant(&mut cx, selector);
    let actual = SymBoolExpr::eq(&mut cx, selector_expr.clone(), selector_word);
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
    let other_word = SymExpr::constant(&mut cx, other);
    let actual = SymBoolExpr::eq(&mut cx, selector_expr, other_word);
    let expected = SymBoolExpr::constant(&mut cx, false);
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
    let offset = SymExpr::zero(&mut cx);
    let loaded = call_data.load_word(&mut cx, offset).unwrap();
    let shift = SymExpr::constant(&mut cx, U256::from(224));
    let selector_expr = SymExpr::binop(&mut cx, SymBinOp::Shr, loaded, shift);

    assert_eq!(selector_expr.known_word(), Some(selector));
    let selector_word = SymExpr::constant(&mut cx, selector);
    let actual = SymBoolExpr::eq(&mut cx, selector_expr, selector_word);
    let expected = SymBoolExpr::constant(&mut cx, true);
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

    assert_eq!(call_data.size_word(), SymExpr::constant(&mut cx, U256::from(100)));
    assert_eq!(call_data.load(&mut cx, 4).unwrap(), SymExpr::constant(&mut cx, U256::from(32)));
    assert_eq!(call_data.load(&mut cx, 36).unwrap(), SymExpr::constant(&mut cx, U256::from(3)));
    let offset = SymExpr::constant(&mut cx, U256::from(71));
    let bytes = call_data.read_bytes_offset(&mut cx, offset, 1);
    assert_eq!(bytes.byte(&mut cx, 0), SymExpr::zero(&mut cx));

    let model = symbolic_model(
        &mut cx,
        [
            ("calldata_0_0".to_string(), U256::from(1)),
            ("calldata_0_1".to_string(), U256::from(2)),
            ("calldata_0_2".to_string(), U256::from(3)),
        ],
    );
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

    assert_eq!(call_data.load(&mut cx, 4).unwrap(), SymExpr::var(&mut cx, "calldata_0"));
    assert_eq!(call_data.load(&mut cx, 36).unwrap(), SymExpr::var(&mut cx, "calldata_1"));
    assert_eq!(call_data.load(&mut cx, 68).unwrap(), SymExpr::var(&mut cx, "calldata_2"));
}

#[test]
fn calldata_copy_to_memory_preserves_structural_words() {
    let mut cx = SymCx::new();
    let function = Function::parse("check(uint256,uint256)").unwrap();
    let calldata = symbolic_calldata_variants(&mut cx, &function, &SymbolicConfig::default())
        .unwrap()
        .remove(0);
    let mut memory = SymMemory::default();
    let dest = SymExpr::constant(&mut cx, U256::from(0x80));
    let offset = SymExpr::constant(&mut cx, U256::from(4));
    let call_data = calldata.call_data(&mut cx);

    memory.copy_calldata_to_offset(&mut cx, dest, offset, 64, &call_data).unwrap();

    assert_eq!(memory.load_word(&mut cx, 0x80).unwrap(), SymExpr::var(&mut cx, "calldata_0"));
    assert_eq!(memory.load_word(&mut cx, 0xa0).unwrap(), SymExpr::var(&mut cx, "calldata_1"));
}

#[test]
fn byte_concat_word_load_assembles_word_fragments() {
    let mut cx = SymCx::new();
    let selector = [0xde, 0xad, 0xbe, 0xef];
    let word = SymExpr::var(&mut cx, "calldata_0");
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
            .eval_model(&symbolic_model(
                &mut cx,
                [("calldata_0".to_string(), U256::from_be_bytes(word_value))]
            ))
            .unwrap(),
        U256::from_be_bytes(expected)
    );
}

#[test]
fn byte_concat_right_aligned_word_assembles_push_fragments() {
    let mut cx = SymCx::new();
    let immediate = SymExpr::var(&mut cx, "push_data");
    let opcode = SymBytes::concrete(&mut cx, vec![opcode::PUSH32]);
    let immediate_bytes = immediate.clone().into_bytes(&mut cx);
    let bytes = SymBytes::concat(&mut cx, [opcode, immediate_bytes]);

    assert_eq!(bytes.right_aligned_word(&mut cx, 1, 32), immediate);
    assert_eq!(
        bytes
            .right_aligned_word(&mut cx, 29, 4)
            .eval_model(&symbolic_model(
                &mut cx,
                [("push_data".to_string(), U256::from(0x12345678))]
            ))
            .unwrap(),
        U256::from(0x12345678)
    );
}

#[test]
fn byte_concat_concrete_bytes_flattens_slices_with_padding() {
    let mut cx = SymCx::new();
    let left = SymBytes::concrete(&mut cx, vec![1, 2, 3]);
    let right = SymBytes::concrete(&mut cx, vec![4, 5]);
    let bytes = SymBytes::concat(&mut cx, [left, right]).slice_concrete(&mut cx, 2, 5);

    assert_eq!(bytes.concrete_bytes(&mut cx, "test concrete bytes").unwrap(), vec![3, 4, 5, 0, 0]);
}

#[test]
fn calldata_load_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let calldata_bytes =
        (0u8..40).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let calldata_bytes = SymBytes::exprs(&mut cx, calldata_bytes);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let offset = SymExpr::var(&mut cx, "offset");
    let loaded = calldata.load_word(&mut cx, offset).unwrap();
    let expected =
        sym_from_bytes!(cx, (1u8..33).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))));

    assert_eq!(
        loaded
            .eval_model(&symbolic_model(&mut cx, [("offset".to_string(), U256::from(1))]))
            .unwrap(),
        expected.eval_model(&SymbolicModel::default()).unwrap()
    );
    assert_eq!(
        loaded
            .eval_model(&symbolic_model(&mut cx, [("offset".to_string(), U256::from(40))]))
            .unwrap(),
        U256::ZERO
    );
}

#[test]
fn memory_read_concrete_handles_overlapping_ranges() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let first = SymBytes::concrete(&mut cx, vec![1, 2, 3, 4]);
    let second = SymBytes::concrete(&mut cx, vec![9, 10]);

    memory.store_bytes(&mut cx, 2, first);
    memory.store_bytes(&mut cx, 4, second);

    assert_eq!(memory.read_concrete(&mut cx, 0, 8).unwrap(), vec![0, 0, 1, 2, 9, 10, 0, 0]);
}

#[test]
fn calldata_preserves_symbolic_size_for_call_frames() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let bytes = vec![
        SymExpr::constant(&mut cx, U256::from(0xaa)),
        SymExpr::constant(&mut cx, U256::from(0xbb)),
        SymExpr::constant(&mut cx, U256::from(0xcc)),
        SymExpr::constant(&mut cx, U256::from(0xdd)),
    ];
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.store_bytes(&mut cx, 0, bytes);
    let size = SymExpr::var(&mut cx, "size");
    let bounded_size = BoundedCopySize::Symbolic { size, max_size: 4 };
    let offset = SymExpr::zero(&mut cx);
    let input = bounded_size.read_from_memory(&mut cx, &memory, offset);
    let calldata = bounded_size.calldata(&mut cx, input);
    let model = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);

    assert_eq!(calldata.size_word().eval_model(&model).unwrap(), U256::from(2));
    let offset = SymExpr::zero(&mut cx);
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
    let dest = SymExpr::constant(&mut cx, U256::from(1));
    let offset = SymExpr::constant(&mut cx, U256::from(68));
    let call_data = calldata.call_data(&mut cx);

    memory.copy_calldata_to_offset(&mut cx, dest, offset, 3, &call_data).unwrap();
    let word = memory.load_word(&mut cx, 0).unwrap();

    let model = symbolic_model(
        &mut cx,
        [
            ("calldata_0_0".to_string(), U256::from(1)),
            ("calldata_0_1".to_string(), U256::from(2)),
            ("calldata_0_2".to_string(), U256::from(3)),
        ],
    );
    let mut expected = [0u8; 32];
    expected[1..4].copy_from_slice(&[1, 2, 3]);
    assert_eq!(word.eval_model(&model).unwrap(), U256::from_be_bytes(expected));
}

#[test]
fn memory_copies_symbolic_calldata_offset() {
    let mut cx = SymCx::new();
    let calldata_bytes =
        (0u8..40).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let calldata_bytes = SymBytes::exprs(&mut cx, calldata_bytes);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let mut memory = SymMemory::default();

    let dest = SymExpr::zero(&mut cx);
    let offset = SymExpr::var(&mut cx, "offset");
    memory.copy_calldata_to_offset(&mut cx, dest, offset, 2, &calldata).unwrap();
    let word = memory.load_word(&mut cx, 0).unwrap();

    let mut expected = [0u8; 32];
    expected[..2].copy_from_slice(&[4, 5]);
    assert_eq!(
        word.eval_model(&symbolic_model(&mut cx, [("offset".to_string(), U256::from(3))])).unwrap(),
        U256::from_be_bytes(expected)
    );
    assert_eq!(
        word.eval_model(&symbolic_model(&mut cx, [("offset".to_string(), U256::from(40))]))
            .unwrap(),
        U256::ZERO
    );
}

#[test]
fn memory_copies_symbolic_calldata_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let calldata_bytes =
        (0u8..8).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let calldata_bytes = SymBytes::exprs(&mut cx, calldata_bytes);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = SymExpr::zero(&mut cx);
    let offset = SymExpr::zero(&mut cx);
    let size = SymExpr::var(&mut cx, "size");

    memory.copy_calldata_symbolic_size(&mut cx, dest, offset, size, 4, &calldata).unwrap();

    let size_two = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = symbolic_model(&mut cx, [("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_copies_symbolic_bytecode_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = SymExpr::zero(&mut cx);
    let size = SymExpr::var(&mut cx, "size");
    let bytes = (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.copy_bytes_size_offset(&mut cx, dest, size, bytes).unwrap();

    let size_two = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = symbolic_model(&mut cx, [("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_copies_folded_symbolic_size_prefix_only() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = SymExpr::zero(&mut cx);
    let size = SymExpr::constant(&mut cx, U256::from(2));
    let bytes = (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.copy_bytes_size_offset(&mut cx, dest, size, bytes).unwrap();

    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &SymbolicModel::default()).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
}

#[test]
fn memory_skips_folded_zero_symbolic_size_copy() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let dest = SymExpr::zero(&mut cx);
    let size = SymExpr::zero(&mut cx);
    let bytes = (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.copy_bytes_size_offset(&mut cx, dest, size, bytes).unwrap();

    assert_eq!(memory.size_word(&mut cx), SymExpr::zero(&mut cx));
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &SymbolicModel::default()).unwrap(),
        vec![0, 0, 0, 0]
    );
}

#[test]
fn memory_reads_symbolic_size_with_zero_guarded_tail() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let source = (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let source = SymBytes::exprs(&mut cx, source);
    memory.store_bytes(&mut cx, 32, source);

    let offset = SymExpr::constant(&mut cx, U256::from(32));
    let size = SymExpr::var(&mut cx, "size");
    let bytes = memory.read_bytes_symbolic_size(&mut cx, offset, size, 4);

    let size_two = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);
    assert_eq!(bytes.eval_model(&mut cx, &size_two).unwrap(), vec![1, 2, 0, 0]);

    let size_four = symbolic_model(&mut cx, [("size".to_string(), U256::from(4))]);
    assert_eq!(bytes.eval_model(&mut cx, &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn memory_reads_folded_symbolic_size_prefix_only() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let source = (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let source = SymBytes::exprs(&mut cx, source);
    memory.store_bytes(&mut cx, 32, source);

    let offset = SymExpr::constant(&mut cx, U256::from(32));
    let size = SymExpr::constant(&mut cx, U256::from(2));
    let bytes = memory.read_bytes_symbolic_size(&mut cx, offset, size, 4);

    assert_eq!(bytes.eval_model(&mut cx, &SymbolicModel::default()).unwrap(), vec![1, 2, 0, 0]);
}

#[test]
fn memory_copies_symbolic_memory_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let source_bytes =
        (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let source_bytes = SymBytes::exprs(&mut cx, source_bytes);
    memory.store_bytes(&mut cx, 32, source_bytes);
    let dest = SymExpr::zero(&mut cx);
    let source = SymExpr::constant(&mut cx, U256::from(32));
    let size = SymExpr::var(&mut cx, "size");

    memory.copy_memory_symbolic_size(&mut cx, dest, source, size, 4).unwrap();

    let size_two = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = symbolic_model(&mut cx, [("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn memory_copies_symbolic_size_to_symbolic_dest() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0x80, tail);
    let source_bytes =
        (0u8..4).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let source_bytes = SymBytes::exprs(&mut cx, source_bytes);
    memory.store_bytes(&mut cx, 0x20, source_bytes);
    let dest = SymExpr::var(&mut cx, "dest");
    let source = SymExpr::constant(&mut cx, U256::from(0x20));
    let size = SymExpr::var(&mut cx, "size");

    memory.copy_memory_symbolic_size(&mut cx, dest, source, size, 4).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("dest".to_string(), U256::from(0x80)), ("size".to_string(), U256::from(2))],
    );
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
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = SymExpr::zero(&mut cx);
    let offset = SymExpr::zero(&mut cx);
    let size = SymExpr::var(&mut cx, "size");

    memory.copy_return_data_symbolic_size(&mut cx, dest, offset, size, 4, &return_data).unwrap();

    let size_two = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = symbolic_model(&mut cx, [("size".to_string(), U256::from(4))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_four).unwrap(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn returndata_reads_symbolic_offset() {
    let mut cx = SymCx::new();
    let return_data = SymReturnData::from_concrete_bytes(&mut cx, vec![1, 2, 3, 4]);
    let offset = SymExpr::var(&mut cx, "offset");
    let bytes = return_data.read_bytes_offset(&mut cx, offset, 2);

    let offset_one = symbolic_model(&mut cx, [("offset".to_string(), U256::from(1))]);
    assert_eq!(bytes.eval_model(&mut cx, &offset_one).unwrap(), vec![2, 3]);

    let offset_four = symbolic_model(&mut cx, [("offset".to_string(), U256::from(4))]);
    assert_eq!(bytes.eval_model(&mut cx, &offset_four).unwrap(), vec![0, 0]);
}

#[test]
fn memory_return_data_accepts_symbolic_size() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let bytes = vec![1, 2, 3, 4]
        .into_iter()
        .map(|byte| SymExpr::constant(&mut cx, U256::from(byte)))
        .collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    memory.store_bytes(&mut cx, 0, bytes);
    let offset = SymExpr::zero(&mut cx);
    let len = SymExpr::var(&mut cx, "len");

    let return_data = memory.return_data_symbolic_size(&mut cx, offset, len, 4).unwrap();

    let len_two = symbolic_model(&mut cx, [("len".to_string(), U256::from(2))]);
    assert_eq!(return_data.len_word().eval_model(&len_two).unwrap(), U256::from(2));
    assert_eq!(return_data.byte(&mut cx, 0).eval_model(&len_two).unwrap(), U256::from(1));
    assert_eq!(return_data.byte(&mut cx, 2).eval_model(&len_two).unwrap(), U256::ZERO);
}

#[test]
fn call_output_preserves_memory_beyond_symbolic_returndata_size() {
    let mut cx = SymCx::new();
    let bytes = vec![1, 2, 3, 4]
        .into_iter()
        .map(|byte| SymExpr::constant(&mut cx, U256::from(byte)))
        .collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    let len = SymExpr::var(&mut cx, "len");
    let return_data = SymReturnData::from_bytes_with_len(bytes, len);
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = SymExpr::zero(&mut cx);

    memory
        .copy_call_output_offset(&mut cx, dest, &BoundedCopySize::Concrete(4), &return_data)
        .unwrap();

    let len_two = symbolic_model(&mut cx, [("len".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &len_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let len_four = symbolic_model(&mut cx, [("len".to_string(), U256::from(4))]);
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

    assert_eq!(call_data.load(&mut cx, 4).unwrap(), SymExpr::constant(&mut cx, U256::from(32)));
    assert_eq!(call_data.load(&mut cx, 36).unwrap(), SymExpr::constant(&mut cx, U256::from(64)));
    assert_eq!(call_data.load(&mut cx, 68).unwrap(), SymExpr::constant(&mut cx, U256::from(160)));
    assert_eq!(call_data.load(&mut cx, 100).unwrap(), SymExpr::constant(&mut cx, U256::from(2)));
    assert_eq!(call_data.load(&mut cx, 196).unwrap(), SymExpr::constant(&mut cx, U256::from(3)));
}

#[test]
fn memory_round_trips_symbolic_words_as_bytes() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    memory.store_word(&mut cx, 7, value.clone());

    let model = symbolic_model(&mut cx, [("word".to_string(), U256::from(0x1234))]);
    assert_eq!(
        memory.load_word(&mut cx, 7).unwrap().eval_model(&model).unwrap(),
        value.eval_model(&model).unwrap()
    );
}

#[test]
fn memory_load_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    memory.store_word(&mut cx, 7, value);
    let offset = SymExpr::var(&mut cx, "offset");
    let loaded = memory.load_word_offset(&mut cx, offset).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(7)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));

    let out_of_range = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(39)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn memory_store_word_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    let offset = SymExpr::var(&mut cx, "offset");
    memory.store_word_offset(&mut cx, offset, value);
    let loaded = memory.load_word(&mut cx, 7).unwrap();

    let matching = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(7)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&matching).unwrap(), U256::from(0x1234));

    let non_matching = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(100)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&non_matching).unwrap(), U256::ZERO);
}

#[test]
fn memory_size_tracks_concrete_and_symbolic_extents() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let concrete = SymExpr::constant(&mut cx, U256::from(0x55));
    memory.store_word(&mut cx, 7, concrete);
    assert_eq!(memory.size_word(&mut cx), SymExpr::constant(&mut cx, U256::from(64)));

    let offset = SymExpr::var(&mut cx, "offset");
    let word = SymExpr::var(&mut cx, "word");
    memory.store_word_offset(&mut cx, offset, word);
    let size = memory.size_word(&mut cx);

    let below_concrete = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(9)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(size.eval_model(&below_concrete).unwrap(), U256::from(64));

    let above_concrete = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(70)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(size.eval_model(&above_concrete).unwrap(), U256::from(128));
}

#[test]
fn memory_concrete_write_overrides_older_symbolic_write() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let offset = SymExpr::var(&mut cx, "offset");
    let word = SymExpr::var(&mut cx, "word");
    memory.store_word_offset(&mut cx, offset, word);
    let concrete = SymExpr::constant(&mut cx, U256::from(0x55));
    memory.store_word(&mut cx, 7, concrete);
    let loaded = memory.load_word(&mut cx, 7).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(7)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x55));
}

#[test]
fn memory_dynamic_read_respects_concrete_overwrite_epoch() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let write_offset = SymExpr::var(&mut cx, "write_offset");
    let byte = SymExpr::var(&mut cx, "byte");
    memory.store_byte_offset(&mut cx, write_offset, byte);
    let concrete = SymExpr::constant(&mut cx, U256::from(0x55));
    memory.store_byte(&mut cx, 5, concrete);
    let read_offset = SymExpr::var(&mut cx, "read_offset");
    let loaded = memory.byte_dynamic_with_delta(&mut cx, &read_offset, 0);

    let model = symbolic_model(
        &mut cx,
        [
            ("write_offset".to_string(), U256::from(5)),
            ("read_offset".to_string(), U256::from(5)),
            ("byte".to_string(), U256::from(0xab)),
        ],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x55));
}

#[test]
fn memory_symbolic_write_after_concrete_overwrite_still_applies() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let older_offset = SymExpr::var(&mut cx, "older_offset");
    let older_byte = SymExpr::var(&mut cx, "older_byte");
    memory.store_byte_offset(&mut cx, older_offset, older_byte);
    let concrete = SymExpr::constant(&mut cx, U256::from(0x55));
    memory.store_byte(&mut cx, 5, concrete);
    let later_offset = SymExpr::var(&mut cx, "later_offset");
    let later_byte = SymExpr::var(&mut cx, "later_byte");
    memory.store_byte_offset(&mut cx, later_offset, later_byte);
    let loaded = memory.byte(&mut cx, 5);

    let concrete_wins = symbolic_model(
        &mut cx,
        [
            ("older_offset".to_string(), U256::from(5)),
            ("older_byte".to_string(), U256::from(0xaa)),
            ("later_offset".to_string(), U256::from(6)),
            ("later_byte".to_string(), U256::from(0xbb)),
        ],
    );
    assert_eq!(loaded.eval_model(&concrete_wins).unwrap(), U256::from(0x55));

    let later_symbolic_wins = symbolic_model(
        &mut cx,
        [
            ("older_offset".to_string(), U256::from(5)),
            ("older_byte".to_string(), U256::from(0xaa)),
            ("later_offset".to_string(), U256::from(5)),
            ("later_byte".to_string(), U256::from(0xbb)),
        ],
    );
    assert_eq!(loaded.eval_model(&later_symbolic_wins).unwrap(), U256::from(0xbb));
}

#[test]
fn memory_store_byte_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let offset = SymExpr::var(&mut cx, "offset");
    let byte = SymExpr::var(&mut cx, "byte");
    memory.store_byte_offset(&mut cx, offset, byte);
    let loaded = memory.byte(&mut cx, 0x80);

    let matching = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(0x80)), ("byte".to_string(), U256::from(0xab))],
    );
    assert_eq!(loaded.eval_model(&matching).unwrap(), U256::from(0xab));

    let non_matching = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(0x81)), ("byte".to_string(), U256::from(0xab))],
    );
    assert_eq!(loaded.eval_model(&non_matching).unwrap(), U256::ZERO);
}

#[test]
fn memory_read_bytes_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    memory.store_word(&mut cx, 7, value);
    let offset = SymExpr::var(&mut cx, "offset");
    let bytes = memory.read_bytes_offset(&mut cx, offset, 32).materialize(&mut cx);
    let loaded = SymExpr::from_bytes(&mut cx, bytes);

    let model = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(7)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));

    let out_of_range = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(39)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&out_of_range).unwrap(), U256::ZERO);
}

#[test]
fn memory_return_data_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    memory.store_word(&mut cx, 7, value);
    let offset = SymExpr::var(&mut cx, "offset");
    let return_data = memory.return_data(&mut cx, offset, 32).unwrap();
    let loaded = return_data.load_word(&mut cx, 0).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(7)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_copy_accepts_symbolic_source_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    memory.store_word(&mut cx, 7, value);
    let dest = SymExpr::constant(&mut cx, U256::from(64));
    let src = SymExpr::var(&mut cx, "src");
    memory.copy_memory_to_offset(&mut cx, dest, src, 32).unwrap();
    let loaded = memory.load_word(&mut cx, 64).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("src".to_string(), U256::from(7)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_copy_accepts_symbolic_destination_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let value = SymExpr::var(&mut cx, "word");

    memory.store_word(&mut cx, 7, value);
    let dest = SymExpr::var(&mut cx, "dest");
    let src = SymExpr::constant(&mut cx, U256::from(7));
    memory.copy_memory_to_offset(&mut cx, dest, src, 32).unwrap();
    let loaded = memory.load_word(&mut cx, 64).unwrap();

    let matching = symbolic_model(
        &mut cx,
        [("dest".to_string(), U256::from(64)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&matching).unwrap(), U256::from(0x1234));

    let non_matching = symbolic_model(
        &mut cx,
        [("dest".to_string(), U256::from(96)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&non_matching).unwrap(), U256::ZERO);
}

#[test]
fn memory_call_output_accepts_symbolic_destination_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();
    let word = SymExpr::var(&mut cx, "word");
    let bytes = word.into_bytes(&mut cx);
    let return_data = SymReturnData::from_bytes(&mut cx, bytes);
    let dest = SymExpr::var(&mut cx, "dest");

    memory
        .copy_call_output_offset(&mut cx, dest, &BoundedCopySize::Concrete(32), &return_data)
        .unwrap();
    let loaded = memory.load_word(&mut cx, 64).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("dest".to_string(), U256::from(64)), ("word".to_string(), U256::from(0x1234))],
    );
    assert_eq!(loaded.eval_model(&model).unwrap(), U256::from(0x1234));
}

#[test]
fn memory_call_output_accepts_symbolic_size_with_guarded_tail() {
    let mut cx = SymCx::new();
    let return_data = SymReturnData::from_concrete_bytes(&mut cx, vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0, tail);
    let dest = SymExpr::zero(&mut cx);
    let size = SymExpr::var(&mut cx, "size");
    let copy_size = BoundedCopySize::Symbolic { size, max_size: 4 };

    memory.copy_call_output_offset(&mut cx, dest, &copy_size, &return_data).unwrap();

    let size_two = symbolic_model(&mut cx, [("size".to_string(), U256::from(2))]);
    assert_eq!(
        memory.read_bytes(&mut cx, 0, 4).eval_model(&mut cx, &size_two).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );

    let size_four = symbolic_model(&mut cx, [("size".to_string(), U256::from(4))]);
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
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0x80, tail);
    let dest = SymExpr::var(&mut cx, "dest");
    let size = SymExpr::var(&mut cx, "size");
    let copy_size = BoundedCopySize::Symbolic { size, max_size: 4 };

    memory.copy_call_output_offset(&mut cx, dest, &copy_size, &return_data).unwrap();

    let model = symbolic_model(
        &mut cx,
        [("dest".to_string(), U256::from(0x80)), ("size".to_string(), U256::from(2))],
    );
    assert_eq!(
        memory.read_bytes(&mut cx, 0x80, 4).eval_model(&mut cx, &model).unwrap(),
        vec![1, 2, 0xaa, 0xaa]
    );
}

#[test]
fn memory_call_output_accepts_symbolic_destination_and_return_len() {
    let mut cx = SymCx::new();
    let bytes = vec![1, 2, 3, 4]
        .into_iter()
        .map(|byte| SymExpr::constant(&mut cx, U256::from(byte)))
        .collect();
    let bytes = SymBytes::exprs(&mut cx, bytes);
    let len = SymExpr::var(&mut cx, "len");
    let return_data = SymReturnData::from_bytes_with_len(bytes, len);
    let mut memory = SymMemory::default();
    let tail = SymExpr::constant(&mut cx, U256::from(0xaa));
    let tail = SymBytes::exprs(&mut cx, vec![tail; 4]);
    memory.store_bytes(&mut cx, 0x80, tail);
    let dest = SymExpr::var(&mut cx, "dest");

    memory
        .copy_call_output_offset(&mut cx, dest, &BoundedCopySize::Concrete(4), &return_data)
        .unwrap();

    let model = symbolic_model(
        &mut cx,
        [("dest".to_string(), U256::from(0x80)), ("len".to_string(), U256::from(2))],
    );
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
    let salt = SymExpr::var(&mut cx, "salt");
    let initcode = vec![opcode::STOP];
    let initcode_hash = SymExpr::constant(&mut cx, U256::from_be_bytes(keccak256(&initcode).0));
    let creator_word = SymExpr::constant(&mut cx, address_word(creator));

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
    let salt = SymExpr::var(&mut cx, "salt");
    let initcode_hash = SymExpr::var(&mut cx, "initcode_hash");
    let creator_word = SymExpr::constant(&mut cx, address_word(creator));

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
    assert!(first.get_var_name(&cx).is_some_and(|name| name.starts_with("create2_address_")));
}

#[test]
fn compute_create_cheatcode_helper_accepts_symbolic_nonce() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let nonce = SymExpr::var(&mut cx, "nonce");
    let creator_word = SymExpr::constant(&mut cx, address_word(creator));

    let first =
        compute_create_address_word(&mut cx, &mut state, creator_word.clone(), nonce.clone())
            .unwrap();
    let second = compute_create_address_word(&mut cx, &mut state, creator_word, nonce).unwrap();

    assert_eq!(first, second);
    assert!(first.get_var_name(&cx).is_some_and(|name| name.starts_with("create_address_")));
}

#[test]
fn compute_create_cheatcode_helper_accepts_symbolic_deployer() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let deployer = SymExpr::var(&mut cx, "deployer");
    let nonce = SymExpr::var(&mut cx, "nonce");

    let first = compute_create_address_word(&mut cx, &mut state, deployer.clone(), nonce.clone())
        .expect("symbolic deployer is supported");
    let second = compute_create_address_word(&mut cx, &mut state, deployer, nonce)
        .expect("symbolic deployer is supported");

    assert_eq!(first, second);
    assert!(first.get_var_name(&cx).is_some_and(|name| name.starts_with("create_address_")));
}

#[test]
fn compute_create2_cheatcode_helper_accepts_symbolic_deployer() {
    let mut cx = SymCx::new();
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(&mut cx, creator, Address::from([0xaa; 20]), false);
    let deployer = SymExpr::var(&mut cx, "deployer");
    let salt = SymExpr::var(&mut cx, "salt");
    let initcode_hash = SymExpr::var(&mut cx, "initcode_hash");

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
    assert!(first.get_var_name(&cx).is_some_and(|name| name.starts_with("create2_address_")));
}

#[test]
fn recorded_logs_return_data_matches_abi_encoding() {
    let mut cx = SymCx::new();
    let emitter = Address::from([0x33; 20]);
    let topic = B256::from([0x11; 32]);
    let data = vec![
        SymExpr::constant(&mut cx, U256::from(0x22)),
        SymExpr::constant(&mut cx, U256::from(0x33)),
    ];
    let data = SymBytes::exprs(&mut cx, data);
    let log = SymbolicLog::new(
        vec![SymExpr::constant(&mut cx, U256::from_be_bytes(topic.0))],
        SymExpr::constant(&mut cx, U256::from(2)),
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
    let data = vec![SymExpr::constant(&mut cx, U256::from(0x12)), SymExpr::var(&mut cx, "byte")];
    let data = SymBytes::exprs(&mut cx, data);
    let log = SymbolicLog::new(
        vec![SymExpr::var(&mut cx, "topic")],
        SymExpr::constant(&mut cx, U256::from(2)),
        data,
        emitter,
    );

    let return_data = recorded_logs_json_return_data(&mut cx, vec![log]).unwrap();
    let encoded = (0..return_data.len()).map(|idx| return_data.byte(&mut cx, idx)).collect();
    let model = symbolic_model(
        &mut cx,
        [("topic".to_string(), U256::from(0xabcd)), ("byte".to_string(), U256::from(0xef))],
    );
    let encoded = SymBytes::exprs(&mut cx, encoded).eval_model(&mut cx, &model).unwrap();
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
        SymExpr::constant(&mut cx, U256::from(0x22)),
        SymExpr::constant(&mut cx, U256::from(0x33)),
        SymExpr::constant(&mut cx, U256::from(0x44)),
    ];
    let bytes = SymBytes::exprs(&mut cx, bytes);
    let len = SymExpr::var(&mut cx, "len");
    let encoded = encode_packed_bytes_with_len(&mut cx, len, &bytes);

    assert_eq!(
        encoded
            .word_at(&mut cx, 0)
            .eval_model(&symbolic_model(&mut cx, [("len".to_string(), U256::from(2))]))
            .unwrap(),
        U256::from(2)
    );
}

#[test]
fn symbolic_world_resolves_symbolic_create2_address_aliases() {
    let mut cx = SymCx::new();
    let mut world = SymbolicWorld::default();
    let word = SymExpr::var(&mut cx, "create2_address");
    let address = world.symbolic_address_slot(word.clone());
    let address_mask = SymExpr::constant(&mut cx, (U256::from(1) << 160) - U256::from(1));
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, word.clone(), address_mask);

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
    let salt = SymExpr::var(&mut cx, "salt");
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
    let salt = SymExpr::var(&mut cx, "salt");
    let init_word = SymExpr::var(&mut cx, "init_word");
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
    let runtime_byte = SymExpr::var(&mut cx, "runtime_byte");
    let bytes = SymBytes::exprs(&mut cx, vec![runtime_byte.clone()]);
    let data = SymReturnData::from_bytes(&mut cx, bytes);

    let code = data.to_code(&mut cx).unwrap();

    assert_eq!(code.read_bytes(&mut cx, 0, 1).materialize(&mut cx), vec![runtime_byte]);
}

#[test]
fn symbolic_runtime_size_is_not_installed_as_concrete_code() {
    let mut cx = SymCx::new();
    let stop = SymExpr::constant(&mut cx, U256::from(opcode::STOP));
    let bytes = SymBytes::exprs(&mut cx, vec![stop]);
    let len = SymExpr::var(&mut cx, "runtime_len");
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
    let stop = SymExpr::constant(&mut cx, U256::from(opcode::STOP));
    let constructor_arg = SymExpr::var(&mut cx, "constructor_arg_byte");
    let bytes = SymBytes::exprs(&mut cx, vec![stop, constructor_arg]);
    let initcode = SymCode::from_bytes(&mut cx, bytes);

    let init_bytes = initcode.read_bytes(&mut cx, 0, 2);
    memory.store_bytes(&mut cx, 0, init_bytes);

    assert_eq!(memory.byte(&mut cx, 0), SymExpr::constant(&mut cx, U256::from(opcode::STOP)));
    assert_eq!(memory.byte(&mut cx, 1), SymExpr::var(&mut cx, "constructor_arg_byte"));
}

#[test]
fn symbolic_codecopy_accepts_symbolic_offsets() {
    let mut cx = SymCx::new();
    let code_bytes = (0u8..40).map(|idx| SymExpr::constant(&mut cx, U256::from(idx + 1))).collect();
    let code_bytes = SymBytes::exprs(&mut cx, code_bytes);
    let code = SymCode::from_bytes(&mut cx, code_bytes);
    let mut memory = SymMemory::default();

    let offset = SymExpr::var(&mut cx, "offset");
    let code_bytes = code.read_bytes_offset(&mut cx, offset, 2);
    memory.store_bytes(&mut cx, 0, code_bytes);
    let word = memory.load_word(&mut cx, 0).unwrap();

    let mut expected = [0u8; 32];
    expected[..2].copy_from_slice(&[4, 5]);
    assert_eq!(
        word.eval_model(&symbolic_model(&mut cx, [("offset".to_string(), U256::from(3))])).unwrap(),
        U256::from_be_bytes(expected)
    );
    assert_eq!(
        word.eval_model(&symbolic_model(&mut cx, [("offset".to_string(), U256::from(40))]))
            .unwrap(),
        U256::ZERO
    );
}

#[test]
fn symbolic_initcode_accepts_symbolic_memory_offsets() {
    let mut cx = SymCx::new();
    let mut memory = SymMemory::default();

    let stop = SymExpr::constant(&mut cx, U256::from(opcode::STOP));
    let arg = SymExpr::var(&mut cx, "arg");
    let bytes = SymBytes::exprs(&mut cx, vec![stop, arg]);
    memory.store_bytes(&mut cx, 7, bytes);
    let offset = SymExpr::var(&mut cx, "offset");
    let initcode = SymCode::from_memory_offset(&mut cx, &memory, offset, 2);
    let mut bytes = initcode.read_bytes(&mut cx, 0, 2).materialize(&mut cx);
    bytes.extend((0..30).map(|_| SymExpr::zero(&mut cx)));
    let word = SymExpr::from_bytes(&mut cx, bytes);

    let mut expected = [0u8; 32];
    expected[0] = opcode::STOP;
    expected[1] = 0x2a;
    let model = symbolic_model(
        &mut cx,
        [("offset".to_string(), U256::from(7)), ("arg".to_string(), U256::from(0x2a))],
    );
    assert_eq!(word.eval_model(&model).unwrap(), U256::from_be_bytes(expected));
}

#[test]
fn path_state_extracts_constrained_symbolic_usize() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let offset = SymExpr::var(&mut cx, "offset");

    let seven = SymExpr::constant(&mut cx, U256::from(7));
    let constraint = SymBoolExpr::eq(&mut cx, offset.clone(), seven);
    state.constraints.push(constraint);

    assert_eq!(state.constrained_usize(&mut cx, &offset), Some(7));
}

#[test]
fn path_state_extracts_symbolic_usize_upper_bound() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let size = SymExpr::var(&mut cx, "size");

    let five = SymExpr::constant(&mut cx, U256::from(5));
    let constraint = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, size.clone(), five);
    state.constraints.push(constraint);

    assert_eq!(state.upper_bound_usize(&mut cx, &size), Some(4));
}

#[test]
fn path_state_child_replaces_frame_and_resets_local_loop_state() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    state.call_depth = 2;
    state.next_symbol = 7;
    state.loop_jumps.insert(3, 4);

    let parent_stack = SymExpr::constant(&mut cx, U256::from(0xab));
    state.stack.push(parent_stack).unwrap();

    let constrained = SymExpr::var(&mut cx, "constrained");
    let seven = SymExpr::constant(&mut cx, U256::from(7));
    let constraint = SymBoolExpr::eq(&mut cx, constrained, seven);
    state.constraints.push(constraint.clone());

    let cached = Address::from([0x22; 20]);
    state.world.set_nonce(cached, 9);

    let calldata_bytes = SymBytes::empty(&mut cx);
    let calldata = SymCalldata::from_bytes(&mut cx, calldata_bytes);
    let callvalue = SymExpr::zero(&mut cx);
    let child_address = Address::from([0x11; 20]);
    let frame = CallFrame::new(
        &mut cx,
        child_address,
        child_address,
        child_address,
        Address::ZERO,
        callvalue,
        false,
        calldata,
    );

    let child = state.child(frame);

    assert_eq!(child.call_depth, 3);
    assert_eq!(child.next_symbol, 7);
    assert_eq!(child.constraints, vec![constraint]);
    assert_eq!(child.world.cached_nonce(cached), Some(9));
    assert_eq!(child.address, child_address);
    assert!(child.loop_jumps.is_empty());
    assert_eq!(state.loop_jumps.get(&3), Some(&4));
    assert!(child.stack.peek(0).is_err());
}

#[test]
fn path_state_extracts_constrained_symbolic_usize_from_encoded_bool_word() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let offset = SymExpr::var(&mut cx, "offset");
    let offset_expr = offset.clone();
    let mask = SymExpr::constant(&mut cx, U256::from(0xffff));

    let masked_offset = SymExpr::binop(&mut cx, SymBinOp::And, offset_expr.clone(), mask.clone());
    let offset_constraint = SymBoolExpr::eq(&mut cx, masked_offset, offset_expr.clone());
    state.constraints.push(offset_constraint);
    let expected_offset = SymExpr::constant(&mut cx, U256::from(0x80));
    let masked_offset = SymExpr::binop(&mut cx, SymBinOp::And, mask, offset_expr);
    let condition = SymBoolExpr::eq(&mut cx, expected_offset, masked_offset);
    let one = SymExpr::one(&mut cx);
    let zero = SymExpr::zero(&mut cx);
    let condition_word = SymExpr::ite(&mut cx, condition, one, zero.clone());
    let shifted_condition = SymExpr::binop(&mut cx, SymBinOp::Shr, condition_word, zero.clone());
    let byte_mask = SymExpr::constant(&mut cx, U256::from(0xff));
    let bool_byte = SymExpr::binop(&mut cx, SymBinOp::And, shifted_condition, byte_mask);
    let bool_word = SymExpr::binop(&mut cx, SymBinOp::Or, zero.clone(), bool_byte);
    let bool_word_zero = SymBoolExpr::eq(&mut cx, bool_word, zero);
    state.constraints.push(bool_word_zero.not(&mut cx));

    assert_eq!(state.constrained_usize(&mut cx, &offset), Some(0x80));
}

#[test]
fn path_state_evaluates_compound_constrained_symbolic_word() {
    let mut cx = SymCx::new();
    let mut state = PathState::empty(&mut cx, Address::ZERO, Address::ZERO, false);
    let value = SymExpr::var(&mut cx, "value");
    let beef = SymExpr::constant(&mut cx, U256::from(0xbeef));
    let value_constraint = SymBoolExpr::eq(&mut cx, value.clone(), beef);
    state.constraints.push(value_constraint);

    let zero = SymExpr::zero(&mut cx);
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let masked_value = SymExpr::binop(&mut cx, SymBinOp::And, value, u64_max);
    let encoded_word = SymExpr::binop(&mut cx, SymBinOp::Or, zero, masked_value);

    assert_eq!(state.constrained_word(&mut cx, &encoded_word), Some(U256::from(0xbeef)));
}

#[test]
fn symbolic_push_data_reconstructs_symbolic_word() {
    let mut cx = SymCx::new();
    let push2 = SymExpr::constant(&mut cx, U256::from(opcode::PUSH2));
    let hi = SymExpr::var(&mut cx, "immutable_hi");
    let lo = SymExpr::var(&mut cx, "immutable_lo");
    let bytes = SymBytes::exprs(&mut cx, vec![push2, hi, lo]);
    let code = SymCode::from_bytes(&mut cx, bytes);

    let word = code.push_data_word(&mut cx, 1, 2);

    assert!(word.as_const().is_none());
}

#[test]
fn symbolic_push32_preserves_structural_word() {
    let mut cx = SymCx::new();
    let immediate = SymExpr::var(&mut cx, "immutable");
    let opcode = SymBytes::concrete(&mut cx, vec![opcode::PUSH32]);
    let immediate_bytes = immediate.clone().into_bytes(&mut cx);
    let bytes = SymBytes::concat(&mut cx, [opcode, immediate_bytes]);
    let code = SymCode::from_bytes(&mut cx, bytes);

    assert_eq!(code.push_data_word(&mut cx, 1, 32), immediate);
}

#[test]
fn abi_bytes_return_encodes_symbolic_bytes() {
    let mut cx = SymCx::new();
    let byte_0 = SymExpr::var(&mut cx, "calldata_byte_0");
    let byte_1 = SymExpr::constant(&mut cx, U256::from(0x42));
    let ret = abi_bytes_return(&mut cx, vec![byte_0.clone(), byte_1.clone()]);
    let offset_bytes = (0..32).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let offset_word = SymExpr::from_bytes(&mut cx, offset_bytes);
    let offset = SymExpr::constant(&mut cx, U256::from(32));

    assert_eq!(offset_word, offset);
    let len_bytes = (32..64).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let len_word = SymExpr::from_bytes(&mut cx, len_bytes);
    let len = SymExpr::constant(&mut cx, U256::from(2));
    assert_eq!(len_word, len);
    assert_eq!(ret.byte(&mut cx, 64), byte_0);
    assert_eq!(ret.byte(&mut cx, 65), byte_1);
}

#[test]
fn abi_bytes_return_can_encode_symbolic_length() {
    let mut cx = SymCx::new();
    let len = SymExpr::var(&mut cx, "len");
    let byte_0 = SymExpr::var(&mut cx, "byte_0");
    let byte_1 = SymExpr::var(&mut cx, "byte_1");
    let ret = abi_bytes_return_with_len(&mut cx, len.clone(), vec![byte_0.clone(), byte_1.clone()]);
    let offset_bytes = (0..32).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let offset_word = SymExpr::from_bytes(&mut cx, offset_bytes);
    let offset = SymExpr::constant(&mut cx, U256::from(32));

    assert_eq!(offset_word, offset);
    let len_bytes = (32..64).map(|idx| ret.byte(&mut cx, idx)).collect::<Vec<_>>();
    let len_word = SymExpr::from_bytes(&mut cx, len_bytes);
    assert_eq!(len_word, len);
    assert_eq!(ret.byte(&mut cx, 64), byte_0);
    assert_eq!(ret.byte(&mut cx, 65), byte_1);
}

#[test]
fn symbolic_keccak_is_deterministic_for_same_symbolic_bytes() {
    let mut cx = SymCx::new();
    let bytes = SymExpr::var(&mut cx, "slot_key").into_byte_exprs(&mut cx);

    let first = keccak_word(&mut cx, bytes.clone());
    let second = keccak_word(&mut cx, bytes);

    assert_eq!(first, second);
    assert!(first.is_keccak());
}

#[test]
fn symbolic_keccak_tracks_symbolic_length() {
    let mut cx = SymCx::new();
    let bytes = vec![
        SymExpr::var(&mut cx, "byte_0"),
        SymExpr::var(&mut cx, "byte_1"),
        SymExpr::zero(&mut cx),
    ];
    let len = SymExpr::var(&mut cx, "len");

    let word = keccak_word_with_len(&mut cx, bytes, len);

    let (hash_len, hash_bytes_len) =
        word.keccak_len_and_byte_count().expect("expected symbolic keccak term");
    assert_eq!(hash_len, &SymExpr::var(&mut cx, "len"));
    assert_eq!(hash_bytes_len, 3);
}

#[test]
fn sym_expr_eval_computes_symbolic_keccak_from_model() {
    let mut cx = SymCx::new();
    let owner = Address::from([0x11; 20]);
    let slot = U256::from(1);
    let mut bytes = SymExpr::var(&mut cx, "owner").into_byte_exprs(&mut cx);
    bytes.extend(SymExpr::constant(&mut cx, slot).into_byte_exprs(&mut cx));
    let word = keccak_word(&mut cx, bytes);

    let mut input = Vec::new();
    input.extend(address_word(owner).to_be_bytes::<32>());
    input.extend(slot.to_be_bytes::<32>());
    let expected = U256::from_be_bytes(keccak256(input).0);
    let model = symbolic_model(&mut cx, [("owner".to_string(), address_word(owner))]);

    assert_eq!(word.eval_model(&model).unwrap(), expected);
}

#[test]
fn symbolic_hash_precompiles_are_deterministic_for_same_symbolic_input() {
    let mut cx = SymCx::new();
    let input = vec![SymExpr::var(&mut cx, "input_0"), SymExpr::var(&mut cx, "input_1")];

    let input_len = SymExpr::constant(&mut cx, U256::from(input.len()));
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
    let sha_word = SymExpr::from_bytes(&mut cx, sha_bytes);
    let sha_again_bytes = (0..32).map(|idx| sha_again.byte(&mut cx, idx)).collect::<Vec<_>>();
    let sha_again_word = SymExpr::from_bytes(&mut cx, sha_again_bytes);

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
        assert_eq!(ecrecover.byte(&mut cx, idx), SymExpr::zero(&mut cx));
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
        assert_eq!(ripemd.byte(&mut cx, idx), SymExpr::zero(&mut cx));
    }
    for idx in 0..32 {
        assert_eq!(ripemd.byte(&mut cx, idx), ripemd_again.byte(&mut cx, idx));
    }
}

#[test]
fn identity_precompile_preserves_symbolic_input_len() {
    let mut cx = SymCx::new();
    let input = vec![
        SymExpr::constant(&mut cx, U256::from(1)),
        SymExpr::constant(&mut cx, U256::from(2)),
        SymExpr::constant(&mut cx, U256::from(3)),
        SymExpr::constant(&mut cx, U256::from(4)),
    ];
    let input_len = SymExpr::var(&mut cx, "size");
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
    assert_eq!(return_data.byte(&mut cx, 0), SymExpr::constant(&mut cx, U256::from(1)));
    assert_eq!(return_data.byte(&mut cx, 3), SymExpr::constant(&mut cx, U256::from(4)));
}

#[test]
fn advanced_precompiles_accept_symbolic_payloads() {
    let mut cx = SymCx::new();
    let mut modexp_input = vec![SymExpr::zero(&mut cx); 99];
    modexp_input[31] = SymExpr::constant(&mut cx, U256::from(1));
    modexp_input[63] = SymExpr::constant(&mut cx, U256::from(1));
    modexp_input[95] = SymExpr::constant(&mut cx, U256::from(1));
    modexp_input[96] = SymExpr::var(&mut cx, "base");
    modexp_input[97] = SymExpr::constant(&mut cx, U256::from(5));
    modexp_input[98] = SymExpr::constant(&mut cx, U256::from(13));

    let modexp_len = SymExpr::constant(&mut cx, U256::from(modexp_input.len()));
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
    let modexp_len = SymExpr::constant(&mut cx, U256::from(modexp_input.len()));
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

    let mut blake_input = vec![SymExpr::var(&mut cx, "blake_input"); 213];
    blake_input[212] = SymExpr::zero(&mut cx);
    let blake_len = SymExpr::constant(&mut cx, U256::from(213));
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
    let bn_input = vec![SymExpr::var(&mut cx, "point"); 128];
    let bn_len = SymExpr::constant(&mut cx, U256::from(128));
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

    let blake_input = vec![SymExpr::var(&mut cx, "blake_input"); 213];
    let blake_len = SymExpr::constant(&mut cx, U256::from(213));
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
    let key = SymExpr::var(&mut cx, "slot");
    let value = SymExpr::var(&mut cx, "value");
    let writes = vec![StorageWrite::new(address, key.clone(), value.clone())];
    let base = SymExpr::zero(&mut cx);

    assert_eq!(StorageWrite::select_from(&mut cx, &writes, address, key, base), value);
}

#[test]
fn symbolic_storage_uses_conditional_value_for_maybe_equal_key() {
    let mut cx = SymCx::new();
    let address = Address::from([0x11; 20]);
    let write_key = SymExpr::var(&mut cx, "write_slot");
    let read_key = SymExpr::var(&mut cx, "read_slot");
    let value = SymExpr::var(&mut cx, "value");
    let writes = vec![StorageWrite::new(address, write_key.clone(), value.clone())];
    let base = SymExpr::zero(&mut cx);
    let selected = StorageWrite::select_from(&mut cx, &writes, address, read_key.clone(), base);
    let keys_match = SymBoolExpr::eq(&mut cx, read_key, write_key);
    let zero = SymExpr::zero(&mut cx);
    let expected = SymExpr::ite(&mut cx, keys_match, value, zero);

    assert_eq!(selected, expected);
}

#[test]
fn symbolic_storage_key_equality_decomposes_keccak_offsets() {
    let mut cx = SymCx::new();
    let owner = SymExpr::var(&mut cx, "owner");
    let owner_bytes = owner.into_byte_exprs(&mut cx);
    let base = keccak_word(&mut cx, owner_bytes);
    let left_index = SymExpr::var(&mut cx, "left_index");
    let right_index = SymExpr::var(&mut cx, "right_index");
    let left = add_words(&mut cx, base.clone(), left_index);
    let right = add_words(&mut cx, base, right_index);

    let actual = left.storage_key_eq(&mut cx, &right);
    let left_index = SymExpr::var(&mut cx, "left_index");
    let right_index = SymExpr::var(&mut cx, "right_index");
    let expected = SymBoolExpr::eq(&mut cx, left_index, right_index);
    assert_eq!(actual, expected);
}

#[test]
fn symbolic_storage_key_equality_expands_distinct_keccak_bases() {
    let mut cx = SymCx::new();
    let left_owner = SymExpr::var(&mut cx, "left_owner");
    let left_base = keccak_word(&mut cx, vec![left_owner]);
    let right_owner = SymExpr::var(&mut cx, "right_owner");
    let right_base = keccak_word(&mut cx, vec![right_owner]);
    let index = SymExpr::var(&mut cx, "index");

    let left = add_words(&mut cx, left_base, index.clone());
    let right = add_words(&mut cx, right_base, index);
    let condition = left.storage_key_eq(&mut cx, &right);

    let left_owner = SymExpr::var(&mut cx, "left_owner");
    let right_owner = SymExpr::var(&mut cx, "right_owner");
    let expected = SymBoolExpr::eq(&mut cx, left_owner, right_owner);
    assert_eq!(condition, expected);
}

#[test]
fn symbolic_storage_key_equality_compares_mapping_key_words() {
    let mut cx = SymCx::new();
    let left_owner = SymExpr::var(&mut cx, "left_owner");
    let right_owner = SymExpr::var(&mut cx, "right_owner");
    let slot = SymExpr::constant(&mut cx, U256::from(3));

    let mut left_key_bytes = left_owner.clone().into_byte_exprs(&mut cx);
    left_key_bytes.extend(slot.clone().into_byte_exprs(&mut cx));
    let left = keccak_word(&mut cx, left_key_bytes);

    let mut right_key_bytes = right_owner.clone().into_byte_exprs(&mut cx);
    right_key_bytes.extend(slot.into_byte_exprs(&mut cx));
    let right = keccak_word(&mut cx, right_key_bytes);

    let condition = left.storage_key_eq(&mut cx, &right);
    let expected = SymBoolExpr::eq(&mut cx, left_owner, right_owner);

    assert_eq!(condition, expected);
}

#[test]
fn symbolic_storage_key_equality_rejects_concrete_plain_slot_alias() {
    let mut cx = SymCx::new();
    let owner = SymExpr::var(&mut cx, "owner");
    let owner = owner.into_byte_exprs(&mut cx);
    let owner = keccak_word(&mut cx, owner);
    let zero = SymExpr::zero(&mut cx);
    let layout_key = add_words(&mut cx, owner, zero);
    let plain_slot = SymExpr::zero(&mut cx);
    let actual = layout_key.storage_key_eq(&mut cx, &plain_slot);
    let expected = SymBoolExpr::constant(&mut cx, false);

    assert_eq!(actual, expected);
}

#[test]
fn nested_mapping_key_does_not_alias_plain_mapping_key_under_model() {
    let mut cx = SymCx::new();
    let owner = SymExpr::var(&mut cx, "owner");
    let spender = SymExpr::var(&mut cx, "spender");
    let recipient = SymExpr::var(&mut cx, "recipient");

    let mut balance_key_bytes = recipient.into_byte_exprs(&mut cx);
    balance_key_bytes.extend(SymExpr::constant(&mut cx, U256::ZERO).into_byte_exprs(&mut cx));
    let balance_key = keccak_word(&mut cx, balance_key_bytes);

    let mut inner_key_bytes = owner.into_byte_exprs(&mut cx);
    inner_key_bytes.extend(SymExpr::constant(&mut cx, U256::from(1)).into_byte_exprs(&mut cx));
    let inner_key = keccak_word(&mut cx, inner_key_bytes);

    let mut allowance_key_bytes = spender.into_byte_exprs(&mut cx);
    allowance_key_bytes.extend(inner_key.into_byte_exprs(&mut cx));
    let allowance_key = keccak_word(&mut cx, allowance_key_bytes);

    let same_address = precompile_address(0x60);
    let model = symbolic_model(
        &mut cx,
        [
            ("owner".to_string(), address_word(Address::from([0x11; 20]))),
            ("spender".to_string(), address_word(same_address)),
            ("recipient".to_string(), address_word(same_address)),
        ],
    );
    let condition = balance_key.storage_key_eq(&mut cx, &allowance_key);

    assert_eq!(condition, SymBoolExpr::constant(&mut cx, false));
    assert!(!condition.eval_model(&model).unwrap());
}

#[test]
fn symbolic_world_snapshot_restores_overlay_state() {
    let mut cx = SymCx::new();
    let address = Address::from([0x11; 20]);
    let mut world = SymbolicWorld::default();
    world.sstore(
        address,
        SymExpr::constant(&mut cx, U256::from(1)),
        SymExpr::constant(&mut cx, U256::from(2)),
    );

    let snapshot = world.snapshot_state();
    world.sstore(
        address,
        SymExpr::constant(&mut cx, U256::from(1)),
        SymExpr::constant(&mut cx, U256::from(3)),
    );

    assert!(world.restore_snapshot(snapshot));
    assert_eq!(world.storage_len(), 1);
    assert_eq!(world.storage_value(0), Some(&SymExpr::constant(&mut cx, U256::from(2))));
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
    let word = SymExpr::var(&mut cx, "word");
    let actual = signextend_word(&mut cx, U256::ZERO, word.clone());
    let sign_mask = SymExpr::constant(&mut cx, U256::from(0x80));
    let sign_bit = SymExpr::binop(&mut cx, SymBinOp::And, word.clone(), sign_mask);
    let zero = SymExpr::zero(&mut cx);
    let sign_bit_zero = SymBoolExpr::eq(&mut cx, sign_bit, zero);
    let positive_mask = SymExpr::constant(&mut cx, U256::from(0x7f));
    let positive = SymExpr::binop(&mut cx, SymBinOp::And, word.clone(), positive_mask);
    let negative_mask = SymExpr::constant(&mut cx, !U256::from(0x7f));
    let negative = SymExpr::binop(&mut cx, SymBinOp::Or, word, negative_mask);
    let expected = SymExpr::ite(&mut cx, sign_bit_zero, positive, negative);
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

    assert_eq!(model.get("calldata_0"), Some(&U256::from(42)));
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

    assert_eq!(model.get("calldata_0"), Some(&U256::from(42)));
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
    let calldata = SymExpr::var(&mut cx, "calldata_0");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let constraints = vec![SymBoolExpr::eq(&mut cx, calldata, one)];

    let err = validate_solver_model_output(&cx, output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
fn fallback_model_finds_wrapping_arithmetic_riddle_candidate() {
    let mut cx = SymCx::new();
    let var = SymExpr::var(&mut cx, "calldata_0");
    let msg_sender = U256::from_str_radix("1804c8ab1f12e6bbf3894d4083f33e07309d1f38", 16).unwrap();
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, var.clone(), var.clone());
    let sender_word = SymExpr::constant(&mut cx, msg_sender);
    let product_below_sender =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, product, sender_word.clone());
    let above_sender = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, var.clone(), sender_word);
    let mask_800 = SymExpr::constant(&mut cx, U256::from(0x800));
    let masked_800 = SymExpr::binop(&mut cx, SymBinOp::And, var.clone(), mask_800);
    let zero = SymExpr::zero(&mut cx);
    let has_800 = SymBoolExpr::eq(&mut cx, masked_800, zero.clone()).not(&mut cx);
    let mask_10000 = SymExpr::constant(&mut cx, U256::from(0x10000));
    let masked_10000 = SymExpr::binop(&mut cx, SymBinOp::And, var, mask_10000);
    let lacks_10000 = SymBoolExpr::eq(&mut cx, masked_10000, zero);
    let constraints = vec![product_below_sender, above_sender, has_800, lacks_10000];

    let model = fallback_single_var_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn fallback_model_keeps_multi_bit_nonzero_masks_weak() {
    let mut cx = SymCx::new();
    let calldata = SymExpr::var(&mut cx, "calldata_0");
    let zero = SymExpr::zero(&mut cx);
    let mask = SymExpr::constant(&mut cx, U256::from(0xffff));
    let value = SymExpr::binop(&mut cx, SymBinOp::And, calldata.clone(), mask);
    let value_nonzero = SymBoolExpr::eq(&mut cx, value.clone(), zero).not(&mut cx);
    let calldata_limit = SymExpr::constant(&mut cx, U256::from(1) << 16);
    let calldata_bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, calldata, calldata_limit);
    let reserve = SymExpr::constant(&mut cx, U256::from(10_000));
    let value_bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value.clone(), reserve.clone());
    let remaining = SymExpr::binop(&mut cx, SymBinOp::Sub, reserve.clone(), value);
    let left_product = SymExpr::binop(&mut cx, SymBinOp::Mul, reserve, remaining);
    let factor = SymExpr::constant(&mut cx, U256::from(100_009_984));
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, left_product, factor);
    let buggy_threshold = SymExpr::constant(&mut cx, U256::from(100_000_000_000_000u64));
    let above_buggy_threshold =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, product.clone(), buggy_threshold).not(&mut cx);
    let intended_threshold = SymExpr::constant(&mut cx, U256::from(10_000_000_000_000_000u64));
    let below_intended_threshold =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, product, intended_threshold);
    let constraints = vec![
        value_nonzero,
        calldata_bound,
        value_bound,
        above_buggy_threshold,
        below_intended_threshold,
    ];

    let model = fallback_single_var_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn expression_op_simplifies_exact_arithmetic_identities() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");

    let zero = SymExpr::zero(&mut cx);
    let actual = SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), zero.clone());
    assert_eq!(actual, zero);
    let one = SymExpr::one(&mut cx);
    let actual = SymExpr::binop(&mut cx, SymBinOp::Mul, one, x.clone());
    assert_eq!(actual, x);
    let x = SymExpr::var(&mut cx, "x");
    let one = SymExpr::one(&mut cx);
    let actual = SymExpr::binop(&mut cx, SymBinOp::UDiv, x.clone(), one);
    assert_eq!(actual, x);
    let x = SymExpr::var(&mut cx, "x");
    let one = SymExpr::one(&mut cx);
    let actual = SymExpr::binop(&mut cx, SymBinOp::URem, x, one);
    let zero = SymExpr::zero(&mut cx);
    assert_eq!(actual, zero);
    let x = SymExpr::var(&mut cx, "x");
    let x_again = SymExpr::var(&mut cx, "x");
    let actual = SymExpr::binop(&mut cx, SymBinOp::Sub, x, x_again);
    let zero = SymExpr::zero(&mut cx);
    assert_eq!(actual, zero);
    let x = SymExpr::var(&mut cx, "x");
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let actual = SymExpr::binop(&mut cx, SymBinOp::And, x.clone(), max);
    assert_eq!(actual, x);
    let x = SymExpr::var(&mut cx, "x");
    let actual = SymExpr::binop(&mut cx, SymBinOp::And, x.clone(), x.clone());
    assert_eq!(actual, x);
    let mask = SymExpr::constant(&mut cx, (U256::from(1) << 160) - U256::from(1));
    let x = SymExpr::var(&mut cx, "x");
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, x, mask.clone());
    let actual = SymExpr::binop(&mut cx, SymBinOp::And, masked.clone(), mask);
    assert_eq!(actual, masked);
    let mask_value = (U256::from(1) << 128) - U256::from(1);
    let mask = SymExpr::constant(&mut cx, mask_value);
    let x = SymExpr::var(&mut cx, "x");
    let left_masked = SymExpr::binop(&mut cx, SymBinOp::And, mask.clone(), x.clone());
    let right_masked = SymExpr::binop(&mut cx, SymBinOp::And, x, mask);
    assert_eq!(left_masked, right_masked);
    let six = SymExpr::constant(&mut cx, U256::from(6));
    let seven = SymExpr::constant(&mut cx, U256::from(7));
    let actual = SymExpr::binop(&mut cx, SymBinOp::Mul, six, seven);
    let forty_two = SymExpr::constant(&mut cx, U256::from(42));
    assert_eq!(actual, forty_two);
    let x = SymExpr::var(&mut cx, "x");
    let scale = SymExpr::constant(&mut cx, U256::from(1) << 128);
    let actual = SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), scale);
    let shift = SymExpr::constant(&mut cx, U256::from(128));
    let expected = SymExpr::binop(&mut cx, SymBinOp::Shl, x, shift);
    assert_eq!(actual, expected);
    let x = SymExpr::var(&mut cx, "x");
    let divisor = SymExpr::constant(&mut cx, U256::from(1) << 128);
    let actual = SymExpr::binop(&mut cx, SymBinOp::UDiv, x.clone(), divisor);
    let shift = SymExpr::constant(&mut cx, U256::from(128));
    let expected = SymExpr::binop(&mut cx, SymBinOp::Shr, x, shift);
    assert_eq!(actual, expected);
    let x = SymExpr::var(&mut cx, "x");
    let divisor = SymExpr::constant(&mut cx, U256::from(1) << 128);
    let actual = SymExpr::binop(&mut cx, SymBinOp::URem, x.clone(), divisor);
    let mask = SymExpr::constant(&mut cx, (U256::from(1) << 128) - U256::from(1));
    let expected = SymExpr::binop(&mut cx, SymBinOp::And, x, mask);
    assert_eq!(actual, expected);
}

#[test]
fn bool_word_equality_simplifies_to_condition() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x, y);
    let word = SymExpr::bool_word(&mut cx, condition.clone());

    let one = SymExpr::one(&mut cx);
    assert_eq!(SymBoolExpr::eq(&mut cx, word.clone(), one), condition);
    let one = SymExpr::one(&mut cx);
    assert_eq!(SymBoolExpr::eq(&mut cx, one, word.clone()), condition);
    let expected = condition.not(&mut cx);
    let zero = SymExpr::zero(&mut cx);
    assert_eq!(SymBoolExpr::eq(&mut cx, word.clone(), zero), expected);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let actual = SymBoolExpr::eq(&mut cx, word, two);
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
}

#[test]
fn inverted_bool_word_equality_simplifies_to_condition() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x, y);
    let zero = SymExpr::zero(&mut cx);
    let one = SymExpr::one(&mut cx);
    let word = SymExpr::ite(&mut cx, condition.clone(), zero, one);

    let one = SymExpr::one(&mut cx);
    let actual = SymBoolExpr::eq(&mut cx, word.clone(), one);
    let expected = condition.clone().not(&mut cx);
    assert_eq!(actual, expected);
    let zero = SymExpr::zero(&mut cx);
    assert_eq!(SymBoolExpr::eq(&mut cx, word, zero), condition);
}

#[test]
fn bool_comparison_folds_reflexive_operands() {
    let mut cx = SymCx::new();

    let x = SymExpr::var(&mut cx, "x");
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), x.clone());
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x.clone(), x.clone());
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, x.clone(), x.clone());
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, x.clone(), x.clone());
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Slt, x.clone(), x.clone());
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Sgt, x.clone(), x);
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
}

#[test]
fn bool_comparison_folds_unsigned_boundaries() {
    let mut cx = SymCx::new();

    let zero = SymExpr::zero(&mut cx);
    let x = SymExpr::var(&mut cx, "x");
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, zero, x);
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let zero = SymExpr::zero(&mut cx);
    let x = SymExpr::var(&mut cx, "x");
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, zero, x);
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
    let x = SymExpr::var(&mut cx, "x");
    let zero = SymExpr::zero(&mut cx);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x, zero);
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let x = SymExpr::var(&mut cx, "x");
    let zero = SymExpr::zero(&mut cx);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, x, zero);
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let x = SymExpr::var(&mut cx, "x");
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, max, x);
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let x = SymExpr::var(&mut cx, "x");
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, max, x);
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
    let x = SymExpr::var(&mut cx, "x");
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x, max);
    let expected = SymBoolExpr::constant(&mut cx, false);
    assert_eq!(actual, expected);
    let x = SymExpr::var(&mut cx, "x");
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let actual = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, x, max);
    let expected = SymBoolExpr::constant(&mut cx, true);
    assert_eq!(actual, expected);
}

#[test]
fn exact_modular_arithmetic_handles_zero_modulus_and_wide_intermediates() {
    let mut cx = SymCx::new();
    assert_eq!(U256::MAX.add_mod(U256::from(2), U256::ZERO), U256::ZERO);
    assert_eq!(U256::MAX.mul_mod(U256::MAX, U256::ZERO), U256::ZERO);

    assert_eq!(U256::MAX.add_mod(U256::from(2), U256::MAX), U256::from(2));
    assert_eq!(U256::MAX.mul_mod(U256::MAX, U256::MAX), U256::ZERO);

    let model = symbolic_model(&mut cx, [("a".to_string(), U256::MAX)]);
    let a = SymExpr::var(&mut cx, "a");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let addmod = SymExpr::ternop(&mut cx, SymTernOp::AddMod, a, two, max);
    assert_eq!(addmod.eval_model(&model).unwrap(), U256::from(2));
    let a = SymExpr::var(&mut cx, "a");
    let a_again = SymExpr::var(&mut cx, "a");
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let mulmod = SymExpr::ternop(&mut cx, SymTernOp::MulMod, a, a_again, max);
    assert_eq!(mulmod.eval_model(&model).unwrap(), U256::ZERO);
}

#[test]
fn exact_modular_arithmetic_smt_widens_before_modulo() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let m = SymExpr::var(&mut cx, "m");
    let addmod = SymExpr::ternop(&mut cx, SymTernOp::AddMod, a, two, m).smt(&cx);
    let a = SymExpr::var(&mut cx, "a");
    let b = SymExpr::var(&mut cx, "b");
    let m = SymExpr::var(&mut cx, "m");
    let mulmod = SymExpr::ternop(&mut cx, SymTernOp::MulMod, a, b, m).smt(&cx);

    assert!(addmod.contains("((_ zero_extend 256) a)"));
    assert!(addmod.contains("bvadd"));
    assert!(addmod.contains("bvurem"));
    assert!(mulmod.contains("((_ zero_extend 256) b)"));
    assert!(mulmod.contains("bvmul"));
    assert!(mulmod.contains("((_ extract 255 0)"));
}

#[test]
fn power_of_two_modular_arithmetic_uses_low_mask() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let b = SymExpr::var(&mut cx, "b");
    let modulus = SymExpr::constant(&mut cx, U256::from(1) << 128);
    let mulmod = SymExpr::ternop(&mut cx, SymTernOp::MulMod, a, b, modulus);

    let smt = mulmod.smt(&cx);
    assert!(smt.contains("bvand"));
    assert!(smt.contains("bvmul"));
    assert!(!smt.contains("bvurem"));

    let model =
        symbolic_model(&mut cx, [("a".to_string(), U256::MAX), ("b".to_string(), U256::from(3))]);
    assert_eq!(
        mulmod.eval_model(&model).unwrap(),
        U256::MAX.mul_mod(U256::from(3), U256::from(1) << 128)
    );
}

#[test]
fn low_masked_power_of_two_division_simplifies_to_shift() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let mask = SymExpr::constant(&mut cx, (U256::from(1) << 128) - U256::from(1));
    let low = SymExpr::binop(&mut cx, SymBinOp::And, x.clone(), mask);
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Sub, x.clone(), low);
    let divisor = SymExpr::constant(&mut cx, U256::from(1) << 128);
    let actual = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, divisor);
    let shift = SymExpr::constant(&mut cx, U256::from(128));
    let expected = SymExpr::binop(&mut cx, SymBinOp::Shr, x, shift);

    assert_eq!(actual, expected);
}

#[test]
fn low_masked_words_preserve_unsigned_ordering() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let mask = SymExpr::constant(&mut cx, (U256::from(1) << 128) - U256::from(1));
    let low = SymExpr::binop(&mut cx, SymBinOp::And, x.clone(), mask);

    assert_eq!(
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), low.clone()).as_const(),
        Some(false)
    );
    assert_eq!(
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, low.clone(), x.clone()).as_const(),
        Some(false)
    );
    assert_eq!(
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, low.clone(), x.clone()).as_const(),
        Some(true)
    );
    assert_eq!(SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, x, low).as_const(), Some(true));
}

#[test]
fn exact_addmod_model_validation_rejects_wrapping_false_pass() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let addmod = SymExpr::ternop(&mut cx, SymTernOp::AddMod, a, two, max);
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let constraints = vec![SymBoolExpr::eq(&mut cx, addmod, one)];
    let output = format!("sat\n((define-fun a () (_ BitVec 256) #x{}))\n", "f".repeat(64));

    let err = validate_solver_model_output(&cx, &output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
fn solver_normalizes_udiv_zero_predicates_without_bvudiv() {
    let mut cx = SymCx::new();
    let numerator = SymExpr::var(&mut cx, "numerator");
    let denominator = SymExpr::var(&mut cx, "denominator");
    let div = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, denominator);
    let zero = SymExpr::zero(&mut cx);
    let original = SymBoolExpr::eq(&mut cx, div, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert!(!normalized.smt(&cx).contains("bvudiv"));

    for (num, den) in [
        (U256::ZERO, U256::ZERO),
        (U256::from(1), U256::ZERO),
        (U256::from(1), U256::from(2)),
        (U256::from(2), U256::from(2)),
        (U256::from(3), U256::from(2)),
        (U256::MAX, U256::MAX),
    ] {
        let model = symbolic_model(
            &mut cx,
            [("numerator".to_string(), num), ("denominator".to_string(), den)],
        );
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
    let numerator = SymExpr::var(&mut cx, "numerator");
    let denominator = SymExpr::var(&mut cx, "denominator");
    let div = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, denominator);
    let zero = SymExpr::zero(&mut cx);
    let original = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, div, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert!(!normalized.smt(&cx).contains("bvudiv"));

    for (num, den) in [
        (U256::ZERO, U256::ZERO),
        (U256::from(1), U256::ZERO),
        (U256::from(1), U256::from(2)),
        (U256::from(2), U256::from(2)),
        (U256::from(3), U256::from(2)),
        (U256::MAX, U256::MAX),
    ] {
        let model = symbolic_model(
            &mut cx,
            [("numerator".to_string(), num), ("denominator".to_string(), den)],
        );
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let ten = SymExpr::constant(&mut cx, U256::from(10));
    let a = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), ten);
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let b = SymBoolExpr::eq(&mut cx, y.clone(), three);
    let grouped = vec![
        {
            let value = SymBoolExpr::constant(&mut cx, true);
            SymBoolExpr::raw_and(&mut cx, vec![b.clone(), value, a.clone()])
        },
        a.clone(),
        SymBoolExpr::raw_and(&mut cx, vec![b.clone()]),
    ];

    let normalized = normalize_constraints_for_solver(&mut cx, &grouped);

    assert_eq!(normalized, vec![b, a]);

    let x_eq_y = SymBoolExpr::eq(&mut cx, x, y);
    let false_expr = SymBoolExpr::constant(&mut cx, false);
    let true_expr = SymBoolExpr::constant(&mut cx, true);
    let unsat = normalize_constraints_for_solver(&mut cx, &[x_eq_y, false_expr, true_expr]);
    assert_eq!(unsat, vec![SymBoolExpr::constant(&mut cx, false)]);
}

#[test]
fn solver_normalizes_erc4626_style_share_zero_predicate() {
    let mut cx = SymCx::new();
    let assets = SymExpr::var(&mut cx, "assets");
    let supply = SymExpr::var(&mut cx, "supply");
    let total_assets = SymExpr::var(&mut cx, "total_assets");
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Mul, assets, supply);
    let shares = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, total_assets);
    let zero = SymExpr::zero(&mut cx);
    let constraints = vec![SymBoolExpr::eq(&mut cx, shares, zero)];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    assert_eq!(normalized.len(), 1);
    assert!(!normalized[0].smt(&cx).contains("bvudiv"));
    assert!(normalized[0].smt(&cx).contains("bvmul"));
}

#[test]
fn expression_rebuilds_word_from_extracted_byte_terms() {
    let mut cx = SymCx::new();
    let word = SymExpr::var(&mut cx, "word");
    let mask = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, word, mask);
    let bytes = masked.clone().into_byte_exprs(&mut cx);
    let rebuilt_word = SymExpr::from_bytes(&mut cx, bytes);

    assert_eq!(rebuilt_word, masked);
}

#[test]
fn expression_rebuilds_word_from_shifted_fragments() {
    let mut cx = SymCx::new();
    let word = SymExpr::var(&mut cx, "word");
    let low_mask = SymExpr::constant(&mut cx, mask_bits(U256::MAX, 32));
    let low_bits = SymExpr::binop(&mut cx, SymBinOp::And, word.clone(), low_mask);
    let shift = SymExpr::constant(&mut cx, U256::from(32));
    let shifted = SymExpr::binop(&mut cx, SymBinOp::Shr, word.clone(), shift.clone());
    let high_mask = SymExpr::constant(&mut cx, mask_bits(U256::MAX, 224));
    let masked_high = SymExpr::binop(&mut cx, SymBinOp::And, shifted, high_mask);
    let high_bits = SymExpr::binop(&mut cx, SymBinOp::Shl, masked_high, shift);
    let rebuilt_expr = SymExpr::binop(&mut cx, SymBinOp::Or, low_bits, high_bits);

    assert_eq!(rebuilt_expr, word);
}

#[test]
fn expression_simplifies_selector_prefixed_word_fragment() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "calldata_0");
    let y = SymExpr::var(&mut cx, "calldata_1");
    let low_32 = SymExpr::constant(&mut cx, mask_bits(U256::MAX, 32));
    let low_224 = SymExpr::constant(&mut cx, mask_bits(U256::MAX, 224));
    let selector = SymExpr::constant(&mut cx, U256::from(0x771602f7u64) << 224);

    let shift_32 = SymExpr::constant(&mut cx, U256::from(32));
    let shifted_x = SymExpr::binop(&mut cx, SymBinOp::Shr, x.clone(), shift_32.clone());
    let x_high = SymExpr::binop(&mut cx, SymBinOp::And, shifted_x, low_224.clone());
    let first_word = SymExpr::binop(&mut cx, SymBinOp::Or, selector, x_high);
    let shifted_y = SymExpr::binop(&mut cx, SymBinOp::Shr, y.clone(), shift_32.clone());
    let y_high = SymExpr::binop(&mut cx, SymBinOp::And, shifted_y, low_224.clone());
    let x_low = SymExpr::binop(&mut cx, SymBinOp::And, x.clone(), low_32.clone());
    let shift_224 = SymExpr::constant(&mut cx, U256::from(224));
    let x_low_shifted = SymExpr::binop(&mut cx, SymBinOp::Shl, x_low, shift_224.clone());
    let second_word = SymExpr::binop(&mut cx, SymBinOp::Or, y_high, x_low_shifted);
    let second_word_shifted =
        SymExpr::binop(&mut cx, SymBinOp::Shr, second_word.clone(), shift_224);
    let rebuilt_low = SymExpr::binop(&mut cx, SymBinOp::And, second_word_shifted, low_32);
    let first_word_masked = SymExpr::binop(&mut cx, SymBinOp::And, first_word, low_224);
    let rebuilt_high = SymExpr::binop(&mut cx, SymBinOp::Shl, first_word_masked, shift_32.clone());
    let rebuilt = SymExpr::binop(&mut cx, SymBinOp::Or, rebuilt_low, rebuilt_high);

    assert_eq!(rebuilt, x);

    let next_low_mask = SymExpr::constant(&mut cx, mask_bits(U256::MAX, 32));
    let next_low = SymExpr::binop(&mut cx, SymBinOp::And, y.clone(), next_low_mask);
    let next_high_mask = SymExpr::constant(&mut cx, mask_bits(U256::MAX, 224));
    let next_high = SymExpr::binop(&mut cx, SymBinOp::And, second_word, next_high_mask);
    let next_high_shifted = SymExpr::binop(&mut cx, SymBinOp::Shl, next_high, shift_32);
    let rebuilt_next = SymExpr::binop(&mut cx, SymBinOp::Or, next_low, next_high_shifted);

    assert_eq!(rebuilt_next, y);
}

fn checked_mul_guard_word(cx: &mut SymCx, zero_operand: &SymExpr, expected: &SymExpr) -> SymExpr {
    let zero = SymExpr::zero(cx);
    let operand_is_zero = SymBoolExpr::eq(cx, zero_operand.clone(), zero.clone());
    let product = SymExpr::binop(cx, SymBinOp::Mul, zero_operand.clone(), expected.clone());
    let quotient = SymExpr::binop(cx, SymBinOp::UDiv, product, zero_operand.clone());
    let checked_product = SymExpr::ite(cx, operand_is_zero.clone(), zero, quotient);
    let operand_is_zero_word = SymExpr::bool_word(cx, operand_is_zero);
    let product_matches_expected = SymBoolExpr::eq(cx, checked_product, expected.clone());
    let product_matches_expected_word = SymExpr::bool_word(cx, product_matches_expected);

    SymExpr::binop(cx, SymBinOp::Or, operand_is_zero_word, product_matches_expected_word)
}

#[test]
fn solver_normalizes_checked_mul_guard_for_bounded_operands() {
    let mut cx = SymCx::new();
    let a_word = SymExpr::var(&mut cx, "a");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let a = SymExpr::binop(&mut cx, SymBinOp::And, a_word, u64_max);
    let b_word = SymExpr::var(&mut cx, "b");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let b = SymExpr::binop(&mut cx, SymBinOp::And, b_word, u64_max);
    let guard = checked_mul_guard_word(&mut cx, &a, &b);
    let zero = SymExpr::zero(&mut cx);
    let guard_is_zero = SymBoolExpr::eq(&mut cx, guard, zero);

    assert_eq!(
        normalize_bool_for_solver(&mut cx, guard_is_zero),
        SymBoolExpr::constant(&mut cx, false)
    );
}

#[test]
fn solver_normalizes_checked_mul_guard_from_path_upper_bound() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let factor = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&mut cx, &a, &factor);
    let zero = SymExpr::zero(&mut cx);
    let guard_is_false = SymBoolExpr::eq(&mut cx, guard, zero);
    let thousand = SymExpr::constant(&mut cx, U256::from(1000));
    let upper_bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, a, thousand);
    let constraints = vec![upper_bound, guard_is_false.clone()];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    assert_eq!(normalized, vec![SymBoolExpr::constant(&mut cx, false)]);

    for value in [U256::ZERO, U256::from(1), U256::from(1000)] {
        let model = symbolic_model(&mut cx, [("a".to_string(), value)]);
        assert!(!guard_is_false.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_guard() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let zero = SymExpr::zero(&mut cx);
    let a_is_zero = SymBoolExpr::eq(&mut cx, a.clone(), zero.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, a.clone(), a);
    let checked_quotient = SymExpr::ite(&mut cx, a_is_zero.clone(), zero.clone(), quotient);
    let a_is_zero_word = SymExpr::bool_word(&mut cx, a_is_zero);
    let one = SymExpr::one(&mut cx);
    let quotient_is_one = SymBoolExpr::eq(&mut cx, checked_quotient, one);
    let quotient_is_one_word = SymExpr::bool_word(&mut cx, quotient_is_one);
    let guard = SymExpr::binop(&mut cx, SymBinOp::Or, a_is_zero_word, quotient_is_one_word);
    let original = SymBoolExpr::eq(&mut cx, guard, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert_eq!(normalized, SymBoolExpr::constant(&mut cx, false));

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = symbolic_model(&mut cx, [("a".to_string(), value)]);
        assert!(!original.eval_model(&model).unwrap());
    }
}

#[test]
fn expression_normalizes_guarded_self_division_word() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let zero = SymExpr::zero(&mut cx);
    let a_is_zero = SymBoolExpr::eq(&mut cx, a.clone(), zero.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, a.clone(), a);
    let checked_quotient = SymExpr::ite(&mut cx, a_is_zero.clone(), zero, quotient);
    let a_is_nonzero = a_is_zero.not(&mut cx);
    let one = SymExpr::one(&mut cx);
    let zero = SymExpr::zero(&mut cx);
    let expected = SymExpr::ite(&mut cx, a_is_nonzero, one, zero);
    assert_eq!(checked_quotient, expected);

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = symbolic_model(&mut cx, [("a".to_string(), value)]);
        assert_eq!(
            checked_quotient.eval_model(&model).unwrap(),
            expected.eval_model(&model).unwrap()
        );
    }
}

#[test]
fn solver_does_not_invert_guarded_zero_self_division() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let zero = SymExpr::zero(&mut cx);
    let a_is_zero = SymBoolExpr::eq(&mut cx, a.clone(), zero.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, a.clone(), a);
    let mirrored = SymExpr::ite(&mut cx, a_is_zero, quotient, zero);
    let normalized = normalize_expr_for_solver(&mut cx, mirrored.clone());

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = symbolic_model(&mut cx, [("a".to_string(), value)]);
        assert_eq!(mirrored.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
    }
}

#[test]
fn solver_normalizes_guarded_self_division_add_overflow_guard() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let zero = SymExpr::zero(&mut cx);
    let a_is_zero = SymBoolExpr::eq(&mut cx, a.clone(), zero.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, a.clone(), a);
    let checked_quotient = SymExpr::ite(&mut cx, a_is_zero, zero, quotient);
    let one = SymExpr::one(&mut cx);
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, one, checked_quotient.clone());
    let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, checked_quotient, sum);

    assert_eq!(
        normalize_bool_for_solver(&mut cx, condition),
        SymBoolExpr::constant(&mut cx, false)
    );
}

#[test]
fn solver_normalizes_checked_mul_guard_with_context_bound() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let scale = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&mut cx, &a, &scale);
    let zero = SymExpr::zero(&mut cx);
    let a_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, a.clone(), zero.clone());
    let thousand = SymExpr::constant(&mut cx, U256::from(1000));
    let above_thousand = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, a, thousand).not(&mut cx);
    let guard_is_zero = SymBoolExpr::eq(&mut cx, guard, zero);
    let constraints = vec![a_positive, above_thousand, guard_is_zero];

    assert_eq!(
        normalize_constraints_for_solver(&mut cx, &constraints),
        vec![SymBoolExpr::constant(&mut cx, false)]
    );
}

#[test]
fn solver_normalizes_mask_equality_with_context_bound() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let mask = (U256::from(1) << 160) - U256::from(1);
    let mask_expr = SymExpr::constant(&mut cx, mask);
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, a.clone(), mask_expr);
    let same_value = SymBoolExpr::eq(&mut cx, masked, a.clone());
    let upper = SymExpr::constant(&mut cx, U256::from(1) << 160);
    let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, a, upper);

    assert_eq!(
        normalize_constraints_for_solver(&mut cx, &[bounded.clone(), same_value]),
        vec![bounded]
    );
}

#[test]
fn solver_normalizes_negated_mask_equality_with_context_bound() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let mask = (U256::from(1) << 160) - U256::from(1);
    let mask_expr = SymExpr::constant(&mut cx, mask);
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, a.clone(), mask_expr);
    let different_value = SymBoolExpr::eq(&mut cx, masked, a.clone()).not(&mut cx);
    let upper = SymExpr::constant(&mut cx, U256::from(1) << 160);
    let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, a, upper);

    assert_eq!(
        normalize_constraints_for_solver(&mut cx, &[bounded, different_value]),
        vec![SymBoolExpr::constant(&mut cx, false)]
    );
}

#[test]
fn solver_normalizes_unsigned_zero_comparisons() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let shift = SymExpr::constant(&mut cx, U256::from(1));
    let shifted = SymExpr::binop(&mut cx, SymBinOp::Shl, x.clone(), shift);
    let zero = SymExpr::zero(&mut cx);
    let positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, zero, shifted.clone());
    let zero = SymExpr::zero(&mut cx);
    let shifted_zero = SymBoolExpr::eq(&mut cx, shifted.clone(), zero);

    assert_eq!(normalize_bool_for_solver(&mut cx, positive), shifted_zero.clone().not(&mut cx));

    let zero = SymExpr::zero(&mut cx);
    let positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, zero, shifted);
    let not_positive = positive.not(&mut cx);
    assert_eq!(normalize_bool_for_solver(&mut cx, not_positive), shifted_zero);

    let one = SymExpr::constant(&mut cx, U256::from(1));
    let below_one = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), one);
    let zero = SymExpr::zero(&mut cx);
    let x_zero = SymBoolExpr::eq(&mut cx, x.clone(), zero);
    assert_eq!(normalize_bool_for_solver(&mut cx, below_one), x_zero);

    let one = SymExpr::constant(&mut cx, U256::from(1));
    let at_least_one = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, one, x);
    assert_eq!(normalize_bool_for_solver(&mut cx, at_least_one), x_zero.not(&mut cx));
}

#[test]
fn solver_does_not_context_normalize_checked_mul_guard_without_tight_bound() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let scale = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&mut cx, &a, &scale);
    let zero = SymExpr::zero(&mut cx);
    let guard_is_zero = SymBoolExpr::eq(&mut cx, guard, zero);

    assert_ne!(
        normalize_constraints_for_solver(&mut cx, std::slice::from_ref(&guard_is_zero)),
        vec![SymBoolExpr::constant(&mut cx, false)]
    );
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, a, max);
    assert_ne!(
        normalize_constraints_for_solver(&mut cx, &[bound, guard_is_zero.clone()]),
        vec![SymBoolExpr::constant(&mut cx, false)]
    );

    let model = symbolic_model(&mut cx, [("a".to_string(), U256::MAX)]);
    assert!(guard_is_zero.eval_model(&model).unwrap());
}

#[test]
fn solver_does_not_normalize_unbounded_checked_mul_guard_to_tautology() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let b = SymExpr::var(&mut cx, "b");
    let guard = checked_mul_guard_word(&mut cx, &a, &b);
    let zero = SymExpr::zero(&mut cx);
    let original = SymBoolExpr::eq(&mut cx, guard, zero);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert_ne!(normalized, SymBoolExpr::constant(&mut cx, false));

    let model =
        symbolic_model(&mut cx, [("a".to_string(), U256::MAX), ("b".to_string(), U256::from(2))]);
    assert!(original.eval_model(&model).unwrap());
    assert_eq!(original.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
}

#[test]
fn solver_does_not_normalize_checked_mul_guard_when_path_bound_can_overflow() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let factor = SymExpr::constant(&mut cx, U256::from(2));
    let guard = checked_mul_guard_word(&mut cx, &a, &factor);
    let zero = SymExpr::zero(&mut cx);
    let original = SymBoolExpr::eq(&mut cx, guard, zero);
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, a, max);
    let normalized = normalize_constraints_for_solver(&mut cx, &[bound, original.clone()]);

    assert!(!matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)));

    let model = symbolic_model(&mut cx, [("a".to_string(), U256::MAX)]);
    assert!(original.eval_model(&model).unwrap());
    assert!(normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn solver_normalizes_checked_add_overflow_guard_for_bounded_operands() {
    let mut cx = SymCx::new();
    let a_raw = SymExpr::var(&mut cx, "a");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let a = SymExpr::binop(&mut cx, SymBinOp::And, a_raw, u64_max);
    let b_raw = SymExpr::var(&mut cx, "b");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let b_masked = SymExpr::binop(&mut cx, SymBinOp::And, b_raw, u64_max);
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Mul, b_masked, a.clone());
    let denominator_raw = SymExpr::var(&mut cx, "denominator");
    let high_mask = SymExpr::constant(&mut cx, U256::MAX >> 64);
    let denominator = SymExpr::binop(&mut cx, SymBinOp::And, denominator_raw, high_mask);
    let b = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, denominator);
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, a.clone(), b);
    let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, a, sum);

    assert_eq!(
        normalize_bool_for_solver(&mut cx, condition),
        SymBoolExpr::constant(&mut cx, false)
    );
}

#[test]
fn solver_does_not_normalize_unbounded_checked_add_overflow_guard() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let b = SymExpr::var(&mut cx, "b");
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, a.clone(), b);
    let original = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, a, sum);
    let normalized = normalize_bool_for_solver(&mut cx, original.clone());

    assert_ne!(normalized, SymBoolExpr::constant(&mut cx, false));

    let model =
        symbolic_model(&mut cx, [("a".to_string(), U256::MAX), ("b".to_string(), U256::from(1))]);
    assert!(original.eval_model(&model).unwrap());
    assert_eq!(original.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
}

#[test]
fn solver_detects_monotonic_product_contradiction() {
    let mut cx = SymCx::new();
    let ink_word = SymExpr::var(&mut cx, "ink");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let ink = SymExpr::binop(&mut cx, SymBinOp::And, ink_word, u64_max);
    let art_word = SymExpr::var(&mut cx, "art");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let art = SymExpr::binop(&mut cx, SymBinOp::And, art_word, u64_max);
    let spot_word = SymExpr::var(&mut cx, "spot");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let spot = SymExpr::binop(&mut cx, SymBinOp::And, spot_word, u64_max);
    let rate_word = SymExpr::var(&mut cx, "rate");
    let u64_max = SymExpr::constant(&mut cx, U256::from(u64::MAX));
    let rate = SymExpr::binop(&mut cx, SymBinOp::And, rate_word, u64_max);
    let zero = SymExpr::zero(&mut cx);
    let ink_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, ink.clone(), zero.clone());
    let art_above_ink = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, art.clone(), ink.clone());
    let spot_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, spot.clone(), zero);
    let rate_above_spot = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, rate.clone(), spot.clone());
    let left_product = SymExpr::binop(&mut cx, SymBinOp::Mul, ink, spot);
    let right_product = SymExpr::binop(&mut cx, SymBinOp::Mul, art, rate);
    let wrapping =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, left_product, right_product).not(&mut cx);
    let constraints = vec![ink_positive, art_above_ink, spot_positive, rate_above_spot, wrapping];

    assert!(product_monotonic_unsat(&mut cx, &constraints));
}

#[test]
fn solver_does_not_prune_wrapping_product_inequality() {
    let mut cx = SymCx::new();
    let ink = SymExpr::var(&mut cx, "ink");
    let art = SymExpr::var(&mut cx, "art");
    let spot = SymExpr::var(&mut cx, "spot");
    let rate = SymExpr::var(&mut cx, "rate");
    let zero = SymExpr::zero(&mut cx);
    let ink_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, ink.clone(), zero.clone());
    let art_above_ink = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, art.clone(), ink.clone());
    let spot_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, spot.clone(), zero);
    let rate_above_spot = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, rate.clone(), spot.clone());
    let left_product = SymExpr::binop(&mut cx, SymBinOp::Mul, ink, spot);
    let right_product = SymExpr::binop(&mut cx, SymBinOp::Mul, art, rate);
    let wrapping =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, left_product, right_product).not(&mut cx);
    let constraints = vec![ink_positive, art_above_ink, spot_positive, rate_above_spot, wrapping];

    assert!(!product_monotonic_unsat(&mut cx, &constraints));

    let model = symbolic_model(
        &mut cx,
        [
            ("ink".to_string(), U256::MAX - U256::from(2)),
            ("spot".to_string(), U256::MAX - U256::from(2)),
            ("art".to_string(), U256::MAX - U256::from(1)),
            ("rate".to_string(), U256::MAX - U256::from(1)),
        ],
    );
    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_detection_flags_symbolic_mul_div_and_mod() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");

    assert!(SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), y.clone()).contains_hard_arith());
    assert!(SymExpr::binop(&mut cx, SymBinOp::UDiv, x.clone(), y.clone()).contains_hard_arith());
    assert!(SymExpr::binop(&mut cx, SymBinOp::URem, x.clone(), y).contains_hard_arith());
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let mul = SymExpr::binop(&mut cx, SymBinOp::Mul, x, one);
    assert!(!mul.contains_hard_arith());
}

#[test]
fn hard_arithmetic_fallback_finds_multi_variable_candidate() {
    let mut cx = SymCx::new();
    let first = SymExpr::var(&mut cx, "first");
    let donation = SymExpr::var(&mut cx, "donation");
    let second = SymExpr::var(&mut cx, "second");
    let denominator = SymExpr::binop(&mut cx, SymBinOp::Add, first.clone(), donation.clone());
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Mul, second.clone(), first.clone());
    let shares = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, denominator);
    let zero = SymExpr::zero(&mut cx);
    let first_nonzero = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, first, zero.clone());
    let donation_nonzero = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, donation, zero.clone());
    let second_nonzero = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, second, zero.clone());
    let shares_zero = SymBoolExpr::eq(&mut cx, shares, zero);
    let constraints = vec![first_nonzero, donation_nonzero, second_nonzero, shares_zero];

    let model = hard_arith_fallback_model(&cx, &constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_finds_wrapping_product_inequality_candidate() {
    let mut cx = SymCx::new();
    let ink = SymExpr::var(&mut cx, "ink");
    let art = SymExpr::var(&mut cx, "art");
    let spot = SymExpr::var(&mut cx, "spot");
    let rate = SymExpr::var(&mut cx, "rate");
    let zero = SymExpr::zero(&mut cx);
    let ink_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, ink.clone(), zero.clone());
    let art_above_ink = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, art.clone(), ink.clone());
    let spot_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, spot.clone(), zero);
    let rate_above_spot = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, rate.clone(), spot.clone());
    let left_product = SymExpr::binop(&mut cx, SymBinOp::Mul, ink, spot);
    let right_product = SymExpr::binop(&mut cx, SymBinOp::Mul, art, rate);
    let wrapping =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, left_product, right_product).not(&mut cx);
    let constraints = vec![ink_positive, art_above_ink, spot_positive, rate_above_spot, wrapping];

    let model = hard_arith_fallback_model(&cx, &constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_finds_exact_mulmod_wide_intermediate_candidate() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let zero = SymExpr::zero(&mut cx);
    let positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, a.clone(), zero.clone());
    let max = SymExpr::constant(&mut cx, U256::MAX);
    let product = SymExpr::ternop(&mut cx, SymTernOp::MulMod, a.clone(), a, max);
    let exact = SymBoolExpr::eq(&mut cx, product, zero);
    let constraints = vec![positive, exact];

    let model = hard_arith_fallback_model(&cx, &constraints).unwrap();

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    assert_eq!(model_value(&cx, &model, "a"), Some(U256::MAX));
}

#[test]
fn hard_arithmetic_fallback_rejects_unvalidated_partial_model() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let b = SymExpr::var(&mut cx, "b");
    let c = SymExpr::var(&mut cx, "c");
    let d = SymExpr::var(&mut cx, "d");
    let e = SymExpr::var(&mut cx, "e");
    let f = SymExpr::var(&mut cx, "f");
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, a, b);
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let product_eq_one = SymBoolExpr::eq(&mut cx, product, one);
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let c_eq_three = SymBoolExpr::eq(&mut cx, c, three);
    let four = SymExpr::constant(&mut cx, U256::from(4));
    let d_eq_four = SymBoolExpr::eq(&mut cx, d, four);
    let five = SymExpr::constant(&mut cx, U256::from(5));
    let e_eq_five = SymBoolExpr::eq(&mut cx, e, five);
    let six = SymExpr::constant(&mut cx, U256::from(6));
    let f_eq_six = SymBoolExpr::eq(&mut cx, f, six);
    let constraints = vec![product_eq_one, c_eq_three, d_eq_four, e_eq_five, f_eq_six];

    let Some(model) = hard_arith_fallback_model(&cx, &constraints) else { return };

    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn hard_arithmetic_fallback_skips_symbolic_hashes() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let hash_symbol = cx.intern("hash");
    let hash = SymExpr::hash_symbol(&mut cx, hash_symbol, "sha256", vec![x.clone()]);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x, hash);
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let constraints = vec![SymBoolExpr::eq(&mut cx, product, one)];

    assert!(hard_arith_fallback_model(&cx, &constraints).is_none());
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
fn is_sat_uses_single_var_witness_before_solver() {
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("single-var-is-sat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let value = SymExpr::var(&mut cx, "calldata_0");
    let limit = SymExpr::constant(&mut cx, U256::from(10));
    let constraints = vec![SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value, limit)];

    assert!(solver.is_sat(&mut cx, &constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(counted_solver_invocations(&marker), 0);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn gasleft_can_use_single_var_witness_before_solver() {
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("gasleft-single-var");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let gas = SymExpr::gas_left(&mut cx, 0);
    let limit = SymExpr::constant(&mut cx, U256::from(10));
    let constraints = vec![SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, gas, limit)];

    assert!(solver.is_sat(&mut cx, &constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(counted_solver_invocations(&marker), 0);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn gasleft_fails_at_smt_emission() {
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("gasleft-smt");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let gas = SymExpr::gas_left(&mut cx, 0);
    let x = SymExpr::var(&mut cx, "x");
    let constraints = vec![SymBoolExpr::eq(&mut cx, gas, x)];

    let err = solver.is_sat(&mut cx, &constraints).unwrap_err();
    assert!(matches!(err, SymbolicError::Unsupported("GAS/gasleft() not modeled")));

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 1);
    assert_eq!(counted_solver_invocations(&marker), 0);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn model_uses_single_var_witness_before_solver() {
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("single-var-model");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let value = SymExpr::var(&mut cx, "calldata_0");
    let zero = SymExpr::zero(&mut cx);
    let not_zero = SymBoolExpr::eq(&mut cx, value.clone(), zero).not(&mut cx);
    let limit = SymExpr::constant(&mut cx, U256::from(10));
    let below_limit = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value, limit);
    let constraints = vec![not_zero, below_limit];

    let model = solver.model(&mut cx, &constraints).unwrap();
    assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(stats.model_queries, 1);
    assert_eq!(counted_solver_invocations(&marker), 0);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
fn is_sat_uses_validated_hard_arithmetic_fallback_before_solver() {
    let mut cx = SymCx::new();
    let marker = portfolio_test_marker("hard-arith-is-sat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let zero = SymExpr::zero(&mut cx);
    let x_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x.clone(), zero.clone());
    let y_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, y.clone(), zero);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x, y);
    let four = SymExpr::constant(&mut cx, U256::from(4));
    let product_eq_four = SymBoolExpr::eq(&mut cx, product, four);
    let constraints = vec![x_positive, y_positive, product_eq_four];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    let model = hard_arith_fallback_model(&cx, &normalized).unwrap();

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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let zero = SymExpr::zero(&mut cx);
    let x_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x.clone(), zero.clone());
    let y_positive = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, y.clone(), zero);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x, y);
    let four = SymExpr::constant(&mut cx, U256::from(4));
    let product_eq_four = SymBoolExpr::eq(&mut cx, product, four);
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let zero = SymExpr::zero(&mut cx);
    let x_is_zero = SymBoolExpr::eq(&mut cx, x.clone(), zero);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x, y);
    let one = SymExpr::one(&mut cx);
    let product_eq_one = SymBoolExpr::eq(&mut cx, product, one);
    let constraints = vec![x_is_zero, product_eq_one];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    assert!(hard_arith_fallback_model(&cx, &normalized).is_none());
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let constraints = vec![
        SymBoolExpr::constant(&mut cx, true),
        SymBoolExpr::eq(&mut cx, x.clone(), one.clone()),
        SymBoolExpr::eq(&mut cx, y.clone(), two.clone()),
    ];
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, x.clone(), y.clone());
    let sum_eq_three = SymBoolExpr::eq(&mut cx, sum, three.clone());
    let x_eq_one = SymBoolExpr::eq(&mut cx, x.clone(), one.clone());
    let constraints = vec![SymBoolExpr::and(&mut cx, vec![sum_eq_three, x_eq_one])];
    let one_eq_x = SymBoolExpr::eq(&mut cx, one, x);
    let x_again = SymExpr::var(&mut cx, "x");
    let reordered_sum = SymExpr::binop(&mut cx, SymBinOp::Add, y, x_again);
    let three_eq_sum = SymBoolExpr::eq(&mut cx, three, reordered_sum);
    let reordered_constraints = vec![SymBoolExpr::and(&mut cx, vec![one_eq_x, three_eq_sum])];

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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let duplicated = vec![x_eq_one.clone(), y_eq_two.clone(), x_eq_one.clone()];
    let deduplicated = vec![x_eq_one, y_eq_two];

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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let duplicated = vec![SymBoolExpr::raw_and(
        &mut cx,
        vec![x_eq_one.clone(), y_eq_two.clone(), x_eq_one.clone()],
    )];
    let deduplicated = vec![x_eq_one, y_eq_two];

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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let z = SymExpr::var(&mut cx, "z");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let a = SymBoolExpr::eq(&mut cx, x, one);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let b = SymBoolExpr::eq(&mut cx, y, two);
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let c = SymBoolExpr::eq(&mut cx, z, three);
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");

    let ugt = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x.clone(), y.clone());
    assert!(solver.is_sat(&mut cx, &[ugt]).unwrap());
    let ult = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, y.clone(), x.clone());
    assert!(solver.is_sat(&mut cx, &[ult]).unwrap());
    let uge = SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, x.clone(), y.clone());
    assert!(solver.is_sat(&mut cx, &[uge]).unwrap());
    let ule = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, y.clone(), x.clone());
    assert!(solver.is_sat(&mut cx, &[ule]).unwrap());
    let sgt = SymBoolExpr::cmp(&mut cx, SymCmpOp::Sgt, x.clone(), y.clone());
    assert!(solver.is_sat(&mut cx, &[sgt]).unwrap());
    let slt = SymBoolExpr::cmp(&mut cx, SymCmpOp::Slt, y, x);
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let ten = SymExpr::constant(&mut cx, U256::from(10));
    let x_lt_ten = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), ten);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let base = vec![x_lt_ten, y_eq_two];
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let condition = SymBoolExpr::eq(&mut cx, x, one);
    let not_condition = condition.clone().not(&mut cx);
    let mut condition_path = base.clone();
    condition_path.push(condition);
    let mut complement_path = base.clone();
    complement_path.push(not_condition);

    assert!(solver.is_sat(&mut cx, &base).unwrap());
    assert!(!solver.is_sat_branch(&mut cx, &condition_path).unwrap());
    assert!(solver.is_sat_branch(&mut cx, &complement_path).unwrap());

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
    let x = SymExpr::var(&mut cx, "x");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let lower = SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, x.clone(), two);
    let upper = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, x.clone(), one.clone());
    let base = SymBoolExpr::and(&mut cx, vec![lower, upper]);
    let condition = SymBoolExpr::eq(&mut cx, x, one);

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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let constraints = vec![x_eq_one, y_eq_two];

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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let constraints = vec![
        SymBoolExpr::constant(&mut cx, true),
        SymBoolExpr::eq(&mut cx, x.clone(), one.clone()),
        SymBoolExpr::eq(&mut cx, y.clone(), two.clone()),
    ];
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let reordered_constraints = vec![y_eq_two, x_eq_one];

    let model = solver.model(&mut cx, &constraints).unwrap();
    assert_eq!(model_value(&cx, &model, "x"), Some(U256::from(1)));
    let model = solver.model(&mut cx, &reordered_constraints).unwrap();
    assert_eq!(model_value(&cx, &model, "y"), Some(U256::from(2)));

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
        #x0000000000000000000000000000000000000000000000000000000000000001)\n\
        (define-fun y () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000002))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let constraints = vec![x_eq_one, y_eq_two];

    let model = solver.model(&mut cx, &constraints).unwrap();
    assert_eq!(model_value(&cx, &model, "x"), Some(U256::from(1)));
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
    let x = SymExpr::var(&mut cx, "x");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let constraint = SymBoolExpr::eq(&mut cx, x, one);
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
    let x = SymExpr::var(&mut cx, "x");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let y = SymExpr::var(&mut cx, "y");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let z = SymExpr::var(&mut cx, "z");
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let z_eq_three = SymBoolExpr::eq(&mut cx, z, three);
    let unsat_subset = vec![x_eq_one.clone(), y_eq_two.clone()];

    assert!(!solver.is_sat(&mut cx, &unsat_subset).unwrap());
    assert!(!solver.is_sat(&mut cx, &[x_eq_one, y_eq_two, z_eq_three]).unwrap());

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
    let x = SymExpr::var(&mut cx, "x");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let y = SymExpr::var(&mut cx, "y");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let z = SymExpr::var(&mut cx, "z");
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let z_eq_three = SymBoolExpr::eq(&mut cx, z, three);
    let sat_subset = vec![x_eq_one.clone(), y_eq_two.clone()];

    assert!(solver.is_sat(&mut cx, &sat_subset).unwrap());
    assert!(matches!(
        solver.is_sat(&mut cx, &[x_eq_one, y_eq_two, z_eq_three]),
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
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let x_eq_one = SymBoolExpr::eq(&mut cx, x, one);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let y_eq_two = SymBoolExpr::eq(&mut cx, y, two);
    let constraints = vec![x_eq_one, y_eq_two];

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

    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let zero = SymExpr::zero(&mut cx);
    let first = vec![
        SymBoolExpr::eq(&mut cx, x.clone(), one),
        SymBoolExpr::eq(&mut cx, y.clone(), zero.clone()),
    ];
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let second = vec![SymBoolExpr::eq(&mut cx, x, two), SymBoolExpr::eq(&mut cx, y, zero)];

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

    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let one = SymExpr::constant(&mut cx, U256::from(1));
    let zero = SymExpr::zero(&mut cx);
    let first = vec![
        SymBoolExpr::eq(&mut cx, x.clone(), one),
        SymBoolExpr::eq(&mut cx, y.clone(), zero.clone()),
    ];
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let second = vec![SymBoolExpr::eq(&mut cx, x, two), SymBoolExpr::eq(&mut cx, y, zero)];

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
        let calldata = SymExpr::var(&mut cx, "calldata_0");
        let one = SymExpr::constant(&mut cx, U256::from(1));
        let calldata_constraint = SymBoolExpr::eq(&mut cx, calldata, one);
        let portfolio_query = SymExpr::var(&mut cx, &format!("portfolio_query_{query}"));
        let zero = SymExpr::zero(&mut cx);
        let query_constraint = SymBoolExpr::eq(&mut cx, portfolio_query, zero);
        let constraints = vec![calldata_constraint, query_constraint];
        let model = solver.model(&mut cx, &constraints).unwrap();
        assert_eq!(model_value(&cx, &model, "calldata_0"), Some(U256::from(1)));
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
    let calldata = SymExpr::var(&mut cx, "calldata_0");
    let byte = calldata.extracted_byte(&mut cx, 0);
    let shift = SymExpr::constant(&mut cx, U256::from(248));
    let shifted = SymExpr::binop(&mut cx, SymBinOp::Shl, byte, shift);
    let zero = SymExpr::zero(&mut cx);
    let shifted_zero = SymBoolExpr::eq(&mut cx, shifted.clone(), zero);
    let other = SymExpr::var(&mut cx, "other");
    let shifted_not_other = SymBoolExpr::eq(&mut cx, shifted, other).not(&mut cx);
    let constraints = vec![shifted_zero, shifted_not_other];

    solver.capture_diagnostics();

    assert!(solver.is_sat(&mut cx, &constraints).unwrap());
    let diagnostics = solver.take_diagnostics().unwrap();

    assert!(diagnostics.contains("(define-fun __sym_expr_"));
    assert!(
        diagnostics
            .lines()
            .any(|line| line.starts_with("(assert (= ") && line.contains("__sym_expr_"))
    );
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
