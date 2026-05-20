use super::{abi::*, runtime::*, *};

/// Regression coverage for `empty_state`.
fn empty_state() -> PathState {
    PathState::new(
        Address::ZERO,
        Address::ZERO,
        U256::ZERO,
        SymbolicCalldata {
            size: 4,
            bytes: vec![SymWord::zero(); 4],
            inputs: Vec::new(),
            constraints: Vec::new(),
        },
        false,
    )
}

/// Regression coverage for `add_words`.
fn add_words(left: SymWord, right: SymWord) -> SymWord {
    SymWord::Expr(expr_add(left.into_expr(), right.into_expr()))
}

/// Regression coverage for `precompile_address`.
fn precompile_address(index: u8) -> Address {
    let mut bytes = [0u8; 20];
    bytes[19] = index;
    Address::from(bytes)
}

#[test]
/// Regression coverage for `binary_helpers_use_evm_operand_order`.
fn binary_helpers_use_evm_operand_order() {
    let mut state = empty_state();
    state.stack.push(SymWord::Concrete(U256::from(2))).unwrap();
    state.stack.push(SymWord::Concrete(U256::from(10))).unwrap();

    state.bin_word(|a, b| a.wrapping_sub(b), ExprOp::Sub).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymWord::Concrete(U256::from(8)));
}

#[test]
/// Regression coverage for `comparison_helpers_use_evm_operand_order`.
fn comparison_helpers_use_evm_operand_order() {
    let mut state = empty_state();
    state.stack.push(SymWord::Concrete(U256::from(2))).unwrap();
    state.stack.push(SymWord::Concrete(U256::from(10))).unwrap();

    state.cmp_word(|a, b| a < b, BoolExprOp::Ult).unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymWord::Concrete(U256::ZERO));
}

#[test]
/// Regression coverage for `exp_helper_uses_evm_operand_order`.
fn exp_helper_uses_evm_operand_order() {
    let mut state = empty_state();
    state.stack.push(SymWord::Concrete(U256::ZERO)).unwrap();
    state.stack.push(SymWord::Concrete(U256::from(0x100))).unwrap();

    state.exp_word().unwrap();

    assert_eq!(state.stack.pop().unwrap(), SymWord::Concrete(U256::from(1)));
}

#[test]
/// Regression coverage for `exp_helper_expands_symbolic_base_for_bounded_concrete_exponent`.
fn exp_helper_expands_symbolic_base_for_bounded_concrete_exponent() {
    let mut state = empty_state();
    state.stack.push(SymWord::Concrete(U256::from(16))).unwrap();
    state.stack.push(SymWord::Expr(Expr::Var("base".to_string()))).unwrap();

    state.exp_word().unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        model_word(&result, &BTreeMap::from([("base".to_string(), U256::from(2))])).unwrap(),
        U256::from(65536)
    );
}

#[test]
/// Regression coverage for `exp_helper_expands_bounded_symbolic_exponent`.
fn exp_helper_expands_bounded_symbolic_exponent() {
    let mut state = empty_state();
    let exponent = SymWord::Expr(Expr::Var("exponent".to_string()));
    state.constraints.push(BoolExpr::cmp(
        BoolExprOp::Ule,
        exponent.clone().into_expr(),
        Expr::Const(U256::from(5)),
    ));
    state.stack.push(exponent).unwrap();
    state.stack.push(SymWord::Concrete(U256::from(3))).unwrap();

    state.exp_word().unwrap();

    let result = state.stack.pop().unwrap();
    assert_eq!(
        model_word(&result, &BTreeMap::from([("exponent".to_string(), U256::from(5))])).unwrap(),
        U256::from(243)
    );
}

#[test]
/// Regression coverage for `shift_helpers_accept_symbolic_amounts`.
fn shift_helpers_accept_symbolic_amounts() {
    let mut shl = empty_state();
    shl.stack.push(SymWord::Concrete(U256::from(1))).unwrap();
    shl.stack.push(SymWord::Expr(Expr::Var("shift".to_string()))).unwrap();
    shl.shift_word(ShiftKind::Shl).unwrap();
    let shifted = shl.stack.pop().unwrap();

    assert_eq!(
        model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(5))])).unwrap(),
        U256::from(32)
    );
    assert_eq!(
        model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(256))])).unwrap(),
        U256::ZERO
    );

    let mut shr = empty_state();
    shr.stack.push(SymWord::Concrete(U256::from(1) << 255)).unwrap();
    shr.stack.push(SymWord::Expr(Expr::Var("shift".to_string()))).unwrap();
    shr.shift_word(ShiftKind::Shr).unwrap();
    let shifted = shr.stack.pop().unwrap();

    assert_eq!(
        model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(255))])).unwrap(),
        U256::from(1)
    );
    assert_eq!(
        model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(256))])).unwrap(),
        U256::ZERO
    );

    let mut sar = empty_state();
    sar.stack.push(SymWord::Concrete(U256::MAX)).unwrap();
    sar.stack.push(SymWord::Expr(Expr::Var("shift".to_string()))).unwrap();
    sar.shift_word(ShiftKind::Sar).unwrap();
    let shifted = sar.stack.pop().unwrap();

    assert_eq!(
        model_word(&shifted, &BTreeMap::from([("shift".to_string(), U256::from(300))])).unwrap(),
        U256::MAX
    );
}

#[test]
/// Regression coverage for `symbolic_division_guards_zero_divisor`.
fn symbolic_division_guards_zero_divisor() {
    let mut state = empty_state();
    state.stack.push(SymWord::Expr(Expr::Var("den".to_string()))).unwrap();
    state.stack.push(SymWord::Expr(Expr::Var("num".to_string()))).unwrap();

    state
        .bin_word_div_zero_guard(|a, b| if b.is_zero() { U256::ZERO } else { a / b }, ExprOp::UDiv)
        .unwrap();

    assert_eq!(
        state.stack.pop().unwrap(),
        SymWord::Expr(Expr::Ite(
            Box::new(BoolExpr::eq(Expr::Var("den".to_string()), Expr::Const(U256::ZERO))),
            Box::new(Expr::Const(U256::ZERO)),
            Box::new(Expr::op(
                ExprOp::UDiv,
                Expr::Var("num".to_string()),
                Expr::Var("den".to_string())
            )),
        ))
    );
}

#[test]
/// Regression coverage for `symbolic_byte_extracts_with_concrete_index`.
fn symbolic_byte_extracts_with_concrete_index() {
    assert_eq!(
        byte_word(U256::from(0), SymWord::Expr(Expr::Var("word".to_string()))),
        SymWord::Expr(Expr::op(
            ExprOp::And,
            Expr::op(ExprOp::Shr, Expr::Var("word".to_string()), Expr::Const(U256::from(248))),
            Expr::Const(U256::from(0xff))
        ))
    );
}

#[test]
/// Regression coverage for `symbolic_byte_extracts_with_symbolic_index`.
fn symbolic_byte_extracts_with_symbolic_index() {
    let word = U256::from_be_bytes(core::array::from_fn::<_, 32, _>(|idx| idx as u8));
    let byte =
        byte_word_dynamic(SymWord::Expr(Expr::Var("index".to_string())), SymWord::Concrete(word));

    let in_range = BTreeMap::from([("index".to_string(), U256::from(9))]);
    assert_eq!(model_word(&byte, &in_range).unwrap(), U256::from(9));

    let out_of_range = BTreeMap::from([("index".to_string(), U256::from(32))]);
    assert_eq!(model_word(&byte, &out_of_range).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `symbolic_signextend_accepts_symbolic_index`.
fn symbolic_signextend_accepts_symbolic_index() {
    let value = SymWord::Concrete(U256::from(0x80));
    let extended = signextend_word_dynamic(SymWord::Expr(Expr::Var("index".to_string())), value);

    let zero_index = BTreeMap::from([("index".to_string(), U256::ZERO)]);
    assert_eq!(model_word(&extended, &zero_index).unwrap(), U256::MAX - U256::from(0x7f));

    let one_index = BTreeMap::from([("index".to_string(), U256::from(1))]);
    assert_eq!(model_word(&extended, &one_index).unwrap(), U256::from(0x80));
}

#[test]
/// Regression coverage for `symbolic_byte_preserves_concrete_packed_selector_bytes`.
fn symbolic_byte_preserves_concrete_packed_selector_bytes() {
    let selector = U256::from(0x12345678);
    let packed = SymWord::Expr(Expr::op(
        ExprOp::Or,
        Expr::op(ExprOp::Shl, Expr::Const(selector), Expr::Const(U256::from(224))),
        Expr::op(ExprOp::Shr, Expr::Var("arg".to_string()), Expr::Const(U256::from(32))),
    ));

    assert_eq!(byte_word(U256::from(0), packed.clone()), SymWord::Concrete(U256::from(0x12)));
    assert_eq!(byte_word(U256::from(1), packed.clone()), SymWord::Concrete(U256::from(0x34)));
    assert_eq!(byte_word(U256::from(2), packed.clone()), SymWord::Concrete(U256::from(0x56)));
    assert_eq!(byte_word(U256::from(3), packed), SymWord::Concrete(U256::from(0x78)));
}

#[test]
/// Regression coverage for `word_reassembly_preserves_split_symbolic_word`.
fn word_reassembly_preserves_split_symbolic_word() {
    let original =
        Expr::op(ExprOp::Add, Expr::Var("value".to_string()), Expr::Const(U256::from(1)));
    let bytes = word_bytes(SymWord::Expr(original.clone()));

    assert_eq!(word_from_bytes(bytes), SymWord::Expr(original));
}

#[test]
/// Regression coverage for `symbolic_address_aliases_match_abi_encoded_address_words`.
fn symbolic_address_aliases_match_abi_encoded_address_words() {
    let source = Expr::Var("beneficiary".to_string());
    let masked =
        Expr::op(ExprOp::And, source.clone(), Expr::Const((U256::from(1) << 160) - U256::from(1)));
    let mut encoded = vec![SymWord::zero(); 12];
    encoded.extend((12..32).map(|idx| byte_word(U256::from(idx), SymWord::Expr(masked.clone()))));
    let reassembled = word_from_bytes(encoded).into_expr();

    assert!(address_expr_equivalent(&source, &reassembled));
    assert_eq!(
        symbolic_address_key(&SymWord::Expr(source)),
        symbolic_address_key(&SymWord::Expr(reassembled))
    );
}

#[test]
/// Regression coverage for `selector_shift_simplifies_to_concrete_word`.
fn selector_shift_simplifies_to_concrete_word() {
    let selector = U256::from(0x12345678);
    let call_word = Expr::op(
        ExprOp::Or,
        Expr::op(ExprOp::Shl, Expr::Const(selector), Expr::Const(U256::from(224))),
        Expr::op(ExprOp::Shr, Expr::Var("arg".to_string()), Expr::Const(U256::from(32))),
    );
    let selector_expr = Expr::op(ExprOp::Shr, call_word, Expr::Const(U256::from(224)));

    assert_eq!(expr_known_word(&selector_expr), Some(selector));
}

#[test]
/// Regression coverage for `dynamic_calldata_encodes_bounded_bytes`.
fn dynamic_calldata_encodes_bounded_bytes() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = SymbolicCalldata::new(&function, &config).unwrap();

    assert_eq!(calldata.size, 100);
    assert_eq!(calldata.load(4).unwrap(), SymWord::Concrete(U256::from(32)));
    assert_eq!(calldata.load(36).unwrap(), SymWord::Concrete(U256::from(3)));
    assert_eq!(calldata.byte(71), SymWord::zero());

    let model = BTreeMap::from([
        ("calldata_0_0".to_string(), U256::from(1)),
        ("calldata_0_1".to_string(), U256::from(2)),
        ("calldata_0_2".to_string(), U256::from(3)),
    ]);
    assert_eq!(calldata.model_to_args(&model).unwrap(), vec![DynSolValue::Bytes(vec![1, 2, 3])]);
}

#[test]
/// Regression coverage for `calldata_load_accepts_symbolic_offsets`.
fn calldata_load_accepts_symbolic_offsets() {
    let calldata =
        SymCalldata::new((0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());
    let loaded = calldata.load_word(SymWord::Expr(Expr::Var("offset".to_string()))).unwrap();
    let expected = word_from_bytes((1u8..33).map(|idx| SymWord::Concrete(U256::from(idx + 1))));

    assert_eq!(
        model_word(&loaded, &BTreeMap::from([("offset".to_string(), U256::from(1))])).unwrap(),
        model_word(&expected, &BTreeMap::new()).unwrap()
    );
    assert_eq!(
        model_word(&loaded, &BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
        U256::ZERO
    );
}

#[test]
/// Regression coverage for `calldata_preserves_symbolic_size_for_call_frames`.
fn calldata_preserves_symbolic_size_for_call_frames() {
    let mut memory = SymMemory::default();
    memory.copy_symbolic(
        0,
        vec![
            SymWord::Concrete(U256::from(0xaa)),
            SymWord::Concrete(U256::from(0xbb)),
            SymWord::Concrete(U256::from(0xcc)),
            SymWord::Concrete(U256::from(0xdd)),
        ],
    );
    let size = SymWord::Expr(Expr::Var("size".to_string()));
    let bounded_size = BoundedCopySize::Symbolic { size, max_size: 4 };
    let input = call_input_from_memory(&memory, SymWord::Concrete(U256::ZERO), &bounded_size);
    let calldata = calldata_from_call_input(input, &bounded_size);
    let model = BTreeMap::from([("size".to_string(), U256::from(2))]);

    assert_eq!(model_word(&calldata.size_word, &model).unwrap(), U256::from(2));
    assert_eq!(model_word(&calldata.byte(0), &model).unwrap(), U256::from(0xaa));
    assert_eq!(model_word(&calldata.byte(1), &model).unwrap(), U256::from(0xbb));
    assert_eq!(model_word(&calldata.byte(2), &model).unwrap(), U256::ZERO);
    assert_eq!(model_word(&calldata.byte(3), &model).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `memory_copies_unaligned_symbolic_calldata_bytes`.
fn memory_copies_unaligned_symbolic_calldata_bytes() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = SymbolicCalldata::new(&function, &config).unwrap();
    let mut memory = SymMemory::default();

    memory.copy_calldata(1, 68, 3, &calldata.call_data()).unwrap();
    let word = memory.load_word(0).unwrap();

    let model = BTreeMap::from([
        ("calldata_0_0".to_string(), U256::from(1)),
        ("calldata_0_1".to_string(), U256::from(2)),
        ("calldata_0_2".to_string(), U256::from(3)),
    ]);
    let mut expected = [0u8; 32];
    expected[1..4].copy_from_slice(&[1, 2, 3]);
    assert_eq!(model_word(&word, &model).unwrap(), U256::from_be_bytes(expected));
}

#[test]
/// Regression coverage for `memory_copies_symbolic_calldata_offset`.
fn memory_copies_symbolic_calldata_offset() {
    let calldata =
        SymCalldata::new((0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());
    let mut memory = SymMemory::default();

    memory
        .copy_calldata_offset(0, SymWord::Expr(Expr::Var("offset".to_string())), 2, &calldata)
        .unwrap();
    let word = memory.load_word(0).unwrap();

    let mut expected = [0u8; 32];
    expected[..2].copy_from_slice(&[4, 5]);
    assert_eq!(
        model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(3))])).unwrap(),
        U256::from_be_bytes(expected)
    );
    assert_eq!(
        model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
        U256::ZERO
    );
}

#[test]
/// Regression coverage for `memory_copies_symbolic_calldata_size_with_guarded_tail`.
fn memory_copies_symbolic_calldata_size_with_guarded_tail() {
    let calldata =
        SymCalldata::new((0u8..8).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());
    let mut memory = SymMemory::default();
    memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_calldata_symbolic_size(
            SymWord::Concrete(U256::ZERO),
            SymWord::Concrete(U256::ZERO),
            SymWord::Expr(Expr::Var("size".to_string())),
            4,
            &calldata,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `memory_copies_symbolic_bytecode_size_with_guarded_tail`.
fn memory_copies_symbolic_bytecode_size_with_guarded_tail() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);
    memory.copy_symbolic_size(
        0,
        SymWord::Expr(Expr::Var("size".to_string())),
        (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect(),
    );

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `memory_reads_symbolic_size_with_zero_guarded_tail`.
fn memory_reads_symbolic_size_with_zero_guarded_tail() {
    let mut memory = SymMemory::default();
    memory.store_bytes(32, (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());

    let bytes = memory.read_bytes_symbolic_size(
        SymWord::Concrete(U256::from(32)),
        SymWord::Expr(Expr::Var("size".to_string())),
        4,
    );

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&bytes, &size_two).unwrap(), vec![1, 2, 0, 0]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&bytes, &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `memory_copies_symbolic_memory_size_with_guarded_tail`.
fn memory_copies_symbolic_memory_size_with_guarded_tail() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);
    memory.store_bytes(32, (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());

    memory
        .copy_memory_symbolic_size(
            SymWord::Concrete(U256::ZERO),
            SymWord::Concrete(U256::from(32)),
            SymWord::Expr(Expr::Var("size".to_string())),
            4,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `memory_copies_symbolic_size_to_symbolic_dest`.
fn memory_copies_symbolic_size_to_symbolic_dest() {
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);
    memory.store_bytes(0x20, (0u8..4).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect());

    memory
        .copy_memory_symbolic_size(
            SymWord::Expr(Expr::Var("dest".to_string())),
            SymWord::Concrete(U256::from(0x20)),
            SymWord::Expr(Expr::Var("size".to_string())),
            4,
        )
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("size".to_string(), U256::from(2)),
    ]);
    assert_eq!(model_bytes(&memory.read_bytes(0x80, 4), &model).unwrap(), vec![1, 2, 0xaa, 0xaa]);
}

#[test]
/// Regression coverage for `memory_copies_symbolic_returndata_size_with_guarded_tail`.
fn memory_copies_symbolic_returndata_size_with_guarded_tail() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_return_data_symbolic_size(
            SymWord::Concrete(U256::ZERO),
            SymWord::Concrete(U256::ZERO),
            SymWord::Expr(Expr::Var("size".to_string())),
            4,
            &return_data,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `returndata_reads_symbolic_offset`.
fn returndata_reads_symbolic_offset() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let bytes = return_data.read_bytes_offset(SymWord::Expr(Expr::Var("offset".to_string())), 2);

    let offset_one = BTreeMap::from([("offset".to_string(), U256::from(1))]);
    assert_eq!(model_bytes(&bytes, &offset_one).unwrap(), vec![2, 3]);

    let offset_four = BTreeMap::from([("offset".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&bytes, &offset_four).unwrap(), vec![0, 0]);
}

#[test]
/// Regression coverage for `memory_return_data_accepts_symbolic_size`.
fn memory_return_data_accepts_symbolic_size() {
    let mut memory = SymMemory::default();
    memory.store_bytes(
        0,
        vec![1, 2, 3, 4].into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
    );

    let return_data = memory
        .return_data_symbolic_size(
            SymWord::Concrete(U256::ZERO),
            SymWord::Expr(Expr::Var("len".to_string())),
            4,
        )
        .unwrap();

    let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
    assert_eq!(model_word(&return_data.len_word(), &len_two).unwrap(), U256::from(2));
    assert_eq!(model_word(&return_data.byte(0), &len_two).unwrap(), U256::from(1));
    assert_eq!(model_word(&return_data.byte(2), &len_two).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `call_output_preserves_memory_beyond_symbolic_returndata_size`.
fn call_output_preserves_memory_beyond_symbolic_returndata_size() {
    let return_data = SymReturnData::from_symbolic_bytes_with_len(
        vec![1, 2, 3, 4].into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
        SymWord::Expr(Expr::Var("len".to_string())),
    );
    let mut memory = SymMemory::default();
    memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_call_output_offset(
            SymWord::Concrete(U256::ZERO),
            &BoundedCopySize::Concrete(4),
            &return_data,
        )
        .unwrap();

    let len_two = BTreeMap::from([("len".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &len_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let len_four = BTreeMap::from([("len".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &len_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `nested_dynamic_calldata_uses_preorder_lengths`.
fn nested_dynamic_calldata_uses_preorder_lengths() {
    let function = Function::parse("check((uint256[],bytes))").unwrap();
    let config = SymbolicConfig { array_lengths: vec![2, 3], ..Default::default() };
    let calldata = SymbolicCalldata::new(&function, &config).unwrap();

    assert_eq!(calldata.load(4).unwrap(), SymWord::Concrete(U256::from(32)));
    assert_eq!(calldata.load(36).unwrap(), SymWord::Concrete(U256::from(64)));
    assert_eq!(calldata.load(68).unwrap(), SymWord::Concrete(U256::from(160)));
    assert_eq!(calldata.load(100).unwrap(), SymWord::Concrete(U256::from(2)));
    assert_eq!(calldata.load(196).unwrap(), SymWord::Concrete(U256::from(3)));
}

#[test]
/// Regression coverage for `memory_round_trips_symbolic_words_as_bytes`.
fn memory_round_trips_symbolic_words_as_bytes() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word(7, value.clone());

    let model = BTreeMap::from([("word".to_string(), U256::from(0x1234))]);
    assert_eq!(
        model_word(&memory.load_word(7).unwrap(), &model).unwrap(),
        model_word(&value, &model).unwrap()
    );
}

#[test]
/// Regression coverage for `memory_load_accepts_symbolic_offsets`.
fn memory_load_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word(7, value);
    let loaded = memory.load_word_offset(SymWord::Expr(Expr::Var("offset".to_string()))).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));

    let out_of_range = BTreeMap::from([
        ("offset".to_string(), U256::from(39)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &out_of_range).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `memory_store_word_accepts_symbolic_offsets`.
fn memory_store_word_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word_offset(SymWord::Expr(Expr::Var("offset".to_string())), value);
    let loaded = memory.load_word(7).unwrap();

    let matching = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &matching).unwrap(), U256::from(0x1234));

    let non_matching = BTreeMap::from([
        ("offset".to_string(), U256::from(100)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &non_matching).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `memory_size_tracks_concrete_and_symbolic_extents`.
fn memory_size_tracks_concrete_and_symbolic_extents() {
    let mut memory = SymMemory::default();

    memory.store_word(7, SymWord::Concrete(U256::from(0x55)));
    assert_eq!(memory.size_word(), SymWord::Concrete(U256::from(64)));

    memory.store_word_offset(
        SymWord::Expr(Expr::Var("offset".to_string())),
        SymWord::Expr(Expr::Var("word".to_string())),
    );
    let size = memory.size_word();

    let below_concrete = BTreeMap::from([
        ("offset".to_string(), U256::from(9)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&size, &below_concrete).unwrap(), U256::from(64));

    let above_concrete = BTreeMap::from([
        ("offset".to_string(), U256::from(70)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&size, &above_concrete).unwrap(), U256::from(128));
}

#[test]
/// Regression coverage for `memory_concrete_write_overrides_older_symbolic_write`.
fn memory_concrete_write_overrides_older_symbolic_write() {
    let mut memory = SymMemory::default();

    memory.store_word_offset(
        SymWord::Expr(Expr::Var("offset".to_string())),
        SymWord::Expr(Expr::Var("word".to_string())),
    );
    memory.store_word(7, SymWord::Concrete(U256::from(0x55)));
    let loaded = memory.load_word(7).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x55));
}

#[test]
/// Regression coverage for `memory_store_byte_accepts_symbolic_offsets`.
fn memory_store_byte_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();

    memory.store_byte_offset(
        SymWord::Expr(Expr::Var("offset".to_string())),
        SymWord::Expr(Expr::Var("byte".to_string())),
    );
    let loaded = memory.byte(0x80);

    let matching = BTreeMap::from([
        ("offset".to_string(), U256::from(0x80)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(model_word(&loaded, &matching).unwrap(), U256::from(0xab));

    let non_matching = BTreeMap::from([
        ("offset".to_string(), U256::from(0x81)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(model_word(&loaded, &non_matching).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `memory_read_bytes_accepts_symbolic_offsets`.
fn memory_read_bytes_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word(7, value);
    let loaded = word_from_bytes(
        memory.read_bytes_offset(SymWord::Expr(Expr::Var("offset".to_string())), 32),
    );

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));

    let out_of_range = BTreeMap::from([
        ("offset".to_string(), U256::from(39)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &out_of_range).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `memory_return_data_accepts_symbolic_offsets`.
fn memory_return_data_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word(7, value);
    let return_data =
        memory.return_data(SymWord::Expr(Expr::Var("offset".to_string())), 32).unwrap();
    let loaded = return_data.load_word(0).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));
}

#[test]
/// Regression coverage for `memory_copy_accepts_symbolic_source_offsets`.
fn memory_copy_accepts_symbolic_source_offsets() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word(7, value);
    memory.copy_memory_offset(64, SymWord::Expr(Expr::Var("src".to_string())), 32).unwrap();
    let loaded = memory.load_word(64).unwrap();

    let model = BTreeMap::from([
        ("src".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));
}

#[test]
/// Regression coverage for `memory_copy_accepts_symbolic_destination_offsets`.
fn memory_copy_accepts_symbolic_destination_offsets() {
    let mut memory = SymMemory::default();
    let value = SymWord::Expr(Expr::Var("word".to_string()));

    memory.store_word(7, value);
    memory
        .copy_memory_to_offset(
            SymWord::Expr(Expr::Var("dest".to_string())),
            SymWord::Concrete(U256::from(7)),
            32,
        )
        .unwrap();
    let loaded = memory.load_word(64).unwrap();

    let matching = BTreeMap::from([
        ("dest".to_string(), U256::from(64)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &matching).unwrap(), U256::from(0x1234));

    let non_matching = BTreeMap::from([
        ("dest".to_string(), U256::from(96)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &non_matching).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `memory_call_output_accepts_symbolic_destination_offsets`.
fn memory_call_output_accepts_symbolic_destination_offsets() {
    let mut memory = SymMemory::default();
    let return_data = SymReturnData::from_symbolic_bytes(word_bytes(SymWord::Expr(Expr::Var(
        "word".to_string(),
    ))));

    memory
        .copy_call_output_offset(
            SymWord::Expr(Expr::Var("dest".to_string())),
            &BoundedCopySize::Concrete(32),
            &return_data,
        )
        .unwrap();
    let loaded = memory.load_word(64).unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(64)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x1234));
}

#[test]
/// Regression coverage for `memory_call_output_accepts_symbolic_size_with_guarded_tail`.
fn memory_call_output_accepts_symbolic_size_with_guarded_tail() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    memory.store_bytes(0, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_call_output_offset(
            SymWord::Concrete(U256::ZERO),
            &BoundedCopySize::Symbolic {
                size: SymWord::Expr(Expr::Var("size".to_string())),
                max_size: 4,
            },
            &return_data,
        )
        .unwrap();

    let size_two = BTreeMap::from([("size".to_string(), U256::from(2))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_two).unwrap(), vec![1, 2, 0xaa, 0xaa]);

    let size_four = BTreeMap::from([("size".to_string(), U256::from(4))]);
    assert_eq!(model_bytes(&memory.read_bytes(0, 4), &size_four).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
/// Regression coverage for `memory_call_output_accepts_symbolic_destination_and_size`.
fn memory_call_output_accepts_symbolic_destination_and_size() {
    let return_data = SymReturnData::from_concrete_bytes(vec![1, 2, 3, 4]);
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_call_output_offset(
            SymWord::Expr(Expr::Var("dest".to_string())),
            &BoundedCopySize::Symbolic {
                size: SymWord::Expr(Expr::Var("size".to_string())),
                max_size: 4,
            },
            &return_data,
        )
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("size".to_string(), U256::from(2)),
    ]);
    assert_eq!(model_bytes(&memory.read_bytes(0x80, 4), &model).unwrap(), vec![1, 2, 0xaa, 0xaa]);
}

#[test]
/// Regression coverage for `memory_call_output_accepts_symbolic_destination_and_return_len`.
fn memory_call_output_accepts_symbolic_destination_and_return_len() {
    let return_data = SymReturnData::from_symbolic_bytes_with_len(
        vec![1, 2, 3, 4].into_iter().map(|byte| SymWord::Concrete(U256::from(byte))).collect(),
        SymWord::Expr(Expr::Var("len".to_string())),
    );
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_call_output_offset(
            SymWord::Expr(Expr::Var("dest".to_string())),
            &BoundedCopySize::Concrete(4),
            &return_data,
        )
        .unwrap();

    let model = BTreeMap::from([
        ("dest".to_string(), U256::from(0x80)),
        ("len".to_string(), U256::from(2)),
    ]);
    assert_eq!(model_bytes(&memory.read_bytes(0x80, 4), &model).unwrap(), vec![1, 2, 0xaa, 0xaa]);
}

#[test]
/// Regression coverage for `create_address_helpers_match_alloy_primitives`.
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
/// Regression coverage for `compute_create2_cheatcode_helper_matches_create2_terms`.
fn compute_create2_cheatcode_helper_matches_create2_terms() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymWord::Expr(Expr::Var("salt".to_string()));
    let initcode = vec![opcode::STOP];
    let initcode_hash = SymWord::Concrete(U256::from_be_bytes(keccak256(&initcode).0));

    let cheatcode_word = compute_create2_address_word(
        &mut state,
        SymWord::Concrete(address_word(creator)),
        salt.clone(),
        initcode_hash,
    )
    .unwrap();
    let opcode_word =
        create2_address_word(&mut state, creator, salt, &SymCode::concrete(initcode)).unwrap().0;

    assert_eq!(cheatcode_word, opcode_word);
}

#[test]
/// Regression coverage for `compute_create2_cheatcode_helper_accepts_symbolic_init_code_hash`.
fn compute_create2_cheatcode_helper_accepts_symbolic_init_code_hash() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymWord::Expr(Expr::Var("salt".to_string()));
    let initcode_hash = SymWord::Expr(Expr::Var("initcode_hash".to_string()));

    let first = compute_create2_address_word(
        &mut state,
        SymWord::Concrete(address_word(creator)),
        salt.clone(),
        initcode_hash.clone(),
    )
    .unwrap();
    let second = compute_create2_address_word(
        &mut state,
        SymWord::Concrete(address_word(creator)),
        salt,
        initcode_hash,
    )
    .unwrap();

    assert_eq!(first, second);
    assert!(
        matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create2_address_"))
    );
}

#[test]
/// Regression coverage for `compute_create_cheatcode_helper_accepts_symbolic_nonce`.
fn compute_create_cheatcode_helper_accepts_symbolic_nonce() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let nonce = SymWord::Expr(Expr::Var("nonce".to_string()));

    let first = compute_create_address_word(
        &mut state,
        SymWord::Concrete(address_word(creator)),
        nonce.clone(),
    )
    .unwrap();
    let second =
        compute_create_address_word(&mut state, SymWord::Concrete(address_word(creator)), nonce)
            .unwrap();

    assert_eq!(first, second);
    assert!(matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create_address_")));
}

#[test]
/// Regression coverage for `compute_create_cheatcode_helper_accepts_symbolic_deployer`.
fn compute_create_cheatcode_helper_accepts_symbolic_deployer() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let deployer = SymWord::Expr(Expr::Var("deployer".to_string()));
    let nonce = SymWord::Expr(Expr::Var("nonce".to_string()));

    let first = compute_create_address_word(&mut state, deployer.clone(), nonce.clone())
        .expect("symbolic deployer is supported");
    let second = compute_create_address_word(&mut state, deployer, nonce)
        .expect("symbolic deployer is supported");

    assert_eq!(first, second);
    assert!(matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create_address_")));
}

#[test]
/// Regression coverage for `compute_create2_cheatcode_helper_accepts_symbolic_deployer`.
fn compute_create2_cheatcode_helper_accepts_symbolic_deployer() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let deployer = SymWord::Expr(Expr::Var("deployer".to_string()));
    let salt = SymWord::Expr(Expr::Var("salt".to_string()));
    let initcode_hash = SymWord::Expr(Expr::Var("initcode_hash".to_string()));

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
    assert!(
        matches!(first, SymWord::Expr(Expr::Var(name)) if name.starts_with("create2_address_"))
    );
}

#[test]
/// Regression coverage for `recorded_logs_return_data_matches_abi_encoding`.
fn recorded_logs_return_data_matches_abi_encoding() {
    let emitter = Address::from([0x33; 20]);
    let topic = B256::from([0x11; 32]);
    let log = SymbolicLog {
        topics: vec![SymWord::Concrete(U256::from_be_bytes(topic.0))],
        data_len: SymWord::Concrete(U256::from(2)),
        data: vec![SymWord::Concrete(U256::from(0x22)), SymWord::Concrete(U256::from(0x33))],
        emitter,
    };

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
/// Regression coverage for `recorded_logs_json_return_data_accepts_symbolic_topics_and_data`.
fn recorded_logs_json_return_data_accepts_symbolic_topics_and_data() {
    let emitter = Address::from([0x33; 20]);
    let log = SymbolicLog {
        topics: vec![SymWord::Expr(Expr::Var("topic".to_string()))],
        data_len: SymWord::Concrete(U256::from(2)),
        data: vec![
            SymWord::Concrete(U256::from(0x12)),
            SymWord::Expr(Expr::Var("byte".to_string())),
        ],
        emitter,
    };

    let return_data = recorded_logs_json_return_data(vec![log]).unwrap();
    let encoded = model_bytes(
        &(0..return_data.len).map(|idx| return_data.byte(idx)).collect::<Vec<_>>(),
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
/// Regression coverage for `abi_bytes_encoding_accepts_symbolic_length`.
fn abi_bytes_encoding_accepts_symbolic_length() {
    let encoded = encode_packed_bytes_with_len(
        SymWord::Expr(Expr::Var("len".to_string())),
        &[
            SymWord::Concrete(U256::from(0x22)),
            SymWord::Concrete(U256::from(0x33)),
            SymWord::Concrete(U256::from(0x44)),
        ],
    );
    let length = word_from_bytes(encoded[..32].iter().cloned());

    assert_eq!(
        model_word(&length, &BTreeMap::from([("len".to_string(), U256::from(2))])).unwrap(),
        U256::from(2)
    );
}

#[test]
/// Regression coverage for `symbolic_world_resolves_symbolic_create2_address_aliases`.
fn symbolic_world_resolves_symbolic_create2_address_aliases() {
    let mut world = SymbolicWorld::default();
    let word = SymWord::Expr(Expr::Var("create2_address".to_string()));
    let address = world.symbolic_address_slot(word.clone());
    let masked = SymWord::Expr(Expr::op(
        ExprOp::And,
        word.clone().into_expr(),
        Expr::Const((U256::from(1) << 160) - U256::from(1)),
    ));

    assert_eq!(world.resolve_address(&word), Some(address));
    assert_eq!(world.resolve_address(&masked), Some(address));
    assert_eq!(world.symbolic_address_slot(word), address);
    assert_ne!(address, Address::ZERO);
}

#[test]
/// Regression coverage for `symbolic_create2_accepts_symbolic_salt`.
fn symbolic_create2_accepts_symbolic_salt() {
    let creator = Address::from([0x11; 20]);
    let mut state = PathState::empty(creator, Address::from([0xaa; 20]), false);
    let salt = SymWord::Expr(Expr::Var("salt".to_string()));
    let initcode = SymCode::concrete(vec![opcode::STOP]);

    let (word, address) = create2_address_word(&mut state, creator, salt, &initcode).unwrap();

    assert!(matches!(word, SymWord::Expr(_)));
    assert_eq!(state.world.resolve_address(&word), Some(address));
    assert_eq!(state.constraints.len(), 1);
    assert_ne!(address, Address::ZERO);
}

#[test]
/// Regression coverage for `symbolic_return_data_can_be_installed_as_runtime_code`.
fn symbolic_return_data_can_be_installed_as_runtime_code() {
    let data = SymReturnData::from_symbolic_bytes(vec![SymWord::Expr(Expr::Var(
        "runtime_byte".to_string(),
    ))]);

    let code = data.to_code();

    assert_eq!(code.read_bytes(0, 1), vec![SymWord::Expr(Expr::Var("runtime_byte".to_string()))]);
}

#[test]
/// Regression coverage for `symbolic_world_tracks_created_code_and_nonce_overlay`.
fn symbolic_world_tracks_created_code_and_nonce_overlay() {
    let created = Address::from([0x22; 20]);
    let mut world = SymbolicWorld::default();

    world.install_code(created, SymCode::concrete(vec![opcode::STOP]));
    world.set_nonce(created, 1);

    assert_eq!(world.code_cache.get(&created), Some(&SymCode::concrete(vec![opcode::STOP])));
    assert_eq!(world.nonces.get(&created), Some(&1));
}

#[test]
/// Regression coverage for `symbolic_codecopy_preserves_symbolic_constructor_bytes`.
fn symbolic_codecopy_preserves_symbolic_constructor_bytes() {
    let mut memory = SymMemory::default();
    let initcode = SymCode {
        bytes: vec![
            SymWord::Concrete(U256::from(opcode::STOP)),
            SymWord::Expr(Expr::Var("constructor_arg_byte".to_string())),
        ],
    };

    memory.copy_symbolic(0, initcode.read_bytes(0, 2));

    assert_eq!(memory.byte(0), SymWord::Concrete(U256::from(opcode::STOP)));
    assert_eq!(memory.byte(1), SymWord::Expr(Expr::Var("constructor_arg_byte".to_string())));
}

#[test]
/// Regression coverage for `symbolic_codecopy_accepts_symbolic_offsets`.
fn symbolic_codecopy_accepts_symbolic_offsets() {
    let code =
        SymCode { bytes: (0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect() };
    let mut memory = SymMemory::default();

    memory.copy_symbolic(
        0,
        code.read_bytes_offset(SymWord::Expr(Expr::Var("offset".to_string())), 2),
    );
    let word = memory.load_word(0).unwrap();

    let mut expected = [0u8; 32];
    expected[..2].copy_from_slice(&[4, 5]);
    assert_eq!(
        model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(3))])).unwrap(),
        U256::from_be_bytes(expected)
    );
    assert_eq!(
        model_word(&word, &BTreeMap::from([("offset".to_string(), U256::from(40))])).unwrap(),
        U256::ZERO
    );
}

#[test]
/// Regression coverage for `symbolic_initcode_accepts_symbolic_memory_offsets`.
fn symbolic_initcode_accepts_symbolic_memory_offsets() {
    let mut memory = SymMemory::default();

    memory.copy_symbolic(
        7,
        vec![
            SymWord::Concrete(U256::from(opcode::STOP)),
            SymWord::Expr(Expr::Var("arg".to_string())),
        ],
    );
    let initcode =
        SymCode::from_memory_offset(&memory, SymWord::Expr(Expr::Var("offset".to_string())), 2);
    let word = word_from_bytes(
        initcode.read_bytes(0, 2).into_iter().chain(std::iter::repeat_with(SymWord::zero).take(30)),
    );

    let mut expected = [0u8; 32];
    expected[0] = opcode::STOP;
    expected[1] = 0x2a;
    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("arg".to_string(), U256::from(0x2a)),
    ]);
    assert_eq!(model_word(&word, &model).unwrap(), U256::from_be_bytes(expected));
}

#[test]
/// Regression coverage for `path_state_extracts_constrained_symbolic_usize`.
fn path_state_extracts_constrained_symbolic_usize() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let offset = SymWord::Expr(Expr::Var("offset".to_string()));

    state.constraints.push(BoolExpr::eq(offset.clone().into_expr(), Expr::Const(U256::from(7))));

    assert_eq!(state.constrained_usize(&offset), Some(7));
}

#[test]
/// Regression coverage for `path_state_extracts_symbolic_usize_upper_bound`.
fn path_state_extracts_symbolic_usize_upper_bound() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let size = SymWord::Expr(Expr::Var("size".to_string()));

    state.constraints.push(BoolExpr::cmp(
        BoolExprOp::Ult,
        size.clone().into_expr(),
        Expr::Const(U256::from(5)),
    ));

    assert_eq!(state.upper_bound_usize(&size), Some(4));
}

#[test]
/// Regression coverage for `path_state_extracts_constrained_symbolic_usize_from_encoded_bool_word`.
fn path_state_extracts_constrained_symbolic_usize_from_encoded_bool_word() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let offset = SymWord::Expr(Expr::Var("offset".to_string()));
    let offset_expr = offset.clone().into_expr();
    let mask = Expr::Const(U256::from(0xffff));

    state.constraints.push(BoolExpr::eq(
        Expr::op(ExprOp::And, offset_expr.clone(), mask.clone()),
        offset_expr.clone(),
    ));
    let condition =
        BoolExpr::eq(Expr::Const(U256::from(0x80)), Expr::op(ExprOp::And, mask, offset_expr));
    let bool_byte = Expr::op(
        ExprOp::And,
        Expr::op(
            ExprOp::Shr,
            Expr::Ite(
                Box::new(condition),
                Box::new(Expr::Const(U256::from(1))),
                Box::new(Expr::Const(U256::ZERO)),
            ),
            Expr::Const(U256::ZERO),
        ),
        Expr::Const(U256::from(0xff)),
    );
    state.constraints.push(
        BoolExpr::eq(
            Expr::op(ExprOp::Or, Expr::Const(U256::ZERO), bool_byte),
            Expr::Const(U256::ZERO),
        )
        .not(),
    );

    assert_eq!(state.constrained_usize(&offset), Some(0x80));
}

#[test]
/// Regression coverage for `path_state_evaluates_compound_constrained_symbolic_word`.
fn path_state_evaluates_compound_constrained_symbolic_word() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let value = SymWord::Expr(Expr::Var("value".to_string()));
    state
        .constraints
        .push(BoolExpr::eq(value.clone().into_expr(), Expr::Const(U256::from(0xbeef))));

    let encoded_word = SymWord::Expr(Expr::op(
        ExprOp::Or,
        Expr::Const(U256::ZERO),
        Expr::op(ExprOp::And, value.into_expr(), Expr::Const(U256::from(u64::MAX))),
    ));

    assert_eq!(state.constrained_word(&encoded_word), Some(U256::from(0xbeef)));
}

#[test]
/// Regression coverage for `symbolic_push_data_reconstructs_symbolic_word`.
fn symbolic_push_data_reconstructs_symbolic_word() {
    let code = SymCode {
        bytes: vec![
            SymWord::Concrete(U256::from(opcode::PUSH2)),
            SymWord::Expr(Expr::Var("immutable_hi".to_string())),
            SymWord::Expr(Expr::Var("immutable_lo".to_string())),
        ],
    };

    let word = word_from_bytes(
        std::iter::repeat_with(SymWord::zero).take(30).chain(code.read_bytes(1, 2)),
    );

    assert!(matches!(word, SymWord::Expr(_)));
}

#[test]
/// Regression coverage for `abi_bytes_return_encodes_symbolic_bytes`.
fn abi_bytes_return_encodes_symbolic_bytes() {
    let ret = abi_bytes_return(vec![
        SymWord::Expr(Expr::Var("calldata_byte_0".to_string())),
        SymWord::Concrete(U256::from(0x42)),
    ]);

    assert_eq!(
        word_from_bytes((0..32).map(|idx| ret.byte(idx))),
        SymWord::Concrete(U256::from(32))
    );
    assert_eq!(
        word_from_bytes((32..64).map(|idx| ret.byte(idx))),
        SymWord::Concrete(U256::from(2))
    );
    assert_eq!(ret.byte(64), SymWord::Expr(Expr::Var("calldata_byte_0".to_string())));
    assert_eq!(ret.byte(65), SymWord::Concrete(U256::from(0x42)));
}

#[test]
/// Regression coverage for `abi_bytes_return_can_encode_symbolic_length`.
fn abi_bytes_return_can_encode_symbolic_length() {
    let ret = abi_bytes_return_with_len(
        SymWord::Expr(Expr::Var("len".to_string())),
        vec![
            SymWord::Expr(Expr::Var("byte_0".to_string())),
            SymWord::Expr(Expr::Var("byte_1".to_string())),
        ],
    );

    assert_eq!(
        word_from_bytes((0..32).map(|idx| ret.byte(idx))),
        SymWord::Concrete(U256::from(32))
    );
    assert_eq!(
        word_from_bytes((32..64).map(|idx| ret.byte(idx))),
        SymWord::Expr(Expr::Var("len".to_string()))
    );
    assert_eq!(ret.byte(64), SymWord::Expr(Expr::Var("byte_0".to_string())));
    assert_eq!(ret.byte(65), SymWord::Expr(Expr::Var("byte_1".to_string())));
}

#[test]
/// Regression coverage for `symbolic_keccak_is_deterministic_for_same_symbolic_bytes`.
fn symbolic_keccak_is_deterministic_for_same_symbolic_bytes() {
    let bytes = word_bytes(SymWord::Expr(Expr::Var("slot_key".to_string())));

    let first = keccak_word(bytes.clone());
    let second = keccak_word(bytes);

    assert_eq!(first, second);
    assert!(matches!(first, SymWord::Expr(Expr::Keccak { .. })));
}

#[test]
/// Regression coverage for `symbolic_keccak_tracks_symbolic_length`.
fn symbolic_keccak_tracks_symbolic_length() {
    let bytes = vec![
        SymWord::Expr(Expr::Var("byte_0".to_string())),
        SymWord::Expr(Expr::Var("byte_1".to_string())),
        SymWord::zero(),
    ];
    let len = SymWord::Expr(Expr::Var("len".to_string()));

    let word = keccak_word_with_len(bytes, len);

    let SymWord::Expr(Expr::Keccak { len, bytes, .. }) = word else {
        panic!("expected symbolic keccak term");
    };
    assert_eq!(*len, Expr::Var("len".to_string()));
    assert_eq!(bytes.len(), 3);
}

#[test]
/// Regression coverage for `model_word_computes_symbolic_keccak_from_model`.
fn model_word_computes_symbolic_keccak_from_model() {
    let owner = Address::from([0x11; 20]);
    let slot = U256::from(1);
    let mut bytes = word_bytes(SymWord::Expr(Expr::Var("owner".to_string())));
    bytes.extend(word_bytes(SymWord::Concrete(slot)));
    let word = keccak_word(bytes);

    let mut input = Vec::new();
    input.extend(address_word(owner).to_be_bytes::<32>());
    input.extend(slot.to_be_bytes::<32>());
    let expected = U256::from_be_bytes(keccak256(input).0);
    let model = BTreeMap::from([("owner".to_string(), address_word(owner))]);

    assert_eq!(model_word(&word, &model).unwrap(), expected);
}

#[test]
/// Regression coverage for `symbolic_hash_precompiles_are_deterministic_for_same_symbolic_input`.
fn symbolic_hash_precompiles_are_deterministic_for_same_symbolic_input() {
    let input = vec![
        SymWord::Expr(Expr::Var("input_0".to_string())),
        SymWord::Expr(Expr::Var("input_1".to_string())),
    ];

    let input_len = SymWord::Concrete(U256::from(input.len()));
    let sha = execute_symbolic_precompile(precompile_address(2), input.clone(), input_len.clone())
        .unwrap()
        .unwrap();
    let sha_again =
        execute_symbolic_precompile(precompile_address(2), input.clone(), input_len.clone())
            .unwrap()
            .unwrap();
    let sha_word = word_from_bytes((0..32).map(|idx| sha.byte(idx)));
    let sha_again_word = word_from_bytes((0..32).map(|idx| sha_again.byte(idx)));

    assert_eq!(sha.len, 32);
    assert_eq!(sha_word, sha_again_word);
    assert!(matches!(sha_word, SymWord::Expr(Expr::Hash { algorithm: "sha256", .. })));

    let ecrecover =
        execute_symbolic_precompile(precompile_address(1), input.clone(), input_len.clone())
            .unwrap()
            .unwrap();
    let ecrecover_again =
        execute_symbolic_precompile(precompile_address(1), input.clone(), input_len.clone())
            .unwrap()
            .unwrap();

    assert_eq!(ecrecover.len, 32);
    for idx in 0..12 {
        assert_eq!(ecrecover.byte(idx), SymWord::zero());
    }
    for idx in 0..32 {
        assert_eq!(ecrecover.byte(idx), ecrecover_again.byte(idx));
    }

    let ripemd =
        execute_symbolic_precompile(precompile_address(3), input.clone(), input_len.clone())
            .unwrap()
            .unwrap();
    let ripemd_again =
        execute_symbolic_precompile(precompile_address(3), input, input_len).unwrap().unwrap();

    assert_eq!(ripemd.len, 32);
    for idx in 0..12 {
        assert_eq!(ripemd.byte(idx), SymWord::zero());
    }
    for idx in 0..32 {
        assert_eq!(ripemd.byte(idx), ripemd_again.byte(idx));
    }
}

#[test]
/// Regression coverage for `identity_precompile_preserves_symbolic_input_len`.
fn identity_precompile_preserves_symbolic_input_len() {
    let input = vec![
        SymWord::Concrete(U256::from(1)),
        SymWord::Concrete(U256::from(2)),
        SymWord::Concrete(U256::from(3)),
        SymWord::Concrete(U256::from(4)),
    ];
    let input_len = SymWord::Expr(Expr::Var("size".to_string()));
    let return_data = execute_symbolic_precompile(precompile_address(4), input, input_len.clone())
        .unwrap()
        .unwrap();

    assert_eq!(return_data.len, 4);
    assert_eq!(return_data.len_word(), input_len);
    assert_eq!(return_data.byte(0), SymWord::Concrete(U256::from(1)));
    assert_eq!(return_data.byte(3), SymWord::Concrete(U256::from(4)));
}

#[test]
/// Regression coverage for `advanced_precompiles_accept_symbolic_payloads`.
fn advanced_precompiles_accept_symbolic_payloads() {
    let mut modexp_input = vec![SymWord::zero(); 99];
    modexp_input[31] = SymWord::Concrete(U256::from(1));
    modexp_input[63] = SymWord::Concrete(U256::from(1));
    modexp_input[95] = SymWord::Concrete(U256::from(1));
    modexp_input[96] = SymWord::Expr(Expr::Var("base".to_string()));
    modexp_input[97] = SymWord::Concrete(U256::from(5));
    modexp_input[98] = SymWord::Concrete(U256::from(13));

    let modexp = execute_symbolic_precompile(
        precompile_address(5),
        modexp_input.clone(),
        SymWord::Concrete(U256::from(modexp_input.len())),
    )
    .unwrap()
    .unwrap();
    let modexp_again = execute_symbolic_precompile(
        precompile_address(5),
        modexp_input.clone(),
        SymWord::Concrete(U256::from(modexp_input.len())),
    )
    .unwrap()
    .unwrap();
    assert_eq!(modexp.len, 1);
    assert_eq!(modexp.byte(0), modexp_again.byte(0));

    let bn_input = vec![SymWord::Expr(Expr::Var("point".to_string())); 128];
    let bn_add = execute_symbolic_precompile(
        precompile_address(6),
        bn_input,
        SymWord::Concrete(U256::from(128)),
    )
    .unwrap()
    .unwrap();
    assert_eq!(bn_add.len, 64);

    let blake_input = vec![SymWord::Expr(Expr::Var("blake_input".to_string())); 213];
    let blake = execute_symbolic_precompile(
        precompile_address(9),
        blake_input,
        SymWord::Concrete(U256::from(213)),
    )
    .unwrap()
    .unwrap();
    assert_eq!(blake.len, 64);
}

#[test]
/// Regression coverage for `symbolic_storage_read_after_write_accepts_symbolic_keys`.
fn symbolic_storage_read_after_write_accepts_symbolic_keys() {
    let address = Address::from([0x11; 20]);
    let key = SymWord::Expr(Expr::Var("slot".to_string()));
    let value = SymWord::Expr(Expr::Var("value".to_string()));
    let writes = vec![StorageWrite::new(address, key.clone(), value.clone())];

    assert_eq!(read_storage_writes(&writes, address, key, SymWord::zero()), value);
}

#[test]
/// Regression coverage for `symbolic_storage_uses_conditional_value_for_maybe_equal_key`.
fn symbolic_storage_uses_conditional_value_for_maybe_equal_key() {
    let address = Address::from([0x11; 20]);
    let write_key = SymWord::Expr(Expr::Var("write_slot".to_string()));
    let read_key = SymWord::Expr(Expr::Var("read_slot".to_string()));
    let value = SymWord::Expr(Expr::Var("value".to_string()));
    let writes = vec![StorageWrite::new(address, write_key.clone(), value.clone())];

    assert_eq!(
        read_storage_writes(&writes, address, read_key.clone(), SymWord::zero()),
        SymWord::Expr(Expr::Ite(
            Box::new(BoolExpr::eq(read_key.into_expr(), write_key.into_expr())),
            Box::new(value.into_expr()),
            Box::new(Expr::Const(U256::ZERO)),
        ))
    );
}

#[test]
/// Regression coverage for `symbolic_storage_key_equality_decomposes_keccak_offsets`.
fn symbolic_storage_key_equality_decomposes_keccak_offsets() {
    let owner = SymWord::Expr(Expr::Var("owner".to_string()));
    let base = keccak_word(word_bytes(owner));
    let left = add_words(base.clone(), SymWord::Expr(Expr::Var("left_index".to_string())));
    let right = add_words(base, SymWord::Expr(Expr::Var("right_index".to_string())));

    assert_eq!(
        storage_key_eq(left, right),
        BoolExpr::eq(Expr::Var("left_index".to_string()), Expr::Var("right_index".to_string()))
    );
}

#[test]
/// Regression coverage for `symbolic_storage_key_equality_expands_distinct_keccak_bases`.
fn symbolic_storage_key_equality_expands_distinct_keccak_bases() {
    let left_base = keccak_word(vec![SymWord::Expr(Expr::Var("left_owner".to_string()))]);
    let right_base = keccak_word(vec![SymWord::Expr(Expr::Var("right_owner".to_string()))]);
    let index = SymWord::Expr(Expr::Var("index".to_string()));

    let condition =
        storage_key_eq(add_words(left_base, index.clone()), add_words(right_base, index));

    assert_eq!(
        condition,
        BoolExpr::eq(Expr::Var("left_owner".to_string()), Expr::Var("right_owner".to_string()))
    );
}

#[test]
/// Regression coverage for `symbolic_storage_key_equality_rejects_concrete_plain_slot_alias`.
fn symbolic_storage_key_equality_rejects_concrete_plain_slot_alias() {
    let owner = SymWord::Expr(Expr::Var("owner".to_string()));
    let layout_key = add_words(keccak_word(word_bytes(owner)), SymWord::Concrete(U256::ZERO));

    assert_eq!(storage_key_eq(layout_key, SymWord::Concrete(U256::ZERO)), BoolExpr::Const(false));
}

#[test]
/// Regression coverage for `nested_mapping_key_does_not_alias_plain_mapping_key_under_model`.
fn nested_mapping_key_does_not_alias_plain_mapping_key_under_model() {
    let owner = SymWord::Expr(Expr::Var("owner".to_string()));
    let spender = SymWord::Expr(Expr::Var("spender".to_string()));
    let recipient = SymWord::Expr(Expr::Var("recipient".to_string()));

    let mut balance_key_bytes = word_bytes(recipient);
    balance_key_bytes.extend(word_bytes(SymWord::Concrete(U256::ZERO)));
    let balance_key = keccak_word(balance_key_bytes);

    let mut inner_key_bytes = word_bytes(owner);
    inner_key_bytes.extend(word_bytes(SymWord::Concrete(U256::from(1))));
    let inner_key = keccak_word(inner_key_bytes);

    let mut allowance_key_bytes = word_bytes(spender);
    allowance_key_bytes.extend(word_bytes(inner_key));
    let allowance_key = keccak_word(allowance_key_bytes);

    let same_address = precompile_address(0x60);
    let model = BTreeMap::from([
        ("owner".to_string(), address_word(Address::from([0x11; 20]))),
        ("spender".to_string(), address_word(same_address)),
        ("recipient".to_string(), address_word(same_address)),
    ]);
    let condition = storage_key_eq(balance_key, allowance_key);

    assert_eq!(condition, BoolExpr::Const(false));
    assert!(!eval_bool_expr(&condition, &model).unwrap());
}

#[test]
/// Regression coverage for `symbolic_world_snapshot_restores_overlay_state`.
fn symbolic_world_snapshot_restores_overlay_state() {
    let address = Address::from([0x11; 20]);
    let mut world = SymbolicWorld::default();
    world.sstore(address, SymWord::Concrete(U256::from(1)), SymWord::Concrete(U256::from(2)));

    let snapshot = world.snapshot_state();
    world.sstore(address, SymWord::Concrete(U256::from(1)), SymWord::Concrete(U256::from(3)));

    assert!(world.restore_snapshot(snapshot));
    assert_eq!(world.storage.len(), 1);
    assert_eq!(world.storage[0].value, SymWord::Concrete(U256::from(2)));
}

#[test]
/// Regression coverage for `extra_dynamic_lengths_are_rejected`.
fn extra_dynamic_lengths_are_rejected() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![1, 2], ..Default::default() };

    let err = SymbolicCalldata::new(&function, &config).unwrap_err();

    assert!(err.to_string().contains("symbolic.array_lengths has 2 entries"));
}

#[test]
/// Regression coverage for `positional_dynamic_lengths_allow_shorter_expanded_variants`.
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
        .map(|calldata| match &calldata.inputs[0].value {
            SymbolicAbiValue::Array { elements } => elements.len(),
            value => panic!("expected array input, got {value:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(element_counts, vec![1, 2]);
}

#[test]
/// Regression coverage for `extra_dynamic_lengths_are_rejected_after_expansion`.
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
/// Regression coverage for `calldata_variant_expansion_respects_path_width`.
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
/// Regression coverage for `symbolic_signextend_uses_sign_bit_ite`.
fn symbolic_signextend_uses_sign_bit_ite() {
    assert_eq!(
        signextend_word(U256::ZERO, SymWord::Expr(Expr::Var("word".to_string()))),
        SymWord::Expr(Expr::Ite(
            Box::new(BoolExpr::eq(
                Expr::op(ExprOp::And, Expr::Var("word".to_string()), Expr::Const(U256::from(0x80))),
                Expr::Const(U256::ZERO)
            )),
            Box::new(Expr::op(
                ExprOp::And,
                Expr::Var("word".to_string()),
                Expr::Const(U256::from(0x7f))
            )),
            Box::new(Expr::op(
                ExprOp::Or,
                Expr::Var("word".to_string()),
                Expr::Const(!U256::from(0x7f))
            )),
        ))
    );
}

#[test]
/// Regression coverage for `parse_smt_hex_model_values`.
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
/// Regression coverage for `parse_smt_binary_model_values`.
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
/// Regression coverage for `parse_model_rejects_oversized_bitvector_literals`.
fn parse_model_rejects_oversized_bitvector_literals() {
    let output =
        format!("sat\n((define-fun calldata_0 () (_ BitVec 257) #b{}))\n", "1".repeat(257));

    let err = parse_model(&output).unwrap_err();

    assert!(err.to_string().contains("exceeds 256 bits"));
}

#[test]
/// Regression coverage for `validate_solver_model_output_rejects_unsatisfied_models`.
fn validate_solver_model_output_rejects_unsatisfied_models() {
    let output = "\
sat
(
  (define-fun calldata_0 () (_ BitVec 8)
    #x00)
)
";
    let constraints =
        vec![BoolExpr::eq(Expr::Var("calldata_0".to_string()), Expr::Const(U256::from(1)))];

    let err = validate_solver_model_output(output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
/// Regression coverage for `fallback_model_finds_wrapping_arithmetic_riddle_candidate`.
fn fallback_model_finds_wrapping_arithmetic_riddle_candidate() {
    let var = Expr::Var("calldata_0".to_string());
    let msg_sender = U256::from_str_radix("1804c8ab1f12e6bbf3894d4083f33e07309d1f38", 16).unwrap();
    let constraints = vec![
        BoolExpr::cmp(
            BoolExprOp::Ult,
            Expr::op(ExprOp::Mul, var.clone(), var.clone()),
            Expr::Const(msg_sender),
        ),
        BoolExpr::cmp(BoolExprOp::Ugt, var.clone(), Expr::Const(msg_sender)),
        BoolExpr::eq(
            Expr::op(ExprOp::And, var.clone(), Expr::Const(U256::from(0x800))),
            Expr::Const(U256::ZERO),
        )
        .not(),
        BoolExpr::eq(
            Expr::op(ExprOp::And, var, Expr::Const(U256::from(0x10000))),
            Expr::Const(U256::ZERO),
        ),
    ];

    let model = fallback_single_var_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
}

#[test]
/// Regression coverage for `concrete_dynamic_array_return_uses_raw_abi_encoding`.
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
/// Regression coverage for `query_limit_is_enforced_before_spawning_solver`.
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
/// Regression coverage for `known_solver_names_resolve_to_smtlib_commands`.
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

    assert_eq!(command.program, "yices-smt2");
    assert_eq!(command.args, vec!["--bvconst-in-decimal"]);
    assert!(!command.smt_timeout);

    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "cvc5-int".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program, "cvc5");
    assert!(command.args.contains(&"--bv-print-consts-as-indexed-symbols".to_string()));
    assert!(!command.smt_timeout);

    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "bitwuzla-abs".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program, "bitwuzla");
    assert_eq!(command.args, vec!["--produce-models", "--abstraction"]);
    assert!(!command.smt_timeout);
}

#[test]
/// Regression coverage for `solver_command_overrides_solver_name`.
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
    assert_eq!(command.program, "custom-solver");
    assert_eq!(command.args, vec!["--flag", "two words"]);
    assert!(!command.smt_timeout);
}

#[test]
/// Regression coverage for `split_solver_command_preserves_empty_quoted_args`.
fn split_solver_command_preserves_empty_quoted_args() {
    let parts = split_solver_command(r#"custom-solver "" arg ''"#).unwrap();

    assert_eq!(parts, vec!["custom-solver", "", "arg", ""]);
}

#[test]
/// Regression coverage for `custom_solver_names_remain_z3_compatible`.
fn custom_solver_names_remain_z3_compatible() {
    let commands = solver_commands_for_config(&SymbolicConfig {
        solver: "/opt/solvers/z3-nightly".to_string(),
        ..Default::default()
    })
    .unwrap();
    let command = &commands[0];

    assert_eq!(command.program, "/opt/solvers/z3-nightly");
    assert_eq!(command.args, vec!["-in", "-smt2"]);
    assert!(command.smt_timeout);
}

#[test]
/// Regression coverage for `solver_portfolio_resolves_parallel_commands`.
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
    assert_eq!(commands[0].program, "z3");
    assert_eq!(commands[0].args, vec!["-in", "-smt2"]);
    assert!(commands[0].smt_timeout);
    assert_eq!(commands[1].program, "cvc5");
    assert!(commands[1].args.contains(&"--bv-print-consts-as-indexed-symbols".to_string()));
    assert_eq!(commands[2].program, "custom-wrapper");
    assert_eq!(commands[2].args, vec!["--stdin"]);
    assert!(!commands[2].smt_timeout);
}

#[test]
/// Regression coverage for `solver_portfolio_availability_warning_reports_missing_entries`.
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
#[test]
/// Regression coverage for `portfolio_sat_beats_early_unsat`.
fn portfolio_sat_beats_early_unsat() {
    let commands = vec![
        SolverCommand::new(
            vec!["/bin/sh".to_string(), "-c".to_string(), "printf 'unsat\n'".to_string()],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec!["/bin/sh".to_string(), "-c".to_string(), "sleep 0.1; printf 'sat\n'".to_string()],
            false,
        )
        .unwrap(),
    ];

    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, false);

    assert!(solver.is_sat(&[]).unwrap());
}

#[cfg(unix)]
#[test]
/// Regression coverage for `run_solver_commands` staged portfolio launching.
fn portfolio_winner_skips_delayed_solver() {
    let marker = std::env::temp_dir().join(format!(
        "foundry-symbolic-delayed-solver-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    let marker_arg = marker.display().to_string();
    let commands = vec![
        SolverCommand::new(vec!["/bin/echo".to_string(), "sat".to_string()], false).unwrap(),
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "printf started > \"$1\"; printf 'sat\n'".to_string(),
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
/// Regression coverage for delayed solvers still rescuing unresolved queries.
fn portfolio_delayed_solver_can_rescue_stalled_leader() {
    let commands = vec![
        SolverCommand::new(
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "sleep 0.3; printf 'unknown\n'".to_string(),
            ],
            false,
        )
        .unwrap(),
        SolverCommand::new(
            vec!["/bin/sh".to_string(), "-c".to_string(), "printf 'sat\n'".to_string()],
            false,
        )
        .unwrap(),
    ];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), Some(5), 1, false);
    let started_at = Instant::now();

    assert!(solver.is_sat(&[]).unwrap());
    assert!(started_at.elapsed() >= Duration::from_millis(100));
}

#[test]
/// Regression coverage for `PortfolioDiagnostics::record`.
fn portfolio_diagnostics_counts_staged_outcomes() {
    let summaries = vec![
        SolverRunSummary::new("primary".to_string(), Duration::from_millis(3), "sat-valid")
            .with_schedule(0, Duration::ZERO, Some(Duration::ZERO))
            .winner(),
        SolverRunSummary::new("secondary".to_string(), Duration::ZERO, "not-started")
            .with_schedule(1, Duration::from_millis(100), None),
        SolverRunSummary::new("rescue".to_string(), Duration::from_millis(5), "cancelled")
            .with_schedule(2, Duration::from_millis(500), Some(Duration::from_millis(500))),
        SolverRunSummary::new("bad".to_string(), Duration::from_millis(1), "sat-invalid")
            .with_schedule(3, Duration::from_millis(1000), Some(Duration::from_millis(1000))),
        SolverRunSummary::new("missing".to_string(), Duration::from_millis(1), "error")
            .with_schedule(4, Duration::from_millis(1500), Some(Duration::from_millis(1500))),
    ];
    let mut diagnostics = PortfolioDiagnostics::default();

    diagnostics.record(&summaries);

    assert_eq!(diagnostics.queries, 1);
    assert_eq!(diagnostics.solver_runs, 4);
    assert_eq!(diagnostics.rescue_runs, 3);
    assert_eq!(diagnostics.not_started, 1);
    assert_eq!(diagnostics.cancelled_after_winner, 1);
    assert_eq!(diagnostics.invalid_models, 1);
    assert_eq!(diagnostics.solver_errors, 1);
    assert_eq!(diagnostics.non_primary_wins, 0);
    assert_eq!(diagnostics.rescue_wins, 0);
    assert_eq!(diagnostics.winner_counts.get("primary"), Some(&1));
    assert_eq!(diagnostics.launch_counts.get("primary"), Some(&1));
    assert_eq!(diagnostics.launch_counts.get("secondary"), None);
    assert_eq!(diagnostics.launch_counts.get("rescue"), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get("sat-valid"), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get("not-started"), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get("cancelled"), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get("sat-invalid"), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get("error"), Some(&1));
}

#[test]
/// Regression coverage for `assertion_revert_classifies_assert_panic_only`.
fn assertion_revert_classifies_assert_panic_only() {
    let mut assert_payload = PANIC_SELECTOR.to_vec();
    assert_payload.extend_from_slice(&U256::from(1).to_be_bytes::<32>());

    let mut overflow_payload = PANIC_SELECTOR.to_vec();
    overflow_payload.extend_from_slice(&U256::from(0x11).to_be_bytes::<32>());

    assert!(is_assertion_revert(&assert_payload));
    assert!(!is_assertion_revert(&overflow_payload));
}

#[test]
/// Regression coverage for `assertion_revert_ignores_plain_require_reverts`.
fn assertion_revert_ignores_plain_require_reverts() {
    assert!(!is_assertion_revert(&error_payload("hit")));
}

#[test]
/// Regression coverage for `assertion_revert_accepts_forge_assertion_reverts`.
fn assertion_revert_accepts_forge_assertion_reverts() {
    assert!(is_assertion_revert(&error_payload("assertion failed: expected 1 to equal 2")));
}

/// Regression coverage for `error_payload`.
fn error_payload(message: &str) -> Vec<u8> {
    let mut payload = ERROR_SELECTOR.to_vec();
    payload.extend_from_slice(&U256::from(32).to_be_bytes::<32>());
    payload.extend_from_slice(&U256::from(message.len()).to_be_bytes::<32>());
    payload.extend_from_slice(message.as_bytes());
    let padded_len = message.len().div_ceil(32) * 32;
    payload.resize(4 + 64 + padded_len, 0);
    payload
}
