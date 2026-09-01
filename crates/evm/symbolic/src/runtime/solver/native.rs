use super::*;
use foundry_solver::{
    BinaryOp, ComparisonOp, Model as NativeModel, OpaqueKind, PredicateNode, QueryDagView,
    QueryRequest, SolveResult, TernaryOp, WordNode,
};
use std::hash::{Hash, Hasher};

pub(super) enum NativeSolveResult {
    Sat(SymbolicModel),
    Unsat,
    Unknown,
}

/// Solves directly over Foundry's hash-consed expressions. The adapter exposes borrowed node IDs;
/// it does not clone expressions, serialize SMT, or construct a second expression tree.
pub(super) fn solve_native(
    normalized_constraints: &[SymBoolExpr],
    original_constraints: &[SymBoolExpr],
    model: bool,
) -> NativeSolveResult {
    let request = if model { QueryRequest::Model } else { QueryRequest::Check };
    let query = FoundryQuery::new(normalized_constraints, request);
    match foundry_solver::solve(&query, |candidate| {
        model_satisfies_constraints(candidate, original_constraints)
    }) {
        SolveResult::Sat(model) => NativeSolveResult::Sat(model.into_values()),
        SolveResult::Unsat => NativeSolveResult::Unsat,
        SolveResult::Unknown => NativeSolveResult::Unknown,
    }
}

impl SymbolicModelLookup for NativeModel<Symbol> {
    fn value(&self, name: Symbol) -> Option<U256> {
        self.value(name)
    }
}

struct FoundryQuery<'a> {
    constraints: &'a [SymBoolExpr],
    request: QueryRequest,
}

impl<'a> FoundryQuery<'a> {
    const fn new(constraints: &'a [SymBoolExpr], request: QueryRequest) -> Self {
        Self { constraints, request }
    }
}

struct WordId<'a>(&'a SymExpr);

impl Clone for WordId<'_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for WordId<'_> {}

impl PartialEq for WordId<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.kind(), other.0.kind())
    }
}

impl Eq for WordId<'_> {}

impl Hash for WordId<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::from_ref(self.0.kind()).hash(state);
    }
}

struct PredicateId<'a>(&'a SymBoolExpr);

impl Clone for PredicateId<'_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for PredicateId<'_> {}

impl PartialEq for PredicateId<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.kind(), other.0.kind())
    }
}

impl Eq for PredicateId<'_> {}

impl Hash for PredicateId<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::from_ref(self.0.kind()).hash(state);
    }
}

impl<'a> QueryDagView for FoundryQuery<'a> {
    type WordId = WordId<'a>;
    type PredicateId = PredicateId<'a>;
    type Variable = Symbol;

    fn request(&self) -> QueryRequest {
        self.request
    }

    fn assertion_count(&self) -> usize {
        self.constraints.len()
    }

    fn assertion(&self, index: usize) -> Self::PredicateId {
        PredicateId(&self.constraints[index])
    }

    fn word(&self, id: Self::WordId) -> WordNode<Self::WordId, Self::PredicateId, Self::Variable> {
        let expression: &'a SymExpr = id.0;
        match expression.kind() {
            SymExprKind::Const(value) => WordNode::Constant(*value),
            SymExprKind::Var(variable) => WordNode::Variable(*variable),
            SymExprKind::GasLeft(variable) => {
                WordNode::Opaque { variable: *variable, kind: OpaqueKind::GasLeft }
            }
            SymExprKind::Keccak { name, .. } => {
                WordNode::Opaque { variable: *name, kind: OpaqueKind::Keccak }
            }
            SymExprKind::Hash { name, .. } => {
                WordNode::Opaque { variable: *name, kind: OpaqueKind::Hash }
            }
            SymExprKind::Not(value) => WordNode::Not(WordId(value)),
            SymExprKind::BinOp(operator, left, right) => WordNode::Binary {
                operator: binary_operator(*operator),
                left: WordId(left),
                right: WordId(right),
            },
            SymExprKind::TernOp(operator, left, right, modulus) => WordNode::Ternary {
                operator: ternary_operator(*operator),
                left: WordId(left),
                right: WordId(right),
                modulus: WordId(modulus),
            },
            SymExprKind::Ite(condition, then_value, else_value) => WordNode::Ite {
                condition: PredicateId(condition),
                then_value: WordId(then_value),
                else_value: WordId(else_value),
            },
        }
    }

    fn predicate(&self, id: Self::PredicateId) -> PredicateNode<Self::WordId, Self::PredicateId> {
        let expression: &'a SymBoolExpr = id.0;
        match expression.kind() {
            SymBoolExprKind::Const(value) => PredicateNode::Constant(*value),
            SymBoolExprKind::Not(value) => PredicateNode::Not(PredicateId(value)),
            SymBoolExprKind::And(values) => PredicateNode::Conjunction { arity: values.len() },
            SymBoolExprKind::Cmp(operator, left, right) => PredicateNode::Compare {
                operator: comparison_operator(*operator),
                left: WordId(left),
                right: WordId(right),
            },
        }
    }

    fn conjunction_child(&self, conjunction: Self::PredicateId, index: usize) -> Self::PredicateId {
        let SymBoolExprKind::And(values) = conjunction.0.kind() else {
            unreachable!("conjunction child requested for a non-conjunction")
        };
        PredicateId(&values[index])
    }
}

const fn binary_operator(operator: SymBinOp) -> BinaryOp {
    match operator {
        SymBinOp::Add => BinaryOp::Add,
        SymBinOp::Sub => BinaryOp::Sub,
        SymBinOp::Mul => BinaryOp::Mul,
        SymBinOp::UDiv => BinaryOp::UDiv,
        SymBinOp::URem => BinaryOp::URem,
        SymBinOp::SDiv => BinaryOp::SDiv,
        SymBinOp::SRem => BinaryOp::SRem,
        SymBinOp::And => BinaryOp::And,
        SymBinOp::Or => BinaryOp::Or,
        SymBinOp::Xor => BinaryOp::Xor,
        SymBinOp::Shl => BinaryOp::Shl,
        SymBinOp::Shr => BinaryOp::Shr,
        SymBinOp::Sar => BinaryOp::Sar,
    }
}

const fn ternary_operator(operator: SymTernOp) -> TernaryOp {
    match operator {
        SymTernOp::AddMod => TernaryOp::AddMod,
        SymTernOp::MulMod => TernaryOp::MulMod,
    }
}

const fn comparison_operator(operator: SymCmpOp) -> ComparisonOp {
    match operator {
        SymCmpOp::Eq => ComparisonOp::Eq,
        SymCmpOp::Ult => ComparisonOp::Ult,
        SymCmpOp::Ugt => ComparisonOp::Ugt,
        SymCmpOp::Ule => ComparisonOp::Ule,
        SymCmpOp::Uge => ComparisonOp::Uge,
        SymCmpOp::Slt => ComparisonOp::Slt,
        SymCmpOp::Sgt => ComparisonOp::Sgt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsupported_dynamic_product_constraints(cx: &mut SymCx) -> [SymBoolExpr; 3] {
        let x = SymExpr::var(cx, "x");
        let y = SymExpr::var(cx, "y");
        let one = SymExpr::one(cx);
        let fifteen = SymExpr::constant(cx, U256::from(15));
        let x_is_nontrivial = SymBoolExpr::cmp(cx, SymCmpOp::Ugt, x.clone(), one.clone());
        let y_is_nontrivial = SymBoolExpr::cmp(cx, SymCmpOp::Ugt, y.clone(), one);
        let product = SymExpr::binop(cx, SymBinOp::Mul, x, y);
        let product_eq_fifteen = SymBoolExpr::eq(cx, product, fifteen);
        [x_is_nontrivial, y_is_nontrivial, product_eq_fifteen]
    }

    fn angstrom_full_mul_constraints(cx: &mut SymCx) -> ([SymBoolExpr; 2], Symbol, Symbol) {
        let x = SymExpr::var(cx, "x");
        let x_symbol = x.kind().get_var().unwrap();
        let y = SymExpr::var(cx, "y");
        let y_symbol = y.kind().get_var().unwrap();
        let zero = SymExpr::zero(cx);
        let one = SymExpr::one(cx);
        let product = SymExpr::binop(cx, SymBinOp::Mul, x.clone(), y.clone());
        let quotient = SymExpr::binop(cx, SymBinOp::UDiv, product.clone(), x.clone());
        let x_is_zero = SymBoolExpr::eq(cx, x.clone(), zero.clone());
        let quotient_or_zero = SymExpr::ite(cx, x_is_zero.clone(), zero.clone(), quotient);
        let quotient_matches = SymBoolExpr::eq(cx, quotient_or_zero, y.clone());
        let quotient_matches_word = SymExpr::ite(cx, quotient_matches, one.clone(), zero.clone());
        let x_is_zero_word = SymExpr::ite(cx, x_is_zero, one.clone(), zero.clone());
        let no_overflow_or_zero =
            SymExpr::binop(cx, SymBinOp::Or, quotient_matches_word, x_is_zero_word);
        let overflow = SymBoolExpr::eq(cx, no_overflow_or_zero, zero);

        let modulus = SymExpr::constant(cx, U256::MAX);
        let wide_product = SymExpr::ternop(cx, SymTernOp::MulMod, x, y, modulus);
        let wide_below_wrapped =
            SymBoolExpr::cmp(cx, SymCmpOp::Ult, wide_product.clone(), product.clone());
        let rounded_product = SymExpr::binop(cx, SymBinOp::Add, product.clone(), one);
        let rounded_product = SymExpr::ite(cx, wide_below_wrapped, rounded_product, product);
        let overflow_delta = SymExpr::binop(cx, SymBinOp::Sub, wide_product, rounded_product);
        let two_to_128 = SymExpr::constant(cx, U256::ONE << 128);
        let bounded_delta = SymBoolExpr::cmp(cx, SymCmpOp::Ult, overflow_delta, two_to_128);

        ([overflow, bounded_delta], x_symbol, y_symbol)
    }

    #[test]
    fn unsupported_dynamic_product_fixture_has_a_concrete_model() {
        let mut cx = SymCx::new();
        let constraints = unsupported_dynamic_product_constraints(&mut cx);
        let x = cx.intern("x");
        let y = cx.intern("y");
        let model: SymbolicModel = [(x, U256::from(3)), (y, U256::from(5))].into_iter().collect();

        assert!(model_satisfies_constraints(&model, &constraints));
    }

    #[test]
    fn canonical_ids_share_cloned_hashcons_handles() {
        let mut cx = SymCx::new();
        let word = SymExpr::var(&mut cx, "word");
        let cloned_word = word.clone();
        assert!(WordId(&word) == WordId(&cloned_word));

        let predicate = SymBoolExpr::eq_word_const(&mut cx, &word, U256::from(7));
        let cloned_predicate = predicate.clone();
        assert!(PredicateId(&predicate) == PredicateId(&cloned_predicate));
    }

    #[test]
    fn adapter_maps_every_binary_ternary_and_comparison_operator() {
        let mut cx = SymCx::new();
        let left = SymExpr::var(&mut cx, "left");
        let right = SymExpr::var(&mut cx, "right");
        let modulus = SymExpr::var(&mut cx, "modulus");
        let query = FoundryQuery::new(&[], QueryRequest::Check);

        for (source, expected) in [
            (SymBinOp::Add, BinaryOp::Add),
            (SymBinOp::Sub, BinaryOp::Sub),
            (SymBinOp::Mul, BinaryOp::Mul),
            (SymBinOp::UDiv, BinaryOp::UDiv),
            (SymBinOp::URem, BinaryOp::URem),
            (SymBinOp::SDiv, BinaryOp::SDiv),
            (SymBinOp::SRem, BinaryOp::SRem),
            (SymBinOp::And, BinaryOp::And),
            (SymBinOp::Or, BinaryOp::Or),
            (SymBinOp::Xor, BinaryOp::Xor),
            (SymBinOp::Shl, BinaryOp::Shl),
            (SymBinOp::Shr, BinaryOp::Shr),
            (SymBinOp::Sar, BinaryOp::Sar),
        ] {
            let expression = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(source, left.clone(), right.clone()),
            );
            assert!(matches!(
                query.word(WordId(&expression)),
                WordNode::Binary { operator, .. } if operator == expected
            ));
        }

        for (source, expected) in
            [(SymTernOp::AddMod, TernaryOp::AddMod), (SymTernOp::MulMod, TernaryOp::MulMod)]
        {
            let expression = SymExpr::from_kind(
                &mut cx,
                SymExprKind::TernOp(source, left.clone(), right.clone(), modulus.clone()),
            );
            assert!(matches!(
                query.word(WordId(&expression)),
                WordNode::Ternary { operator, .. } if operator == expected
            ));
        }

        for (source, expected) in [
            (SymCmpOp::Eq, ComparisonOp::Eq),
            (SymCmpOp::Ult, ComparisonOp::Ult),
            (SymCmpOp::Ugt, ComparisonOp::Ugt),
            (SymCmpOp::Ule, ComparisonOp::Ule),
            (SymCmpOp::Uge, ComparisonOp::Uge),
            (SymCmpOp::Slt, ComparisonOp::Slt),
            (SymCmpOp::Sgt, ComparisonOp::Sgt),
        ] {
            let predicate = SymBoolExpr::from_kind(
                &mut cx,
                SymBoolExprKind::Cmp(source, left.clone(), right.clone()),
            );
            assert!(matches!(
                query.predicate(PredicateId(&predicate)),
                PredicateNode::Compare { operator, .. } if operator == expected
            ));
        }
    }

    #[test]
    fn adapter_maps_all_structural_and_opaque_nodes() {
        let mut cx = SymCx::new();
        let variable = SymExpr::var(&mut cx, "variable");
        let constant = SymExpr::constant(&mut cx, U256::from(7));
        let gas_left = SymExpr::gas_left(&mut cx, 0);
        let hash_name = cx.intern("hash");
        let hash = SymExpr::hash_symbol(&mut cx, hash_name, "sha256", vec![variable.clone()]);
        let keccak_name = cx.intern("keccak");
        let len = SymExpr::one(&mut cx);
        let keccak = SymExpr::keccak_symbol(&mut cx, keccak_name, len, vec![variable.clone()]);
        let not = SymExpr::from_kind(&mut cx, SymExprKind::Not(variable.clone()));
        let condition = SymBoolExpr::eq_word_const(&mut cx, &variable, U256::ZERO);
        let ite = SymExpr::from_kind(
            &mut cx,
            SymExprKind::Ite(condition.clone(), variable.clone(), constant.clone()),
        );
        let bool_true = SymBoolExpr::constant(&mut cx, true);
        let conjunction = SymBoolExpr::raw_and(&mut cx, vec![condition.clone(), bool_true]);
        let query = FoundryQuery::new(&[], QueryRequest::Check);

        assert!(
            matches!(query.word(WordId(&constant)), WordNode::Constant(value) if value == U256::from(7))
        );
        assert!(matches!(query.word(WordId(&variable)), WordNode::Variable(_)));
        assert!(matches!(
            query.word(WordId(&gas_left)),
            WordNode::Opaque { kind: OpaqueKind::GasLeft, .. }
        ));
        assert!(matches!(
            query.word(WordId(&hash)),
            WordNode::Opaque { kind: OpaqueKind::Hash, .. }
        ));
        assert!(matches!(
            query.word(WordId(&keccak)),
            WordNode::Opaque { kind: OpaqueKind::Keccak, .. }
        ));
        assert!(matches!(query.word(WordId(&not)), WordNode::Not(_)));
        assert!(matches!(query.word(WordId(&ite)), WordNode::Ite { .. }));
        assert!(matches!(query.predicate(PredicateId(&condition)), PredicateNode::Compare { .. }));
        assert!(matches!(
            query.predicate(PredicateId(&conjunction)),
            PredicateNode::Conjunction { arity: 2 }
        ));
        assert!(PredicateId(&condition) == query.conjunction_child(PredicateId(&conjunction), 0));
    }

    #[test]
    fn adapter_evaluator_matches_foundry_on_evm_edge_semantics() {
        fn assert_binary(operator: SymBinOp, left: U256, right: U256, expected: U256) {
            let mut cx = SymCx::new();
            let left_word = SymExpr::var(&mut cx, "left");
            let right_word = SymExpr::var(&mut cx, "right");
            let left_symbol = left_word.kind().get_var().unwrap();
            let right_symbol = right_word.kind().get_var().unwrap();
            let expression =
                SymExpr::from_kind(&mut cx, SymExprKind::BinOp(operator, left_word, right_word));
            let expected_word = SymExpr::constant(&mut cx, expected);
            let assertion = SymBoolExpr::from_kind(
                &mut cx,
                SymBoolExprKind::Cmp(SymCmpOp::Eq, expression, expected_word),
            );
            let assertions = [assertion];
            let query = FoundryQuery::new(&assertions, QueryRequest::Check);
            let mut model = SymbolicModel::default();
            model.insert(left_symbol, left);
            model.insert(right_symbol, right);

            assert!(foundry_solver::evaluate_query(&query, |symbol| model.value(symbol)).unwrap());
            assert!(model_satisfies_constraints(&model, &assertions));
        }

        let sign_bit = U256::ONE << 255;
        for (operator, left, right, expected) in [
            (SymBinOp::Add, U256::MAX, U256::ONE, U256::ZERO),
            (SymBinOp::Sub, U256::ZERO, U256::ONE, U256::MAX),
            (SymBinOp::Mul, U256::MAX, U256::from(2), U256::MAX - U256::ONE),
            (SymBinOp::UDiv, U256::from(7), U256::ZERO, U256::ZERO),
            (SymBinOp::URem, U256::from(7), U256::ZERO, U256::ZERO),
            (SymBinOp::SDiv, sign_bit, U256::MAX, sign_bit),
            (SymBinOp::SRem, sign_bit, U256::MAX, U256::ZERO),
            (SymBinOp::And, U256::from(0xaa), U256::from(0x0f), U256::from(0x0a)),
            (SymBinOp::Or, U256::from(0xa0), U256::from(0x0f), U256::from(0xaf)),
            (SymBinOp::Xor, U256::from(0xaa), U256::from(0x0f), U256::from(0xa5)),
            (SymBinOp::Shl, U256::ONE, U256::from(255), sign_bit),
            (SymBinOp::Shl, U256::ONE, U256::from(256), U256::ZERO),
            (SymBinOp::Shl, U256::ONE, U256::from(257), U256::ZERO),
            (SymBinOp::Shr, sign_bit, U256::from(255), U256::ONE),
            (SymBinOp::Shr, sign_bit, U256::from(256), U256::ZERO),
            (SymBinOp::Shr, sign_bit, U256::from(257), U256::ZERO),
            (SymBinOp::Sar, sign_bit, U256::from(255), U256::MAX),
            (SymBinOp::Sar, sign_bit, U256::from(256), U256::MAX),
            (SymBinOp::Sar, sign_bit, U256::from(257), U256::MAX),
            (SymBinOp::Sar, U256::ONE, U256::from(256), U256::ZERO),
        ] {
            assert_binary(operator, left, right, expected);
        }

        let mut cx = SymCx::new();
        let left = SymExpr::var(&mut cx, "left");
        let right = SymExpr::var(&mut cx, "right");
        let modulus = SymExpr::var(&mut cx, "modulus");
        let left_symbol = left.kind().get_var().unwrap();
        let right_symbol = right.kind().get_var().unwrap();
        let modulus_symbol = modulus.kind().get_var().unwrap();
        let mut model = SymbolicModel::default();
        model.insert(left_symbol, U256::MAX);
        model.insert(right_symbol, U256::MAX);
        model.insert(modulus_symbol, U256::MAX - U256::ONE);
        for (operator, expected) in [
            (SymTernOp::AddMod, U256::MAX.add_mod(U256::MAX, U256::MAX - U256::ONE)),
            (SymTernOp::MulMod, U256::MAX.mul_mod(U256::MAX, U256::MAX - U256::ONE)),
        ] {
            let expression = SymExpr::from_kind(
                &mut cx,
                SymExprKind::TernOp(operator, left.clone(), right.clone(), modulus.clone()),
            );
            let expected_word = SymExpr::constant(&mut cx, expected);
            let assertion = SymBoolExpr::from_kind(
                &mut cx,
                SymBoolExprKind::Cmp(SymCmpOp::Eq, expression, expected_word),
            );
            let assertions = [assertion];
            let query = FoundryQuery::new(&assertions, QueryRequest::Check);
            assert!(foundry_solver::evaluate_query(&query, |symbol| model.value(symbol)).unwrap());
            assert!(model_satisfies_constraints(&model, &assertions));
        }

        for (operator, left_value, right_value, expected) in [
            (SymCmpOp::Eq, U256::MAX, U256::MAX, true),
            (SymCmpOp::Ult, U256::ZERO, U256::MAX, true),
            (SymCmpOp::Ugt, U256::MAX, U256::ZERO, true),
            (SymCmpOp::Ule, U256::MAX, U256::MAX, true),
            (SymCmpOp::Uge, U256::ZERO, U256::ONE, false),
            (SymCmpOp::Slt, U256::MAX, U256::ZERO, true),
            (SymCmpOp::Sgt, U256::ZERO, U256::MAX, true),
        ] {
            model.insert(left_symbol, left_value);
            model.insert(right_symbol, right_value);
            let assertion = SymBoolExpr::from_kind(
                &mut cx,
                SymBoolExprKind::Cmp(operator, left.clone(), right.clone()),
            );
            let assertions = [assertion];
            let query = FoundryQuery::new(&assertions, QueryRequest::Check);
            assert_eq!(
                foundry_solver::evaluate_query(&query, |symbol| model.value(symbol)).unwrap(),
                expected
            );
            assert_eq!(model_satisfies_constraints(&model, &assertions), expected);
        }
    }

    #[test]
    fn native_sat_model_is_validated_and_converted_once() {
        let mut cx = SymCx::new();
        let word = SymExpr::var(&mut cx, "word");
        let symbol = word.kind().get_var().unwrap();
        let constraint = SymBoolExpr::eq_word_const(&mut cx, &word, U256::from(7));
        let constraints = [constraint];

        let NativeSolveResult::Sat(model) = solve_native(&constraints, &constraints, true) else {
            panic!("expected native SAT model")
        };
        assert_eq!(model.get(&symbol), Some(&U256::from(7)));
        assert!(model_satisfies_constraints(&model, &constraints));
    }

    #[test]
    fn native_exact_contradiction_is_unsat() {
        let mut cx = SymCx::new();
        let constraint = SymBoolExpr::constant(&mut cx, false);
        let constraints = [constraint];
        assert!(matches!(
            solve_native(&constraints, &constraints, false),
            NativeSolveResult::Unsat
        ));
    }

    #[test]
    fn native_capacity_contradiction_is_unsat() {
        let mut cx = SymCx::new();
        let a = SymExpr::var(&mut cx, "a");
        let b = SymExpr::var(&mut cx, "b");
        let cap = SymExpr::constant(&mut cx, U256::from(10));
        let residual =
            SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::Sub, cap.clone(), a.clone()));
        let sum =
            SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::Add, a.clone(), b.clone()));
        let constraints = [
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, a, cap.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, b, residual),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, sum, cap),
        ];
        assert!(matches!(
            solve_native(&constraints, &constraints, false),
            NativeSolveResult::Unsat
        ));
    }

    #[test]
    fn native_rejects_model_that_only_satisfies_normalized_constraints() {
        let mut cx = SymCx::new();
        let word = SymExpr::var(&mut cx, "word");
        let normalized = [SymBoolExpr::eq_word_const(&mut cx, &word, U256::from(1))];
        let original = [SymBoolExpr::eq_word_const(&mut cx, &word, U256::from(2))];
        assert!(matches!(solve_native(&normalized, &original, true), NativeSolveResult::Unknown));
    }

    #[test]
    fn native_rejects_opaque_keccak_candidate_against_concrete_evaluator() {
        let mut cx = SymCx::new();
        let input = SymExpr::var(&mut cx, "input");
        let hash = keccak_word(&mut cx, vec![input]);
        let constraint = SymBoolExpr::eq_word_const(&mut cx, &hash, U256::ZERO);
        let constraints = [constraint];
        assert!(matches!(
            solve_native(&constraints, &constraints, true),
            NativeSolveResult::Unknown
        ));
    }

    #[test]
    fn native_diversifies_inputs_until_concrete_keccak_validates() {
        let mut cx = SymCx::new();
        let input = SymExpr::var(&mut cx, "input");
        let input_symbol = input.kind().get_var().unwrap();
        let dummy_a = SymExpr::var(&mut cx, "dummy_a");
        let dummy_b = SymExpr::var(&mut cx, "dummy_b");
        let dummy_c = SymExpr::var(&mut cx, "dummy_c");
        let upper = SymExpr::one(&mut cx);
        let normalized = [
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, input.clone(), upper.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, dummy_a, upper.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, dummy_b, upper.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, dummy_c, upper),
        ];
        let hash = keccak_word(&mut cx, vec![input]);
        let expected = U256::from_be_bytes(alloy_primitives::keccak256([1_u8]).0);
        let mut original = normalized.to_vec();
        original.push(SymBoolExpr::eq_word_const(&mut cx, &hash, expected));

        let NativeSolveResult::Sat(model) = solve_native(&normalized, &original, true) else {
            panic!("expected an evaluator-confirmed concrete Keccak model")
        };
        assert_eq!(model.get(&input_symbol), Some(&U256::ONE));
        assert!(model_satisfies_constraints(&model, &original));
    }

    #[test]
    fn replay_model_materializes_a_missing_keccak_preimage_as_zero() {
        let mut cx = SymCx::new();
        let input = SymExpr::var(&mut cx, "storage_input");
        let input_symbol = input.kind().get_var().unwrap();
        let input_bytes = input.into_byte_exprs(&mut cx);
        let hash = keccak_word(&mut cx, input_bytes);
        let expected = U256::from_be_bytes(alloy_primitives::keccak256([0_u8; 32]).0);
        let constraint = SymBoolExpr::eq_word_const(&mut cx, &hash, expected);
        let constraints = [constraint];
        let replayable_storage = [input_symbol].into_iter().collect();
        let mut solver = SmtLibSubprocessSolver::from_config(&SymbolicConfig::default());

        let model = solver
            .model_with_replayable_storage(&mut cx, &constraints, &replayable_storage)
            .unwrap();

        assert_eq!(model.get(&input_symbol), Some(&U256::ZERO));
        assert!(model_satisfies_constraints(&model, &constraints));
    }

    #[test]
    fn native_preserves_gasleft_as_an_smt_emission_error() {
        let mut cx = SymCx::new();
        let gas_left = SymExpr::gas_left(&mut cx, 0);
        let constraint = SymBoolExpr::eq_word_const(&mut cx, &gas_left, U256::ZERO);
        assert!(matches!(
            solve_native(
                std::slice::from_ref(&constraint),
                std::slice::from_ref(&constraint),
                false
            ),
            NativeSolveResult::Unknown
        ));

        let contradiction = SymBoolExpr::constant(&mut cx, false);
        assert!(matches!(
            solve_native(&[constraint.clone(), contradiction.clone()], &[], false),
            NativeSolveResult::Unknown
        ));
        assert!(matches!(
            solve_native(&[contradiction, constraint], &[], false),
            NativeSolveResult::Unknown
        ));
    }

    #[test]
    fn adapter_does_not_pre_traverse_a_shared_dag() {
        let mut cx = SymCx::new();
        let mut shared = SymExpr::var(&mut cx, "input");
        for _ in 0..64 {
            shared = SymExpr::binop(&mut cx, SymBinOp::Add, shared.clone(), shared);
        }
        let deep = SymBoolExpr::eq_word_const(&mut cx, &shared, U256::ZERO);
        let contradiction = SymBoolExpr::constant(&mut cx, false);
        let constraints = [deep, contradiction];
        let query = FoundryQuery::new(&constraints, QueryRequest::Check);
        assert_eq!(query.variable_capacity_hint(), 0);
        assert!(matches!(
            solve_native(&constraints, &constraints, false),
            NativeSolveResult::Unsat
        ));
    }

    #[test]
    fn subprocess_solver_uses_native_sat_model_and_cache() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let z = SymExpr::var(&mut cx, "z");
        let zero = SymExpr::zero(&mut cx);
        let three = SymExpr::constant(&mut cx, U256::from(3));
        let xy = SymExpr::binop(&mut cx, SymBinOp::Add, x.clone(), y.clone());
        let sum = SymExpr::binop(&mut cx, SymBinOp::Add, xy, z.clone());
        let constraints = [
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x, zero.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, y, zero.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, z, zero),
            SymBoolExpr::eq(&mut cx, sum, three),
        ];
        let mut solver = SmtLibSubprocessSolver::from_config(&SymbolicConfig::default());

        assert!(solver.is_sat(&mut cx, &constraints).unwrap());
        let model = solver.model(&mut cx, &constraints).unwrap();
        assert!(model_satisfies_constraints(&model, &constraints));

        let stats = solver.stats();
        assert_eq!(stats.native_queries, 1);
        assert_eq!(stats.native_sat_queries, 1);
        assert_eq!(stats.native_unsat_queries, 0);
        assert_eq!(stats.native_unknown_queries, 0);
        assert_eq!(stats.smt_queries, 0);
        assert_eq!(stats.model_cache_hits, 1);
        assert_eq!(
            stats.native_queries,
            stats.native_sat_queries + stats.native_unsat_queries + stats.native_unknown_queries
        );
        assert!(stats.native_max_query_time_ns <= stats.native_solver_time_ns);
    }

    #[test]
    fn native_query_records_exact_unsat() {
        let mut cx = SymCx::new();
        let a = SymExpr::var(&mut cx, "a");
        let b = SymExpr::var(&mut cx, "b");
        let sum = SymExpr::binop(&mut cx, SymBinOp::Add, a.clone(), b.clone());
        let constraints = [
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, sum.clone(), a),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, sum, b),
        ];
        let mut solver = SmtLibSubprocessSolver::from_config(&SymbolicConfig::default());

        assert!(matches!(
            solver.query_native(&constraints, &constraints, false),
            NativeSolveResult::Unsat
        ));

        let stats = solver.stats();
        assert_eq!(stats.native_queries, 1);
        assert_eq!(stats.native_sat_queries, 0);
        assert_eq!(stats.native_unsat_queries, 1);
        assert_eq!(stats.native_unknown_queries, 0);
        assert_eq!(stats.smt_queries, 0);
        assert!(stats.native_max_query_time_ns <= stats.native_solver_time_ns);
    }

    #[test]
    fn native_only_query_does_not_require_external_solver() {
        let missing = SolverCommand::new(
            vec!["foundry-symbolic-definitely-missing-solver".to_string()],
            false,
        )
        .unwrap();
        let mut solver = SmtLibSubprocessSolver::new(Ok(vec![missing]), None, 2, false);
        assert!(solver.check_available().is_err());
        solver.enable_native_for_test();
        assert!(solver.check_available().is_ok());

        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let z = SymExpr::var(&mut cx, "z");
        let zero = SymExpr::zero(&mut cx);
        let three = SymExpr::constant(&mut cx, U256::from(3));
        let xy = SymExpr::binop(&mut cx, SymBinOp::Add, x.clone(), y.clone());
        let sum = SymExpr::binop(&mut cx, SymBinOp::Add, xy, z.clone());
        let constraints = [
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x, zero.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, y, zero.clone()),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, z, zero),
            SymBoolExpr::eq(&mut cx, sum, three),
        ];
        assert!(solver.is_sat(&mut cx, &constraints).unwrap());

        let stats = solver.stats();
        assert_eq!(stats.native_sat_queries, 1);
        assert_eq!(stats.native_unknown_queries, 0);
        assert_eq!(stats.smt_queries, 0);
    }

    #[test]
    fn production_constructor_can_build_native_and_same_binary_control_modes() {
        let config = SymbolicConfig {
            solver_command: Some("foundry-symbolic-definitely-missing-solver".to_string()),
            ..SymbolicConfig::default()
        };
        let native = SmtLibSubprocessSolver::from_config_with_routing(
            &config,
            SolverRouting::NativeWithFallback,
        );
        let control =
            SmtLibSubprocessSolver::from_config_with_routing(&config, SolverRouting::External);
        let probe_free_control = SmtLibSubprocessSolver::from_config_with_routing(
            &config,
            SolverRouting::ProbeFreeZ3Control,
        );

        assert!(native.check_available().is_ok());
        assert!(control.check_available().is_err());
        assert!(probe_free_control.check_available().is_ok());
    }

    #[test]
    fn probe_free_z3_control_requires_the_z3_control_guard() {
        assert_eq!(
            SolverRouting::from_internal_controls(false, true),
            SolverRouting::NativeWithFallback
        );
        assert_eq!(SolverRouting::from_internal_controls(true, false), SolverRouting::External);
        assert_eq!(
            SolverRouting::from_internal_controls(true, true),
            SolverRouting::ProbeFreeZ3Control
        );
    }

    #[test]
    fn native_enabled_still_rejects_malformed_solver_configuration() {
        let config = SymbolicConfig {
            solver_command: Some("\"unterminated".to_string()),
            ..SymbolicConfig::default()
        };
        let solver = SmtLibSubprocessSolver::from_config_with_routing(
            &config,
            SolverRouting::NativeWithFallback,
        );

        assert!(matches!(solver.check_available(), Err(SymbolicError::Solver(_))));
    }

    #[test]
    fn probe_free_z3_control_still_rejects_malformed_solver_configuration() {
        let config = SymbolicConfig {
            solver_command: Some("\"unterminated".to_string()),
            ..SymbolicConfig::default()
        };
        let solver = SmtLibSubprocessSolver::from_config_with_routing(
            &config,
            SolverRouting::ProbeFreeZ3Control,
        );

        assert!(matches!(solver.check_available(), Err(SymbolicError::Solver(_))));
    }

    #[test]
    fn probe_free_z3_control_still_pays_solver_startup_and_query_cost() {
        let config = SymbolicConfig {
            solver_command: Some("foundry-symbolic-definitely-missing-solver".to_string()),
            ..SymbolicConfig::default()
        };
        let mut solver = SmtLibSubprocessSolver::from_config_with_routing(
            &config,
            SolverRouting::ProbeFreeZ3Control,
        );
        assert!(solver.check_available().is_ok());

        let mut cx = SymCx::new();
        let constraints = unsupported_dynamic_product_constraints(&mut cx);

        assert!(matches!(solver.is_sat(&mut cx, &constraints), Err(SymbolicError::Solver(_))));
        let stats = solver.stats();
        assert_eq!(stats.native_queries, 0);
        assert_eq!(stats.smt_queries, 1);
    }

    #[test]
    fn native_unknown_still_requires_external_solver() {
        let missing = SolverCommand::new(
            vec!["foundry-symbolic-definitely-missing-solver".to_string()],
            false,
        )
        .unwrap();
        let mut solver = SmtLibSubprocessSolver::new(Ok(vec![missing]), None, 2, false);
        solver.enable_native_for_test();

        let mut cx = SymCx::new();
        let constraints = unsupported_dynamic_product_constraints(&mut cx);

        assert!(matches!(solver.is_sat(&mut cx, &constraints), Err(SymbolicError::Solver(_))));
        let stats = solver.stats();
        assert_eq!(stats.native_unknown_queries, 1);
        assert_eq!(stats.smt_queries, 1);
    }

    #[test]
    fn native_unknown_model_still_requires_external_solver() {
        let missing = SolverCommand::new(
            vec!["foundry-symbolic-definitely-missing-solver".to_string()],
            false,
        )
        .unwrap();
        let mut solver = SmtLibSubprocessSolver::new(Ok(vec![missing]), None, 2, false);
        solver.enable_native_for_test();

        let mut cx = SymCx::new();
        let constraints = unsupported_dynamic_product_constraints(&mut cx);

        assert!(matches!(solver.model(&mut cx, &constraints), Err(SymbolicError::Solver(_))));
        let stats = solver.stats();
        assert_eq!(stats.native_unknown_queries, 1);
        assert_eq!(stats.smt_queries, 1);
    }

    #[test]
    fn native_only_unknown_is_explicit_without_subprocess_fallback() {
        let config = SymbolicConfig { solver: "native".to_string(), ..Default::default() };
        let mut solver = SmtLibSubprocessSolver::from_config(&config);
        assert!(solver.check_available().is_ok());
        assert!(solver.commands().unwrap().is_empty());

        let mut cx = SymCx::new();
        let constraints = unsupported_dynamic_product_constraints(&mut cx);

        assert!(matches!(solver.is_sat(&mut cx, &constraints), Err(SymbolicError::SolverUnknown)));
        let stats = solver.stats();
        assert_eq!(stats.native_unknown_queries, 1);
        assert_eq!(stats.smt_queries, 0);

        let mut model_solver = SmtLibSubprocessSolver::from_config(&config);
        assert!(matches!(
            model_solver.model(&mut cx, &constraints),
            Err(SymbolicError::SolverUnknown)
        ));
        let stats = model_solver.stats();
        assert_eq!(stats.native_unknown_queries, 1);
        assert_eq!(stats.smt_queries, 0);
    }

    #[test]
    fn native_unsat_precedes_deferred_hard_arithmetic_fallback() {
        let missing = SolverCommand::new(
            vec!["foundry-symbolic-definitely-missing-solver".to_string()],
            false,
        )
        .unwrap();
        let mut solver = SmtLibSubprocessSolver::new(Ok(vec![missing]), None, 2, false);
        solver.enable_native_for_test();

        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let zero = SymExpr::zero(&mut cx);
        let x_is_zero = SymBoolExpr::eq(&mut cx, x.clone(), zero);
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x, y);
        let one = SymExpr::one(&mut cx);
        let product_eq_one = SymBoolExpr::eq(&mut cx, product, one);

        assert!(!solver.is_sat_branch(&mut cx, &[x_is_zero, product_eq_one]).unwrap());
        let stats = solver.stats();
        assert_eq!(stats.native_queries, 1);
        assert_eq!(stats.native_unsat_queries, 1);
        assert_eq!(stats.native_unknown_queries, 0);
        assert_eq!(stats.smt_queries, 0);
    }

    #[test]
    fn native_sat_precedes_hard_arithmetic_witness_for_angstrom_mulmod() {
        let config = SymbolicConfig { solver: "native".to_string(), ..Default::default() };
        let mut cx = SymCx::new();
        let (constraints, x, y) = angstrom_full_mul_constraints(&mut cx);
        let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
        assert!(constraints_prefer_hard_arith_fallback_first(&cx, &normalized));
        assert!(validated_hard_arith_fallback_model(&cx, &normalized, &constraints).is_some());

        let mut sat_solver = SmtLibSubprocessSolver::from_config(&config);
        assert!(sat_solver.is_sat(&mut cx, &constraints).unwrap());
        let stats = sat_solver.stats();
        assert_eq!(stats.native_queries, 1);
        assert_eq!(stats.native_sat_queries, 1);
        assert_eq!(stats.native_unknown_queries, 0);
        assert_eq!(stats.smt_queries, 0);
        assert_eq!(sat_solver.heuristic_witnesses(), 0);

        let mut model_solver = SmtLibSubprocessSolver::from_config(&config);
        let model = model_solver.model(&mut cx, &constraints).unwrap();
        assert_eq!(model.get(&x), Some(&(U256::ONE << 128)));
        assert_eq!(model.get(&y), Some(&(U256::ONE << 128)));
        assert!(model_satisfies_constraints(&model, &constraints));
        let stats = model_solver.stats();
        assert_eq!(stats.native_queries, 1);
        assert_eq!(stats.native_sat_queries, 1);
        assert_eq!(stats.native_unknown_queries, 0);
        assert_eq!(stats.smt_queries, 0);
        assert_eq!(model_solver.heuristic_witnesses(), 0);
    }

    #[test]
    fn native_unknown_hard_arithmetic_branch_does_not_require_unused_fallback() {
        let missing = SolverCommand::new(
            vec!["foundry-symbolic-definitely-missing-solver".to_string()],
            false,
        )
        .unwrap();
        let mut solver = SmtLibSubprocessSolver::new(Ok(vec![missing]), None, 2, false);
        solver.enable_native_for_test();

        let mut cx = SymCx::new();
        let constraints = unsupported_dynamic_product_constraints(&mut cx);

        assert!(matches!(
            solver.is_sat_branch(&mut cx, &constraints),
            Err(SymbolicError::SolverUnknown)
        ));
        let stats = solver.stats();
        assert_eq!(stats.native_queries, 1);
        assert_eq!(stats.native_unknown_queries, 1);
        assert_eq!(stats.smt_queries, 0);
    }
}
