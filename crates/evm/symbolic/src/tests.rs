use super::{abi::*, runtime::*, *};

/// Regression coverage for `empty_state`.
fn empty_state() -> PathState {
    PathState::new(
        Address::ZERO,
        Address::ZERO,
        U256::ZERO,
        SymbolicCalldata {
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
/// Regression coverage for `precompile_number_for_spec`.
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
/// Regression coverage for `pop_worklist` respecting configured exploration order.
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
/// Regression coverage for local batches respecting configured exploration order.
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
    state.stack.push(SymWord::Expr(Expr::var("base"))).unwrap();

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
    let exponent = SymWord::Expr(Expr::var("exponent"));
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
    shl.stack.push(SymWord::Expr(Expr::var("shift"))).unwrap();
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
    shr.stack.push(SymWord::Expr(Expr::var("shift"))).unwrap();
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
    sar.stack.push(SymWord::Expr(Expr::var("shift"))).unwrap();
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
    state.stack.push(SymWord::Expr(Expr::var("den"))).unwrap();
    state.stack.push(SymWord::Expr(Expr::var("num"))).unwrap();

    state
        .bin_word_div_zero_guard(|a, b| if b.is_zero() { U256::ZERO } else { a / b }, ExprOp::UDiv)
        .unwrap();

    assert_eq!(
        state.stack.pop().unwrap(),
        SymWord::Expr(Expr::Ite(
            Box::new(BoolExpr::eq(Expr::var("den"), Expr::Const(U256::ZERO))),
            Box::new(Expr::Const(U256::ZERO)),
            Box::new(Expr::op(ExprOp::UDiv, Expr::var("num"), Expr::var("den"))),
        ))
    );
}

#[test]
/// Regression coverage for `symbolic_byte_extracts_with_concrete_index`.
fn symbolic_byte_extracts_with_concrete_index() {
    assert_eq!(
        byte_word(U256::from(0), SymWord::Expr(Expr::var("word"))),
        SymWord::Expr(Expr::op(
            ExprOp::And,
            Expr::op(ExprOp::Shr, Expr::var("word"), Expr::Const(U256::from(248))),
            Expr::Const(U256::from(0xff))
        ))
    );
}

#[test]
/// Regression coverage for `symbolic_byte_extracts_with_symbolic_index`.
fn symbolic_byte_extracts_with_symbolic_index() {
    let word = U256::from_be_bytes(core::array::from_fn::<_, 32, _>(|idx| idx as u8));
    let byte = byte_word_dynamic(SymWord::Expr(Expr::var("index")), SymWord::Concrete(word));

    let in_range = BTreeMap::from([("index".to_string(), U256::from(9))]);
    assert_eq!(model_word(&byte, &in_range).unwrap(), U256::from(9));

    let out_of_range = BTreeMap::from([("index".to_string(), U256::from(32))]);
    assert_eq!(model_word(&byte, &out_of_range).unwrap(), U256::ZERO);
}

#[test]
/// Regression coverage for `symbolic_signextend_accepts_symbolic_index`.
fn symbolic_signextend_accepts_symbolic_index() {
    let value = SymWord::Concrete(U256::from(0x80));
    let extended = signextend_word_dynamic(SymWord::Expr(Expr::var("index")), value);

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
        Expr::op(ExprOp::Shr, Expr::var("arg"), Expr::Const(U256::from(32))),
    ));

    assert_eq!(byte_word(U256::from(0), packed.clone()), SymWord::Concrete(U256::from(0x12)));
    assert_eq!(byte_word(U256::from(1), packed.clone()), SymWord::Concrete(U256::from(0x34)));
    assert_eq!(byte_word(U256::from(2), packed.clone()), SymWord::Concrete(U256::from(0x56)));
    assert_eq!(byte_word(U256::from(3), packed), SymWord::Concrete(U256::from(0x78)));
}

#[test]
/// Regression coverage for `word_reassembly_preserves_split_symbolic_word`.
fn word_reassembly_preserves_split_symbolic_word() {
    let original = Expr::op(ExprOp::Add, Expr::var("value"), Expr::Const(U256::from(1)));
    let bytes = word_bytes(SymWord::Expr(original.clone()));

    assert_eq!(word_from_bytes(bytes), SymWord::Expr(original));
}

#[test]
/// Regression coverage for `symbolic_address_aliases_match_abi_encoded_address_words`.
fn symbolic_address_aliases_match_abi_encoded_address_words() {
    let source = Expr::var("beneficiary");
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
        Expr::op(ExprOp::Shr, Expr::var("arg"), Expr::Const(U256::from(32))),
    );
    let selector_expr = Expr::op(ExprOp::Shr, call_word, Expr::Const(U256::from(224)));

    assert_eq!(expr_known_word(&selector_expr), Some(selector));
}

#[test]
/// Regression coverage for `selector_equality_folds_known_word_expressions`.
fn selector_equality_folds_known_word_expressions() {
    let selector = U256::from(0x12345678u32);
    let other = U256::from(0x9a8325a0u32);
    let call_word = Expr::op(
        ExprOp::Or,
        Expr::op(ExprOp::Shl, Expr::Const(selector), Expr::Const(U256::from(224))),
        Expr::op(ExprOp::Shr, Expr::var("arg"), Expr::Const(U256::from(32))),
    );
    let selector_expr = Expr::op(ExprOp::Shr, call_word, Expr::Const(U256::from(224)));

    assert_eq!(BoolExpr::eq(selector_expr.clone(), Expr::Const(selector)), BoolExpr::Const(true));
    assert_eq!(BoolExpr::eq(selector_expr, Expr::Const(other)), BoolExpr::Const(false));
}

#[test]
/// Regression coverage for `calldata_selector_load_simplifies_to_concrete_word`.
fn calldata_selector_load_simplifies_to_concrete_word() {
    let function = Function::parse("check(bytes32)").unwrap();
    let calldata = SymbolicCalldata::new(&function, &SymbolicConfig::default()).unwrap();
    let selector = U256::from_be_slice(function.selector().as_slice());
    let loaded = calldata.call_data().load_word(SymWord::zero()).unwrap();
    let selector_expr = Expr::op(ExprOp::Shr, loaded.into_expr(), Expr::Const(U256::from(224)));

    assert_eq!(expr_known_word(&selector_expr), Some(selector));
    assert_eq!(BoolExpr::eq(selector_expr, Expr::Const(selector)), BoolExpr::Const(true));
}

#[test]
/// Regression coverage for `artifact_json_fallback_paths`.
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
/// Regression coverage for `dynamic_calldata_encodes_bounded_bytes`.
fn dynamic_calldata_encodes_bounded_bytes() {
    let function = Function::parse("check(bytes)").unwrap();
    let config = SymbolicConfig { array_lengths: vec![3], ..Default::default() };
    let calldata = SymbolicCalldata::new(&function, &config).unwrap();

    assert_eq!(calldata.bytes.len(), 100);
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
    let loaded = calldata.load_word(SymWord::Expr(Expr::var("offset"))).unwrap();
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
    let size = SymWord::Expr(Expr::var("size"));
    let bounded_size = BoundedCopySize::Symbolic { size, max_size: 4 };
    let input = call_input_from_memory(&memory, SymWord::Concrete(U256::ZERO), &bounded_size);
    let calldata = calldata_from_call_input(input, &bounded_size);
    let model = BTreeMap::from([("size".to_string(), U256::from(2))]);

    assert_eq!(model_word(&calldata.size_word(), &model).unwrap(), U256::from(2));
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

    memory.copy_calldata_offset(0, SymWord::Expr(Expr::var("offset")), 2, &calldata).unwrap();
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
            SymWord::Expr(Expr::var("size")),
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
        SymWord::Expr(Expr::var("size")),
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
        SymWord::Expr(Expr::var("size")),
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
            SymWord::Expr(Expr::var("size")),
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
            SymWord::Expr(Expr::var("dest")),
            SymWord::Concrete(U256::from(0x20)),
            SymWord::Expr(Expr::var("size")),
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
            SymWord::Expr(Expr::var("size")),
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
    let bytes = return_data.read_bytes_offset(SymWord::Expr(Expr::var("offset")), 2);

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
            SymWord::Expr(Expr::var("len")),
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
        SymWord::Expr(Expr::var("len")),
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
    let value = SymWord::Expr(Expr::var("word"));

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
    let value = SymWord::Expr(Expr::var("word"));

    memory.store_word(7, value);
    let loaded = memory.load_word_offset(SymWord::Expr(Expr::var("offset"))).unwrap();

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
    let value = SymWord::Expr(Expr::var("word"));

    memory.store_word_offset(SymWord::Expr(Expr::var("offset")), value);
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

    memory.store_word_offset(SymWord::Expr(Expr::var("offset")), SymWord::Expr(Expr::var("word")));
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

    memory.store_word_offset(SymWord::Expr(Expr::var("offset")), SymWord::Expr(Expr::var("word")));
    memory.store_word(7, SymWord::Concrete(U256::from(0x55)));
    let loaded = memory.load_word(7).unwrap();

    let model = BTreeMap::from([
        ("offset".to_string(), U256::from(7)),
        ("word".to_string(), U256::from(0x1234)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x55));
}

#[test]
/// Regression coverage for `memory_dynamic_read_respects_concrete_overwrite_epoch`.
fn memory_dynamic_read_respects_concrete_overwrite_epoch() {
    let mut memory = SymMemory::default();

    memory.store_byte_offset(
        SymWord::Expr(Expr::var("write_offset")),
        SymWord::Expr(Expr::var("byte")),
    );
    memory.store_byte(5, SymWord::Concrete(U256::from(0x55)));
    let loaded = memory.byte_dynamic_with_delta(Expr::var("read_offset"), 0);

    let model = BTreeMap::from([
        ("write_offset".to_string(), U256::from(5)),
        ("read_offset".to_string(), U256::from(5)),
        ("byte".to_string(), U256::from(0xab)),
    ]);
    assert_eq!(model_word(&loaded, &model).unwrap(), U256::from(0x55));
}

#[test]
/// Regression coverage for `memory_store_byte_accepts_symbolic_offsets`.
fn memory_store_byte_accepts_symbolic_offsets() {
    let mut memory = SymMemory::default();

    memory.store_byte_offset(SymWord::Expr(Expr::var("offset")), SymWord::Expr(Expr::var("byte")));
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
    let value = SymWord::Expr(Expr::var("word"));

    memory.store_word(7, value);
    let loaded = word_from_bytes(memory.read_bytes_offset(SymWord::Expr(Expr::var("offset")), 32));

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
    let value = SymWord::Expr(Expr::var("word"));

    memory.store_word(7, value);
    let return_data = memory.return_data(SymWord::Expr(Expr::var("offset")), 32).unwrap();
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
    let value = SymWord::Expr(Expr::var("word"));

    memory.store_word(7, value);
    memory.copy_memory_offset(64, SymWord::Expr(Expr::var("src")), 32).unwrap();
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
    let value = SymWord::Expr(Expr::var("word"));

    memory.store_word(7, value);
    memory
        .copy_memory_to_offset(
            SymWord::Expr(Expr::var("dest")),
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
    let return_data =
        SymReturnData::from_symbolic_bytes(word_bytes(SymWord::Expr(Expr::var("word"))));

    memory
        .copy_call_output_offset(
            SymWord::Expr(Expr::var("dest")),
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
            &BoundedCopySize::Symbolic { size: SymWord::Expr(Expr::var("size")), max_size: 4 },
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
            SymWord::Expr(Expr::var("dest")),
            &BoundedCopySize::Symbolic { size: SymWord::Expr(Expr::var("size")), max_size: 4 },
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
        SymWord::Expr(Expr::var("len")),
    );
    let mut memory = SymMemory::default();
    memory.store_bytes(0x80, vec![SymWord::Concrete(U256::from(0xaa)); 4]);

    memory
        .copy_call_output_offset(
            SymWord::Expr(Expr::var("dest")),
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
    let salt = SymWord::Expr(Expr::var("salt"));
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
    let salt = SymWord::Expr(Expr::var("salt"));
    let initcode_hash = SymWord::Expr(Expr::var("initcode_hash"));

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
    let nonce = SymWord::Expr(Expr::var("nonce"));

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
    let deployer = SymWord::Expr(Expr::var("deployer"));
    let nonce = SymWord::Expr(Expr::var("nonce"));

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
    let deployer = SymWord::Expr(Expr::var("deployer"));
    let salt = SymWord::Expr(Expr::var("salt"));
    let initcode_hash = SymWord::Expr(Expr::var("initcode_hash"));

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
        topics: vec![SymWord::Expr(Expr::var("topic"))],
        data_len: SymWord::Concrete(U256::from(2)),
        data: vec![SymWord::Concrete(U256::from(0x12)), SymWord::Expr(Expr::var("byte"))],
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
        SymWord::Expr(Expr::var("len")),
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
    let word = SymWord::Expr(Expr::var("create2_address"));
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
    let salt = SymWord::Expr(Expr::var("salt"));
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
    let data = SymReturnData::from_symbolic_bytes(vec![SymWord::Expr(Expr::var("runtime_byte"))]);

    let code = data.to_code().unwrap();

    assert_eq!(code.read_bytes(0, 1), vec![SymWord::Expr(Expr::var("runtime_byte"))]);
}

#[test]
/// Regression coverage for `symbolic_runtime_size_is_not_installed_as_concrete_code`.
fn symbolic_runtime_size_is_not_installed_as_concrete_code() {
    let data = SymReturnData::from_symbolic_bytes_with_len(
        vec![SymWord::Concrete(U256::from(opcode::STOP))],
        SymWord::Expr(Expr::var("runtime_len")),
    );

    assert!(matches!(
        data.to_code(),
        Err(SymbolicError::Unsupported("CREATE with symbolic runtime size not modeled"))
    ));
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
    let initcode = SymCode::from_symbolic_bytes(vec![
        SymWord::Concrete(U256::from(opcode::STOP)),
        SymWord::Expr(Expr::var("constructor_arg_byte")),
    ]);

    memory.copy_symbolic(0, initcode.read_bytes(0, 2));

    assert_eq!(memory.byte(0), SymWord::Concrete(U256::from(opcode::STOP)));
    assert_eq!(memory.byte(1), SymWord::Expr(Expr::var("constructor_arg_byte")));
}

#[test]
/// Regression coverage for `symbolic_codecopy_accepts_symbolic_offsets`.
fn symbolic_codecopy_accepts_symbolic_offsets() {
    let code = SymCode::from_symbolic_bytes(
        (0u8..40).map(|idx| SymWord::Concrete(U256::from(idx + 1))).collect(),
    );
    let mut memory = SymMemory::default();

    memory.copy_symbolic(0, code.read_bytes_offset(SymWord::Expr(Expr::var("offset")), 2));
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
        vec![SymWord::Concrete(U256::from(opcode::STOP)), SymWord::Expr(Expr::var("arg"))],
    );
    let initcode = SymCode::from_memory_offset(&memory, SymWord::Expr(Expr::var("offset")), 2);
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
    let offset = SymWord::Expr(Expr::var("offset"));

    state.constraints.push(BoolExpr::eq(offset.clone().into_expr(), Expr::Const(U256::from(7))));

    assert_eq!(state.constrained_usize(&offset), Some(7));
}

#[test]
/// Regression coverage for `path_state_extracts_symbolic_usize_upper_bound`.
fn path_state_extracts_symbolic_usize_upper_bound() {
    let mut state = PathState::empty(Address::ZERO, Address::ZERO, false);
    let size = SymWord::Expr(Expr::var("size"));

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
    let offset = SymWord::Expr(Expr::var("offset"));
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
    let value = SymWord::Expr(Expr::var("value"));
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
    let code = SymCode::from_symbolic_bytes(vec![
        SymWord::Concrete(U256::from(opcode::PUSH2)),
        SymWord::Expr(Expr::var("immutable_hi")),
        SymWord::Expr(Expr::var("immutable_lo")),
    ]);

    let word = word_from_bytes(
        std::iter::repeat_with(SymWord::zero).take(30).chain(code.read_bytes(1, 2)),
    );

    assert!(matches!(word, SymWord::Expr(_)));
}

#[test]
/// Regression coverage for `abi_bytes_return_encodes_symbolic_bytes`.
fn abi_bytes_return_encodes_symbolic_bytes() {
    let ret = abi_bytes_return(vec![
        SymWord::Expr(Expr::var("calldata_byte_0")),
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
    assert_eq!(ret.byte(64), SymWord::Expr(Expr::var("calldata_byte_0")));
    assert_eq!(ret.byte(65), SymWord::Concrete(U256::from(0x42)));
}

#[test]
/// Regression coverage for `abi_bytes_return_can_encode_symbolic_length`.
fn abi_bytes_return_can_encode_symbolic_length() {
    let ret = abi_bytes_return_with_len(
        SymWord::Expr(Expr::var("len")),
        vec![SymWord::Expr(Expr::var("byte_0")), SymWord::Expr(Expr::var("byte_1"))],
    );

    assert_eq!(
        word_from_bytes((0..32).map(|idx| ret.byte(idx))),
        SymWord::Concrete(U256::from(32))
    );
    assert_eq!(word_from_bytes((32..64).map(|idx| ret.byte(idx))), SymWord::Expr(Expr::var("len")));
    assert_eq!(ret.byte(64), SymWord::Expr(Expr::var("byte_0")));
    assert_eq!(ret.byte(65), SymWord::Expr(Expr::var("byte_1")));
}

#[test]
/// Regression coverage for `symbolic_keccak_is_deterministic_for_same_symbolic_bytes`.
fn symbolic_keccak_is_deterministic_for_same_symbolic_bytes() {
    let bytes = word_bytes(SymWord::Expr(Expr::var("slot_key")));

    let first = keccak_word(bytes.clone());
    let second = keccak_word(bytes);

    assert_eq!(first, second);
    assert!(matches!(first, SymWord::Expr(Expr::Keccak(_))));
}

#[test]
/// Regression coverage for `symbolic_keccak_tracks_symbolic_length`.
fn symbolic_keccak_tracks_symbolic_length() {
    let bytes = vec![
        SymWord::Expr(Expr::var("byte_0")),
        SymWord::Expr(Expr::var("byte_1")),
        SymWord::zero(),
    ];
    let len = SymWord::Expr(Expr::var("len"));

    let word = keccak_word_with_len(bytes, len);

    let SymWord::Expr(Expr::Keccak(hash)) = word else {
        panic!("expected symbolic keccak term");
    };
    assert_eq!(*hash.len, Expr::var("len"));
    assert_eq!(hash.bytes.len(), 3);
}

#[test]
/// Regression coverage for `model_word_computes_symbolic_keccak_from_model`.
fn model_word_computes_symbolic_keccak_from_model() {
    let owner = Address::from([0x11; 20]);
    let slot = U256::from(1);
    let mut bytes = word_bytes(SymWord::Expr(Expr::var("owner")));
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
    let input = vec![SymWord::Expr(Expr::var("input_0")), SymWord::Expr(Expr::var("input_1"))];

    let input_len = SymWord::Concrete(U256::from(input.len()));
    let sha = execute_symbolic_precompile(
        precompile_address(2),
        input.clone(),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let sha_again = execute_symbolic_precompile(
        precompile_address(2),
        input.clone(),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let sha_word = word_from_bytes((0..32).map(|idx| sha.byte(idx)));
    let sha_again_word = word_from_bytes((0..32).map(|idx| sha_again.byte(idx)));

    assert_eq!(sha.len, 32);
    assert_eq!(sha_word, sha_again_word);
    assert!(matches!(sha_word, SymWord::Expr(Expr::Hash(hash)) if hash.algorithm == "sha256"));

    let ecrecover = execute_symbolic_precompile(
        precompile_address(1),
        input.clone(),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let ecrecover_again = execute_symbolic_precompile(
        precompile_address(1),
        input.clone(),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();

    assert_eq!(ecrecover.len, 32);
    for idx in 0..12 {
        assert_eq!(ecrecover.byte(idx), SymWord::zero());
    }
    for idx in 0..32 {
        assert_eq!(ecrecover.byte(idx), ecrecover_again.byte(idx));
    }

    let ripemd = execute_symbolic_precompile(
        precompile_address(3),
        input.clone(),
        input_len.clone(),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let ripemd_again =
        execute_symbolic_precompile(precompile_address(3), input, input_len, SpecId::CANCUN)
            .unwrap()
            .unwrap();

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
    let input_len = SymWord::Expr(Expr::var("size"));
    let return_data = execute_symbolic_precompile(
        precompile_address(4),
        input,
        input_len.clone(),
        SpecId::CANCUN,
    )
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
    modexp_input[96] = SymWord::Expr(Expr::var("base"));
    modexp_input[97] = SymWord::Concrete(U256::from(5));
    modexp_input[98] = SymWord::Concrete(U256::from(13));

    let modexp = execute_symbolic_precompile(
        precompile_address(5),
        modexp_input.clone(),
        SymWord::Concrete(U256::from(modexp_input.len())),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    let modexp_again = execute_symbolic_precompile(
        precompile_address(5),
        modexp_input.clone(),
        SymWord::Concrete(U256::from(modexp_input.len())),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(modexp.len, 1);
    assert_eq!(modexp.byte(0), modexp_again.byte(0));

    let mut blake_input = vec![SymWord::Expr(Expr::var("blake_input")); 213];
    blake_input[212] = SymWord::zero();
    let blake = execute_symbolic_precompile(
        precompile_address(9),
        blake_input,
        SymWord::Concrete(U256::from(213)),
        SpecId::CANCUN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(blake.len, 64);
}

#[test]
/// Regression coverage for `validity_sensitive_symbolic_precompiles_report_incomplete`.
fn validity_sensitive_symbolic_precompiles_report_incomplete() {
    let bn_input = vec![SymWord::Expr(Expr::var("point")); 128];
    let err = execute_symbolic_precompile(
        precompile_address(6),
        bn_input,
        SymWord::Concrete(U256::from(128)),
        SpecId::CANCUN,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SymbolicError::Unsupported("symbolic bn254 precompile validity not modeled")
    ));

    let blake_input = vec![SymWord::Expr(Expr::var("blake_input")); 213];
    let err = execute_symbolic_precompile(
        precompile_address(9),
        blake_input,
        SymWord::Concrete(U256::from(213)),
        SpecId::CANCUN,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SymbolicError::Unsupported("symbolic blake2f precompile final flag not modeled")
    ));
}

#[test]
/// Regression coverage for `symbolic_storage_read_after_write_accepts_symbolic_keys`.
fn symbolic_storage_read_after_write_accepts_symbolic_keys() {
    let address = Address::from([0x11; 20]);
    let key = SymWord::Expr(Expr::var("slot"));
    let value = SymWord::Expr(Expr::var("value"));
    let writes = vec![StorageWrite::new(address, key.clone(), value.clone())];

    assert_eq!(read_storage_writes(&writes, address, key, SymWord::zero()), value);
}

#[test]
/// Regression coverage for `symbolic_storage_uses_conditional_value_for_maybe_equal_key`.
fn symbolic_storage_uses_conditional_value_for_maybe_equal_key() {
    let address = Address::from([0x11; 20]);
    let write_key = SymWord::Expr(Expr::var("write_slot"));
    let read_key = SymWord::Expr(Expr::var("read_slot"));
    let value = SymWord::Expr(Expr::var("value"));
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
    let owner = SymWord::Expr(Expr::var("owner"));
    let base = keccak_word(word_bytes(owner));
    let left = add_words(base.clone(), SymWord::Expr(Expr::var("left_index")));
    let right = add_words(base, SymWord::Expr(Expr::var("right_index")));

    assert_eq!(
        storage_key_eq(left, right),
        BoolExpr::eq(Expr::var("left_index"), Expr::var("right_index"))
    );
}

#[test]
/// Regression coverage for `symbolic_storage_key_equality_expands_distinct_keccak_bases`.
fn symbolic_storage_key_equality_expands_distinct_keccak_bases() {
    let left_base = keccak_word(vec![SymWord::Expr(Expr::var("left_owner"))]);
    let right_base = keccak_word(vec![SymWord::Expr(Expr::var("right_owner"))]);
    let index = SymWord::Expr(Expr::var("index"));

    let condition =
        storage_key_eq(add_words(left_base, index.clone()), add_words(right_base, index));

    assert_eq!(condition, BoolExpr::eq(Expr::var("left_owner"), Expr::var("right_owner")));
}

#[test]
/// Regression coverage for `symbolic_storage_key_equality_rejects_concrete_plain_slot_alias`.
fn symbolic_storage_key_equality_rejects_concrete_plain_slot_alias() {
    let owner = SymWord::Expr(Expr::var("owner"));
    let layout_key = add_words(keccak_word(word_bytes(owner)), SymWord::Concrete(U256::ZERO));

    assert_eq!(storage_key_eq(layout_key, SymWord::Concrete(U256::ZERO)), BoolExpr::Const(false));
}

#[test]
/// Regression coverage for `nested_mapping_key_does_not_alias_plain_mapping_key_under_model`.
fn nested_mapping_key_does_not_alias_plain_mapping_key_under_model() {
    let owner = SymWord::Expr(Expr::var("owner"));
    let spender = SymWord::Expr(Expr::var("spender"));
    let recipient = SymWord::Expr(Expr::var("recipient"));

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
/// Regression coverage for `symbolic_world_tracks_current_transaction_created_accounts`.
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
        signextend_word(U256::ZERO, SymWord::Expr(Expr::var("word"))),
        SymWord::Expr(Expr::Ite(
            Box::new(BoolExpr::eq(
                Expr::op(ExprOp::And, Expr::var("word"), Expr::Const(U256::from(0x80))),
                Expr::Const(U256::ZERO)
            )),
            Box::new(Expr::op(ExprOp::And, Expr::var("word"), Expr::Const(U256::from(0x7f)))),
            Box::new(Expr::op(ExprOp::Or, Expr::var("word"), Expr::Const(!U256::from(0x7f)))),
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
    let constraints = vec![BoolExpr::eq(Expr::var("calldata_0"), Expr::Const(U256::from(1)))];

    let err = validate_solver_model_output(output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
/// Regression coverage for `fallback_model_finds_wrapping_arithmetic_riddle_candidate`.
fn fallback_model_finds_wrapping_arithmetic_riddle_candidate() {
    let var = Expr::var("calldata_0");
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
/// Regression coverage for exact arithmetic expression simplification.
fn expression_op_simplifies_exact_arithmetic_identities() {
    let x = Expr::var("x");

    assert_eq!(Expr::op(ExprOp::Mul, x.clone(), Expr::Const(U256::ZERO)), Expr::Const(U256::ZERO));
    assert_eq!(Expr::op(ExprOp::Mul, Expr::Const(U256::from(1)), x.clone()), x);
    assert_eq!(Expr::op(ExprOp::UDiv, Expr::var("x"), Expr::Const(U256::from(1))), Expr::var("x"));
    assert_eq!(
        Expr::op(ExprOp::URem, Expr::var("x"), Expr::Const(U256::from(1))),
        Expr::Const(U256::ZERO)
    );
    assert_eq!(Expr::op(ExprOp::Sub, Expr::var("x"), Expr::var("x")), Expr::Const(U256::ZERO));
    assert_eq!(Expr::op(ExprOp::And, Expr::var("x"), Expr::Const(U256::MAX)), Expr::var("x"));
    assert_eq!(Expr::op(ExprOp::And, x.clone(), x.clone()), x);
    assert_eq!(
        Expr::op(
            ExprOp::And,
            Expr::op(
                ExprOp::And,
                Expr::var("x"),
                Expr::Const((U256::from(1) << 160) - U256::from(1))
            ),
            Expr::Const((U256::from(1) << 160) - U256::from(1))
        ),
        Expr::op(ExprOp::And, Expr::var("x"), Expr::Const((U256::from(1) << 160) - U256::from(1)))
    );
    assert_eq!(
        Expr::op(ExprOp::Mul, Expr::Const(U256::from(6)), Expr::Const(U256::from(7))),
        Expr::Const(U256::from(42))
    );
}

#[test]
/// Regression coverage for reflexive comparison simplification.
fn bool_comparison_folds_reflexive_operands() {
    let x = || Expr::var("x");

    assert_eq!(BoolExpr::cmp(BoolExprOp::Ult, x(), x()), BoolExpr::Const(false));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Ugt, x(), x()), BoolExpr::Const(false));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Ule, x(), x()), BoolExpr::Const(true));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Uge, x(), x()), BoolExpr::Const(true));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Slt, x(), x()), BoolExpr::Const(false));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Sgt, x(), x()), BoolExpr::Const(false));
}

#[test]
/// Regression coverage for unsigned min/max comparison simplification.
fn bool_comparison_folds_unsigned_boundaries() {
    let x = || Expr::var("x");

    assert_eq!(
        BoolExpr::cmp(BoolExprOp::Ugt, Expr::Const(U256::ZERO), x()),
        BoolExpr::Const(false)
    );
    assert_eq!(BoolExpr::cmp(BoolExprOp::Ule, Expr::Const(U256::ZERO), x()), BoolExpr::Const(true));
    assert_eq!(
        BoolExpr::cmp(BoolExprOp::Ult, x(), Expr::Const(U256::ZERO)),
        BoolExpr::Const(false)
    );
    assert_eq!(BoolExpr::cmp(BoolExprOp::Uge, x(), Expr::Const(U256::ZERO)), BoolExpr::Const(true));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Ult, Expr::Const(U256::MAX), x()), BoolExpr::Const(false));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Uge, Expr::Const(U256::MAX), x()), BoolExpr::Const(true));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Ugt, x(), Expr::Const(U256::MAX)), BoolExpr::Const(false));
    assert_eq!(BoolExpr::cmp(BoolExprOp::Ule, x(), Expr::Const(U256::MAX)), BoolExpr::Const(true));
}

#[test]
/// Regression coverage for exact `ADDMOD`/`MULMOD` edge-case semantics.
fn exact_modular_arithmetic_handles_zero_modulus_and_wide_intermediates() {
    assert_eq!(addmod_word(U256::MAX, U256::from(2), U256::ZERO), U256::ZERO);
    assert_eq!(mulmod_word(U256::MAX, U256::MAX, U256::ZERO), U256::ZERO);

    assert_eq!(addmod_word(U256::MAX, U256::from(2), U256::MAX), U256::from(2));
    assert_eq!(mulmod_word(U256::MAX, U256::MAX, U256::MAX), U256::ZERO);

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert_eq!(
        eval_expr(
            &Expr::addmod(Expr::var("a"), Expr::Const(U256::from(2)), Expr::Const(U256::MAX)),
            &model,
        )
        .unwrap(),
        U256::from(2)
    );
    assert_eq!(
        eval_expr(&Expr::mulmod(Expr::var("a"), Expr::var("a"), Expr::Const(U256::MAX)), &model,)
            .unwrap(),
        U256::ZERO
    );
}

#[test]
/// Regression coverage for exact modular arithmetic SMT emission widening intermediates.
fn exact_modular_arithmetic_smt_widens_before_modulo() {
    let addmod = Expr::addmod(Expr::var("a"), Expr::Const(U256::from(2)), Expr::var("m")).smt();
    let mulmod = Expr::mulmod(Expr::var("a"), Expr::var("b"), Expr::var("m")).smt();

    assert!(addmod.contains("((_ zero_extend 256) a)"));
    assert!(addmod.contains("bvadd"));
    assert!(addmod.contains("bvurem"));
    assert!(mulmod.contains("((_ zero_extend 256) b)"));
    assert!(mulmod.contains("bvmul"));
    assert!(mulmod.contains("((_ extract 255 0)"));
}

#[test]
/// Regression coverage for rejecting old wrapping-intermediate false witnesses.
fn exact_addmod_model_validation_rejects_wrapping_false_pass() {
    let constraints = vec![BoolExpr::eq(
        Expr::addmod(Expr::var("a"), Expr::Const(U256::from(2)), Expr::Const(U256::MAX)),
        Expr::Const(U256::from(1)),
    )];
    let output = format!("sat\n((define-fun a () (_ BitVec 256) #x{}))\n", "f".repeat(64));

    let err = validate_solver_model_output(&output, &constraints).unwrap_err();

    assert!(err.to_string().contains("does not satisfy path constraints"));
}

#[test]
/// Regression coverage for exact unsigned-division zero predicate normalization.
fn solver_normalizes_udiv_zero_predicates_without_bvudiv() {
    let numerator = Expr::var("numerator");
    let denominator = Expr::var("denominator");
    let div = Expr::op(ExprOp::UDiv, numerator, denominator);
    let original = BoolExpr::eq(div, Expr::Const(U256::ZERO));
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
            eval_bool_expr(&original, &model).unwrap(),
            eval_bool_expr(&normalized, &model).unwrap(),
            "num={num} den={den}"
        );
    }
}

#[test]
/// Regression coverage for exact unsigned-division nonzero predicate normalization.
fn solver_normalizes_udiv_nonzero_predicates_without_bvudiv() {
    let numerator = Expr::var("numerator");
    let denominator = Expr::var("denominator");
    let div = Expr::op(ExprOp::UDiv, numerator, denominator);
    let original = BoolExpr::cmp(BoolExprOp::Ugt, div, Expr::Const(U256::ZERO));
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
            eval_bool_expr(&original, &model).unwrap(),
            eval_bool_expr(&normalized, &model).unwrap(),
            "num={num} den={den}"
        );
    }
}

#[test]
/// Regression coverage for normalized constraint batches being compact and order-stable.
fn solver_normalizes_constraint_batches_by_flattening_and_deduping() {
    let x = Expr::var("x");
    let y = Expr::var("y");
    let a = BoolExpr::cmp(BoolExprOp::Ult, x.clone(), Expr::Const(U256::from(10)));
    let b = BoolExpr::eq(y.clone(), Expr::Const(U256::from(3)));
    let grouped = vec![
        BoolExpr::And(vec![b.clone(), BoolExpr::Const(true), a.clone()]),
        a.clone(),
        BoolExpr::And(vec![b.clone()]),
    ];

    let normalized = normalize_constraints_for_solver(&grouped);

    assert_eq!(normalized, vec![b, a]);

    let unsat = normalize_constraints_for_solver(&[
        BoolExpr::eq(x, y),
        BoolExpr::Const(false),
        BoolExpr::Const(true),
    ]);
    assert_eq!(unsat, vec![BoolExpr::Const(false)]);
}

#[test]
/// Regression coverage for ERC4626-style share predicates losing `bvudiv` before SMT.
fn solver_normalizes_erc4626_style_share_zero_predicate() {
    let assets = Expr::var("assets");
    let supply = Expr::var("supply");
    let total_assets = Expr::var("total_assets");
    let shares = Expr::op(ExprOp::UDiv, Expr::op(ExprOp::Mul, assets, supply), total_assets);
    let constraints = vec![BoolExpr::eq(shares, Expr::Const(U256::ZERO))];
    let normalized = normalize_constraints_for_solver(&constraints);

    assert_eq!(normalized.len(), 1);
    assert!(!normalized[0].smt().contains("bvudiv"));
    assert!(normalized[0].smt().contains("bvmul"));
}

#[test]
/// Regression coverage for rebuilding OR-ed extracted bytes before SMT emission.
fn solver_rebuilds_word_from_extracted_byte_terms() {
    let masked = Expr::op(ExprOp::And, Expr::var("word"), Expr::Const(U256::from(u64::MAX)));
    let rebuilt = normalize_expr_for_solver(
        word_from_bytes(word_bytes(SymWord::Expr(masked.clone()))).into_expr(),
    );

    assert_eq!(rebuilt, normalize_expr_for_solver(masked));
}

fn checked_mul_guard_word(zero_operand: &Expr, expected: &Expr) -> Expr {
    let operand_is_zero = BoolExpr::eq(zero_operand.clone(), Expr::Const(U256::ZERO));
    let checked_product = Expr::Ite(
        Box::new(operand_is_zero.clone()),
        Box::new(Expr::Const(U256::ZERO)),
        Box::new(Expr::op(
            ExprOp::UDiv,
            Expr::op(ExprOp::Mul, zero_operand.clone(), expected.clone()),
            zero_operand.clone(),
        )),
    );

    Expr::op(
        ExprOp::Or,
        SymWord::from_bool(operand_is_zero).into_expr(),
        SymWord::from_bool(BoolExpr::eq(checked_product, expected.clone())).into_expr(),
    )
}

#[test]
/// Regression coverage for Solidity checked-mul guard tautology normalization.
fn solver_normalizes_checked_mul_guard_for_bounded_operands() {
    let a = Expr::op(ExprOp::And, Expr::var("a"), Expr::Const(U256::from(u64::MAX)));
    let b = Expr::op(ExprOp::And, Expr::var("b"), Expr::Const(U256::from(u64::MAX)));
    let guard = checked_mul_guard_word(&a, &b);

    assert_eq!(
        normalize_bool_for_solver(BoolExpr::eq(guard, Expr::Const(U256::ZERO))),
        BoolExpr::Const(false)
    );
}

#[test]
/// Regression coverage for path-bounded Solidity checked-mul guard tautology normalization.
fn solver_normalizes_checked_mul_guard_from_path_upper_bound() {
    let a = Expr::var("a");
    let factor = Expr::Const(U256::from(1_000_000_000_000_000_000u128));
    let guard_is_false = BoolExpr::eq(checked_mul_guard_word(&a, &factor), Expr::Const(U256::ZERO));
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ule, a, Expr::Const(U256::from(1000))),
        guard_is_false.clone(),
    ];
    let normalized = normalize_constraints_for_solver(&constraints);

    assert_eq!(normalized, vec![BoolExpr::Const(false)]);

    for value in [U256::ZERO, U256::from(1), U256::from(1000)] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert!(!eval_bool_expr(&guard_is_false, &model).unwrap());
    }
}

#[test]
/// Regression coverage for guarded self-division boolean tautology normalization.
fn solver_normalizes_guarded_self_division_guard() {
    let a = Expr::var("a");
    let a_is_zero = BoolExpr::eq(a.clone(), Expr::Const(U256::ZERO));
    let checked_quotient = Expr::Ite(
        Box::new(a_is_zero.clone()),
        Box::new(Expr::Const(U256::ZERO)),
        Box::new(Expr::op(ExprOp::UDiv, a.clone(), a)),
    );
    let guard = Expr::op(
        ExprOp::Or,
        SymWord::from_bool(a_is_zero).into_expr(),
        SymWord::from_bool(BoolExpr::eq(checked_quotient, Expr::Const(U256::from(1)))).into_expr(),
    );
    let original = BoolExpr::eq(guard, Expr::Const(U256::ZERO));
    let normalized = normalize_bool_for_solver(original.clone());

    assert_eq!(normalized, BoolExpr::Const(false));

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert!(!eval_bool_expr(&original, &model).unwrap());
    }
}

#[test]
/// Regression coverage for guarded self-division word normalization.
fn solver_normalizes_guarded_self_division_word() {
    let a = Expr::var("a");
    let a_is_zero = BoolExpr::eq(a.clone(), Expr::Const(U256::ZERO));
    let checked_quotient = Expr::Ite(
        Box::new(a_is_zero.clone()),
        Box::new(Expr::Const(U256::ZERO)),
        Box::new(Expr::op(ExprOp::UDiv, a.clone(), a)),
    );
    let normalized = normalize_expr_for_solver(checked_quotient.clone());

    assert!(!normalized.smt().contains("bvudiv"));
    assert_eq!(
        normalized,
        Expr::Ite(
            Box::new(a_is_zero.not()),
            Box::new(Expr::Const(U256::from(1))),
            Box::new(Expr::Const(U256::ZERO)),
        )
    );

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert_eq!(
            eval_expr(&checked_quotient, &model).unwrap(),
            eval_expr(&normalized, &model).unwrap()
        );
    }
}

#[test]
/// Regression coverage for preserving mirrored zero-guarded self-division semantics.
fn solver_does_not_invert_guarded_zero_self_division() {
    let a = Expr::var("a");
    let a_is_zero = BoolExpr::eq(a.clone(), Expr::Const(U256::ZERO));
    let mirrored = Expr::Ite(
        Box::new(a_is_zero),
        Box::new(Expr::op(ExprOp::UDiv, a.clone(), a)),
        Box::new(Expr::Const(U256::ZERO)),
    );
    let normalized = normalize_expr_for_solver(mirrored.clone());

    for value in [U256::ZERO, U256::from(1), U256::from(2), U256::MAX] {
        let model = BTreeMap::from([("a".to_string(), value)]);
        assert_eq!(eval_expr(&mirrored, &model).unwrap(), eval_expr(&normalized, &model).unwrap());
    }
}

#[test]
/// Regression coverage for guarded self-division overflow-guard normalization.
fn solver_normalizes_guarded_self_division_add_overflow_guard() {
    let a = Expr::var("a");
    let a_is_zero = BoolExpr::eq(a.clone(), Expr::Const(U256::ZERO));
    let checked_quotient = Expr::Ite(
        Box::new(a_is_zero),
        Box::new(Expr::Const(U256::ZERO)),
        Box::new(Expr::op(ExprOp::UDiv, a.clone(), a)),
    );

    assert_eq!(
        normalize_bool_for_solver(BoolExpr::cmp(
            BoolExprOp::Ugt,
            checked_quotient.clone(),
            Expr::op(ExprOp::Add, Expr::Const(U256::from(1)), checked_quotient),
        )),
        BoolExpr::Const(false)
    );
}

#[test]
/// Regression coverage for checked-mul guards proven by normalized path bounds.
fn solver_normalizes_checked_mul_guard_with_context_bound() {
    let a = Expr::var("a");
    let scale = Expr::Const(U256::from(1_000_000_000_000_000_000u128));
    let guard = checked_mul_guard_word(&a, &scale);
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, a.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, a, Expr::Const(U256::from(1000))).not(),
        BoolExpr::eq(guard, Expr::Const(U256::ZERO)),
    ];

    assert_eq!(normalize_constraints_for_solver(&constraints), vec![BoolExpr::Const(false)]);
}

#[test]
/// Regression coverage for preserving checked-mul guards without a useful path bound.
fn solver_does_not_context_normalize_checked_mul_guard_without_tight_bound() {
    let a = Expr::var("a");
    let scale = Expr::Const(U256::from(1_000_000_000_000_000_000u128));
    let guard_is_zero = BoolExpr::eq(checked_mul_guard_word(&a, &scale), Expr::Const(U256::ZERO));

    assert_ne!(
        normalize_constraints_for_solver(std::slice::from_ref(&guard_is_zero)),
        vec![BoolExpr::Const(false)]
    );
    assert_ne!(
        normalize_constraints_for_solver(&[
            BoolExpr::cmp(BoolExprOp::Ule, a, Expr::Const(U256::MAX)),
            guard_is_zero.clone(),
        ]),
        vec![BoolExpr::Const(false)]
    );

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert!(eval_bool_expr(&guard_is_zero, &model).unwrap());
}

#[test]
/// Regression coverage for preserving unbounded checked-mul overflow guards.
fn solver_does_not_normalize_unbounded_checked_mul_guard_to_tautology() {
    let a = Expr::var("a");
    let b = Expr::var("b");
    let original = BoolExpr::eq(checked_mul_guard_word(&a, &b), Expr::Const(U256::ZERO));
    let normalized = normalize_bool_for_solver(original.clone());

    assert_ne!(normalized, BoolExpr::Const(false));

    let model = BTreeMap::from([("a".to_string(), U256::MAX), ("b".to_string(), U256::from(2))]);
    assert!(eval_bool_expr(&original, &model).unwrap());
    assert_eq!(
        eval_bool_expr(&original, &model).unwrap(),
        eval_bool_expr(&normalized, &model).unwrap()
    );
}

#[test]
/// Regression coverage for preserving path-bounded checked-mul guards with loose bounds.
fn solver_does_not_normalize_checked_mul_guard_when_path_bound_can_overflow() {
    let a = Expr::var("a");
    let factor = Expr::Const(U256::from(2));
    let original = BoolExpr::eq(checked_mul_guard_word(&a, &factor), Expr::Const(U256::ZERO));
    let normalized = normalize_constraints_for_solver(&[
        BoolExpr::cmp(BoolExprOp::Ule, a, Expr::Const(U256::MAX)),
        original.clone(),
    ]);

    assert!(!matches!(normalized.as_slice(), [BoolExpr::Const(false)]));

    let model = BTreeMap::from([("a".to_string(), U256::MAX)]);
    assert!(eval_bool_expr(&original, &model).unwrap());
    assert!(normalized.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
}

#[test]
/// Regression coverage for checked-add overflow guards over bounded expressions.
fn solver_normalizes_checked_add_overflow_guard_for_bounded_operands() {
    let a = Expr::op(ExprOp::And, Expr::var("a"), Expr::Const(U256::from(u64::MAX)));
    let b = Expr::op(
        ExprOp::UDiv,
        Expr::op(
            ExprOp::Mul,
            Expr::op(ExprOp::And, Expr::var("b"), Expr::Const(U256::from(u64::MAX))),
            a.clone(),
        ),
        Expr::op(ExprOp::And, Expr::var("denominator"), Expr::Const(U256::MAX >> 64)),
    );

    assert_eq!(
        normalize_bool_for_solver(BoolExpr::cmp(
            BoolExprOp::Ugt,
            a.clone(),
            Expr::op(ExprOp::Add, a, b),
        )),
        BoolExpr::Const(false)
    );
}

#[test]
/// Regression coverage for preserving unbounded checked-add overflow guards.
fn solver_does_not_normalize_unbounded_checked_add_overflow_guard() {
    let a = Expr::var("a");
    let b = Expr::var("b");
    let original = BoolExpr::cmp(BoolExprOp::Ugt, a.clone(), Expr::op(ExprOp::Add, a, b));
    let normalized = normalize_bool_for_solver(original.clone());

    assert_ne!(normalized, BoolExpr::Const(false));

    let model = BTreeMap::from([("a".to_string(), U256::MAX), ("b".to_string(), U256::from(1))]);
    assert!(eval_bool_expr(&original, &model).unwrap());
    assert_eq!(
        eval_bool_expr(&original, &model).unwrap(),
        eval_bool_expr(&normalized, &model).unwrap()
    );
}

#[test]
/// Regression coverage for monotonic product contradictions over bounded operands.
fn solver_detects_monotonic_product_contradiction() {
    let ink = Expr::op(ExprOp::And, Expr::var("ink"), Expr::Const(U256::from(u64::MAX)));
    let art = Expr::op(ExprOp::And, Expr::var("art"), Expr::Const(U256::from(u64::MAX)));
    let spot = Expr::op(ExprOp::And, Expr::var("spot"), Expr::Const(U256::from(u64::MAX)));
    let rate = Expr::op(ExprOp::And, Expr::var("rate"), Expr::Const(U256::from(u64::MAX)));
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, ink.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, art.clone(), ink.clone()),
        BoolExpr::cmp(BoolExprOp::Ugt, spot.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, rate.clone(), spot.clone()),
        BoolExpr::cmp(
            BoolExprOp::Ult,
            Expr::op(ExprOp::Mul, ink, spot),
            Expr::op(ExprOp::Mul, art, rate),
        )
        .not(),
    ];

    assert!(product_monotonic_unsat(&constraints));
}

#[test]
/// Regression coverage for preserving satisfiable wrapping product inequalities.
fn solver_does_not_prune_wrapping_product_inequality() {
    let ink = Expr::var("ink");
    let art = Expr::var("art");
    let spot = Expr::var("spot");
    let rate = Expr::var("rate");
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, ink.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, art.clone(), ink.clone()),
        BoolExpr::cmp(BoolExprOp::Ugt, spot.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, rate.clone(), spot.clone()),
        BoolExpr::cmp(
            BoolExprOp::Ult,
            Expr::op(ExprOp::Mul, ink, spot),
            Expr::op(ExprOp::Mul, art, rate),
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
    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
}

#[test]
/// Regression coverage for detecting hard nonlinear arithmetic.
fn hard_arithmetic_detection_flags_symbolic_mul_div_and_mod() {
    let x = Expr::var("x");
    let y = Expr::var("y");

    assert!(expr_contains_hard_arith(&Expr::op(ExprOp::Mul, x.clone(), y.clone())));
    assert!(expr_contains_hard_arith(&Expr::op(ExprOp::UDiv, x.clone(), y.clone())));
    assert!(expr_contains_hard_arith(&Expr::op(ExprOp::URem, x.clone(), y)));
    assert!(!expr_contains_hard_arith(&Expr::op(ExprOp::Mul, x, Expr::Const(U256::from(1)))));
}

#[test]
/// Regression coverage for multi-variable hard arithmetic witness search.
fn hard_arithmetic_fallback_finds_multi_variable_candidate() {
    let first = Expr::var("first");
    let donation = Expr::var("donation");
    let second = Expr::var("second");
    let denominator = Expr::op(ExprOp::Add, first.clone(), donation.clone());
    let shares =
        Expr::op(ExprOp::UDiv, Expr::op(ExprOp::Mul, second.clone(), first.clone()), denominator);
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, first, Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, donation, Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, second, Expr::Const(U256::ZERO)),
        BoolExpr::eq(shares, Expr::Const(U256::ZERO)),
    ];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
}

#[test]
/// Regression coverage for MiniVat-style wrapping product witness search.
fn hard_arithmetic_fallback_finds_wrapping_product_inequality_candidate() {
    let ink = Expr::var("ink");
    let art = Expr::var("art");
    let spot = Expr::var("spot");
    let rate = Expr::var("rate");
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, ink.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, art.clone(), ink.clone()),
        BoolExpr::cmp(BoolExprOp::Ugt, spot.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, rate.clone(), spot.clone()),
        BoolExpr::cmp(
            BoolExprOp::Ult,
            Expr::op(ExprOp::Mul, ink, spot),
            Expr::op(ExprOp::Mul, art, rate),
        )
        .not(),
    ];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
}

#[test]
/// Regression coverage for solver-hard exact `MULMOD` witness search.
fn hard_arithmetic_fallback_finds_exact_mulmod_wide_intermediate_candidate() {
    let a = Expr::var("a");
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, a.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::eq(Expr::mulmod(a.clone(), a, Expr::Const(U256::MAX)), Expr::Const(U256::ZERO)),
    ];

    let model = hard_arith_fallback_model(&constraints).unwrap();

    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
    assert_eq!(model.get("a"), Some(&U256::MAX));
}

#[test]
/// Regression coverage for rejecting partial hard-arithmetic fallback models.
fn hard_arithmetic_fallback_rejects_unvalidated_partial_model() {
    let a = Expr::var("a");
    let b = Expr::var("b");
    let c = Expr::var("c");
    let d = Expr::var("d");
    let e = Expr::var("e");
    let f = Expr::var("f");
    let constraints = vec![
        BoolExpr::eq(Expr::op(ExprOp::Mul, a, b), Expr::Const(U256::from(1))),
        BoolExpr::eq(c, Expr::Const(U256::from(3))),
        BoolExpr::eq(d, Expr::Const(U256::from(4))),
        BoolExpr::eq(e, Expr::Const(U256::from(5))),
        BoolExpr::eq(f, Expr::Const(U256::from(6))),
    ];

    let Some(model) = hard_arith_fallback_model(&constraints) else { return };

    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
}

#[test]
/// Regression coverage for local hard arithmetic search avoiding unsupported hash symbols.
fn hard_arithmetic_fallback_skips_symbolic_hashes() {
    let x = Expr::var("x");
    let hash = Expr::hash("hash".to_string(), "sha256", vec![x.clone()]);
    let constraints =
        vec![BoolExpr::eq(Expr::op(ExprOp::Mul, x, hash), Expr::Const(U256::from(1)))];

    assert!(hard_arith_fallback_model(&constraints).is_none());
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
/// Regression coverage for `split_solver_command_rejects_unterminated_double_quote`.
fn split_solver_command_rejects_unterminated_double_quote() {
    let err = split_solver_command(r#"z3 "unterm"#).unwrap_err();

    assert!(
        matches!(err, SolverConfigError::UnterminatedQuote('"')),
        "expected UnterminatedQuote('\"'), got {err:?}"
    );
    assert_eq!(err.to_string(), r#"unterminated " quote in symbolic solver command"#);
}

#[test]
/// Regression coverage for `split_solver_command_rejects_unterminated_single_quote`.
fn split_solver_command_rejects_unterminated_single_quote() {
    let err = split_solver_command("z3 'unterm").unwrap_err();

    assert!(
        matches!(err, SolverConfigError::UnterminatedQuote('\'')),
        "expected UnterminatedQuote('\\''), got {err:?}"
    );
    assert_eq!(err.to_string(), "unterminated ' quote in symbolic solver command");
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
fn counted_solver_command(marker: &Path, response: &'static str) -> SolverCommand {
    SolverCommand::new(
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!(
                "count=$(cat \"$1\" 2>/dev/null || printf 0); \
                 count=$((count + 1)); printf '%s' \"$count\" > \"$1\"; \
                 cat >/dev/null; printf '{response}\\n'"
            ),
            "sh".to_string(),
            marker.display().to_string(),
        ],
        false,
    )
    .unwrap()
}

#[cfg(unix)]
/// Returns how many times a counted fake solver command was invoked.
fn counted_solver_invocations(marker: &Path) -> usize {
    std::fs::read_to_string(marker).ok().and_then(|count| count.parse().ok()).unwrap_or_default()
}

#[cfg(unix)]
#[test]
/// Regression coverage for hard-arithmetic `is_sat` fallback before SMT.
fn is_sat_uses_validated_hard_arithmetic_fallback_before_solver() {
    let marker = portfolio_test_marker("hard-arith-is-sat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, x.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, y.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::eq(Expr::op(ExprOp::Mul, x, y), Expr::Const(U256::from(4))),
    ];
    let normalized = normalize_constraints_for_solver(&constraints);
    let model = hard_arith_fallback_model(&normalized).unwrap();

    assert!(normalized.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap()));
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
/// Regression coverage for validated hard-arithmetic model results populating caches.
fn model_uses_validated_hard_arithmetic_fallback_cache() {
    let marker = portfolio_test_marker("hard-arith-model-cache");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let constraints = vec![
        BoolExpr::cmp(BoolExprOp::Ugt, x.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(BoolExprOp::Ugt, y.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::eq(Expr::op(ExprOp::Mul, x, y), Expr::Const(U256::from(4))),
    ];

    let first = solver.model(&constraints).unwrap();
    assert!(constraints.iter().all(|constraint| eval_bool_expr(constraint, &first).unwrap()));
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
/// Regression coverage for hard-arithmetic `is_sat` preserving solver `unsat`.
fn is_sat_hard_arithmetic_without_witness_still_honors_solver_unsat() {
    let marker = portfolio_test_marker("hard-arith-is-sat-unsat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let constraints = vec![
        BoolExpr::eq(x.clone(), Expr::Const(U256::ZERO)),
        BoolExpr::eq(Expr::op(ExprOp::Mul, x, y), Expr::Const(U256::from(1))),
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
/// Regression coverage for normalized satisfiability query cache hits.
fn sat_cache_reuses_normalized_is_sat_results() {
    let marker = portfolio_test_marker("sat-cache");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let constraints = vec![
        BoolExpr::Const(true),
        BoolExpr::eq(x.clone(), Expr::Const(U256::from(1))),
        BoolExpr::eq(y.clone(), Expr::Const(U256::from(2))),
    ];
    let reordered_constraints = vec![
        BoolExpr::eq(y, Expr::Const(U256::from(2))),
        BoolExpr::eq(x, Expr::Const(U256::from(1))),
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
/// Regression coverage for nested canonical satisfiability query cache keys.
fn sat_cache_reuses_nested_commutative_results() {
    let marker = portfolio_test_marker("sat-cache-canonical");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let constraints = vec![BoolExpr::and(vec![
        BoolExpr::eq(Expr::op(ExprOp::Add, x.clone(), y.clone()), Expr::Const(U256::from(3))),
        BoolExpr::eq(x.clone(), Expr::Const(U256::from(1))),
    ])];
    let reordered_constraints = vec![BoolExpr::and(vec![
        BoolExpr::eq(Expr::Const(U256::from(1)), x),
        BoolExpr::eq(Expr::Const(U256::from(3)), Expr::op(ExprOp::Add, y, Expr::var("x"))),
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
/// Regression coverage for duplicate conjuncts sharing satisfiability cache keys.
fn sat_cache_deduplicates_repeated_constraints() {
    let marker = portfolio_test_marker("sat-cache-dedup");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let constraint = BoolExpr::eq(x, Expr::Const(U256::from(1)));
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
/// Regression coverage for nested duplicate conjuncts sharing satisfiability cache keys.
fn sat_cache_deduplicates_nested_repeated_constraints() {
    let marker = portfolio_test_marker("sat-cache-nested-dedup");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let constraint = BoolExpr::eq(x, Expr::Const(U256::from(1)));
    let duplicated = vec![BoolExpr::And(vec![constraint.clone(), constraint.clone()])];
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
/// Regression coverage for grouped conjunctions sharing satisfiability cache keys.
fn sat_cache_flattens_grouped_conjunction_keys() {
    let marker = portfolio_test_marker("sat-cache-and-flatten");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let z = Expr::var("z");
    let a = BoolExpr::eq(x, Expr::Const(U256::from(1)));
    let b = BoolExpr::eq(y, Expr::Const(U256::from(2)));
    let c = BoolExpr::eq(z, Expr::Const(U256::from(3)));
    let grouped = vec![BoolExpr::And(vec![BoolExpr::And(vec![b.clone(), c.clone()]), a.clone()])];
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
/// Regression coverage for comparison-direction canonical satisfiability cache keys.
fn sat_cache_reuses_reversed_comparisons() {
    let marker = portfolio_test_marker("sat-cache-reversed-cmp");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 3, false);
    let x = Expr::var("x");
    let y = Expr::var("y");

    assert!(solver.is_sat(&[BoolExpr::cmp(BoolExprOp::Ugt, x.clone(), y.clone())]).unwrap());
    assert!(solver.is_sat(&[BoolExpr::cmp(BoolExprOp::Ult, y.clone(), x.clone())]).unwrap());
    assert!(solver.is_sat(&[BoolExpr::cmp(BoolExprOp::Uge, x.clone(), y.clone())]).unwrap());
    assert!(solver.is_sat(&[BoolExpr::cmp(BoolExprOp::Ule, y.clone(), x.clone())]).unwrap());
    assert!(solver.is_sat(&[BoolExpr::cmp(BoolExprOp::Sgt, x.clone(), y.clone())]).unwrap());
    assert!(solver.is_sat(&[BoolExpr::cmp(BoolExprOp::Slt, y, x)]).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 3);
    assert_eq!(stats.sat_queries, 6);
    assert_eq!(stats.sat_cache_hits, 3);
    assert_eq!(counted_solver_invocations(&marker), 3);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
/// Regression coverage for leaving unknown satisfiability results uncached.
fn sat_cache_does_not_cache_unknown_results() {
    let marker = portfolio_test_marker("sat-cache-unknown");
    let commands = vec![counted_solver_command(&marker, "unknown")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 4, false);
    let constraints = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)))];

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
/// Regression coverage for normalized solver model cache hits.
fn model_cache_reuses_normalized_model_results() {
    let marker = portfolio_test_marker("model-cache");
    let model_output = "sat\n\
        ((define-fun x () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000001)\n\
        (define-fun y () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000002))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x = Expr::var("x");
    let y = Expr::var("y");
    let constraints = vec![
        BoolExpr::Const(true),
        BoolExpr::eq(x.clone(), Expr::Const(U256::from(1))),
        BoolExpr::eq(y.clone(), Expr::Const(U256::from(2))),
    ];
    let reordered_constraints = vec![
        BoolExpr::eq(y, Expr::Const(U256::from(2))),
        BoolExpr::eq(x, Expr::Const(U256::from(1))),
    ];

    assert_eq!(solver.model(&constraints).unwrap().get("x"), Some(&U256::from(1)));
    assert_eq!(solver.model(&reordered_constraints).unwrap().get("y"), Some(&U256::from(2)));

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
/// Regression coverage for model queries populating the satisfiability cache.
fn model_query_populates_sat_cache() {
    let marker = portfolio_test_marker("model-cache-sat");
    let model_output = "sat\n\
        ((define-fun x () (_ BitVec 256) \
        #x0000000000000000000000000000000000000000000000000000000000000001))";
    let commands = vec![counted_solver_command(&marker, model_output)];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let constraints = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)))];

    assert_eq!(solver.model(&constraints).unwrap().get("x"), Some(&U256::from(1)));
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
/// Regression coverage for direct contradictions avoiding SMT calls.
fn direct_contradiction_is_sat_short_circuits_locally() {
    let mut solver = SmtLibSubprocessSolver::new(Ok(Vec::new()), None, 1, false);
    let constraint = BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)));
    let constraints = vec![constraint.clone(), constraint.not()];

    assert!(!solver.is_sat(&constraints).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 0);
    assert_eq!(stats.sat_queries, 1);
}

#[cfg(unix)]
#[test]
/// Regression coverage for reusing cached unsat subsets without another SMT query.
fn is_sat_reuses_cached_unsat_subset() {
    let marker = portfolio_test_marker("unsat-subset-cache");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 2, false);
    let x_eq_one = BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)));
    let y_eq_two = BoolExpr::eq(Expr::var("y"), Expr::Const(U256::from(2)));

    assert!(!solver.is_sat(std::slice::from_ref(&x_eq_one)).unwrap());
    assert!(!solver.is_sat(&[x_eq_one, y_eq_two]).unwrap());

    let stats = solver.stats();
    assert_eq!(stats.solver_queries, 1);
    assert_eq!(stats.smt_queries, 1);
    assert_eq!(stats.sat_queries, 2);
    assert_eq!(stats.sat_cache_hits, 1);
    assert_eq!(counted_solver_invocations(&marker), 1);
    let _ = std::fs::remove_file(&marker);
}

#[cfg(unix)]
#[test]
/// Regression coverage for not generalizing cached sat subsets to stricter constraints.
fn is_sat_does_not_reuse_cached_sat_subset() {
    let marker = portfolio_test_marker("sat-subset-cache");
    let commands = vec![counted_solver_command(&marker, "sat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let x_eq_one = BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)));
    let y_eq_two = BoolExpr::eq(Expr::var("y"), Expr::Const(U256::from(2)));

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
/// Regression coverage for satisfiability cache unsat results short-circuiting model queries.
fn model_short_circuits_when_sat_cache_proved_unsat() {
    let marker = portfolio_test_marker("model-cache-unsat");
    let commands = vec![counted_solver_command(&marker, "unsat")];
    let mut solver = SmtLibSubprocessSolver::new(Ok(commands), None, 1, false);
    let constraints = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)))];

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
/// Regression coverage for `portfolio_sat_beats_early_unsat`.
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
/// Regression coverage for adaptive portfolio scheduling promoting recent winners.
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

    let first = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)))];
    let second = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(2)))];

    assert!(solver.is_sat(&first).unwrap());
    assert!(marker.exists());
    let _ = std::fs::remove_file(&marker);

    assert!(solver.is_sat(&second).unwrap());
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
/// Regression coverage for adaptive portfolio scheduling promoting unsat winners.
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

    let first = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(1)))];
    let second = vec![BoolExpr::eq(Expr::var("x"), Expr::Const(U256::from(2)))];

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
/// Regression coverage for adaptive portfolio scheduling penalizing invalid models.
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
            BoolExpr::eq(Expr::var("calldata_0"), Expr::Const(U256::from(1))),
            BoolExpr::eq(Expr::var(format!("portfolio_query_{query}")), Expr::Const(U256::ZERO)),
        ];
        assert_eq!(solver.model(&constraints).unwrap().get("calldata_0"), Some(&U256::from(1)));
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
/// Regression coverage for `run_solver_commands` staged portfolio launching.
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
/// Regression coverage for delayed solvers still rescuing unresolved queries.
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
/// Regression coverage for deferring SMT and portfolio diagnostics during progress rendering.
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

#[test]
/// Regression coverage for `PortfolioDiagnostics::record`.
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
    assert_eq!(diagnostics.outcome_counts.get(&SolverOutcome::SatValid), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get(&SolverOutcome::NotStarted), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get(&SolverOutcome::Cancelled), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get(&SolverOutcome::SatInvalid), Some(&1));
    assert_eq!(diagnostics.outcome_counts.get(&SolverOutcome::Error), Some(&1));
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
