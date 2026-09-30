//! Tests for constraint and expression normalization.

use super::*;

#[test]
fn cached_normalization_keeps_contextual_rewrites_per_query() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let mask = SymExpr::constant(&mut cx, (U256::from(1) << 160) - U256::from(1));
    let masked = SymExpr::binop(&mut cx, SymBinOp::And, value.clone(), mask);
    let identity = SymBoolExpr::eq(&mut cx, masked, value.clone());
    let upper = SymExpr::constant(&mut cx, U256::from(1) << 160);
    let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value, upper);
    let mut cache = HashMap::default();

    let normalized = normalize_constraints_for_solver_cached(
        &mut cx,
        &[identity.clone(), bounded.clone()],
        &mut cache,
    );
    assert_eq!(normalized, vec![bounded]);
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.get(&identity), Some(&identity));

    let normalized = normalize_constraints_for_solver_cached(
        &mut cx,
        std::slice::from_ref(&identity),
        &mut cache,
    );
    assert_eq!(normalized, vec![identity]);
    assert_eq!(cache.len(), 2);
}

#[test]
fn cached_normalization_keeps_udiv_rewrites_contextual() {
    let mut cx = SymCx::new();
    let numerator = SymExpr::var(&mut cx, "numerator");
    let threshold = SymExpr::var(&mut cx, "threshold");
    let scale = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u128));
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, scale);
    let comparison = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, quotient, threshold.clone());
    let uint128_max = SymExpr::constant(&mut cx, U256::from(u128::MAX));
    let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, threshold, uint128_max);
    let mut cache = HashMap::default();

    let normalized = normalize_constraints_for_solver_cached(
        &mut cx,
        &[comparison.clone(), bounded],
        &mut cache,
    );
    assert!(normalized.iter().all(|constraint| !constraint.contains_udiv()));
    assert_eq!(cache.get(&comparison), Some(&comparison));

    let normalized = normalize_constraints_for_solver_cached(
        &mut cx,
        std::slice::from_ref(&comparison),
        &mut cache,
    );
    assert_eq!(normalized, vec![comparison]);
}

#[test]
fn symbolic_divisor_rounding_preserves_its_own_premise() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let divisor = SymExpr::var(&mut cx, "divisor");
    let offset = SymExpr::constant(&mut cx, U256::from(36));
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Add, value.clone(), offset);
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, divisor.clone());
    let rounded = SymExpr::binop(&mut cx, SymBinOp::Mul, quotient, divisor.clone());
    let divisor_value = SymExpr::constant(&mut cx, U256::from(37));
    let fixed_divisor = SymBoolExpr::eq(&mut cx, divisor.clone(), divisor_value);
    let rounded_bound =
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &rounded, U256::from(100));
    let premise = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, value.clone(), rounded);
    let constraints = vec![fixed_divisor, rounded_bound, premise];
    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, U256::MAX));
    assert!(divisor.assign_model_value(&mut model, U256::from(37)));
    assert!(!constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));

    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    assert!(!normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));

    let mut cache = HashMap::default();
    let normalized = normalize_constraints_for_solver_cached(&mut cx, &constraints, &mut cache);
    assert!(!normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    let normalized =
        normalize_constraints_for_solver_cached(&mut cx, &constraints[1..], &mut cache);
    assert!(!normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn direct_contradiction_uses_members_of_derived_positive_conjunction() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let zero = SymExpr::zero(&mut cx);
    let x_is_zero = SymBoolExpr::eq(&mut cx, x, zero.clone());
    let y_is_zero = SymBoolExpr::eq(&mut cx, y, zero.clone());
    let x_word = SymExpr::bool_word(&mut cx, x_is_zero.clone());
    let y_word = SymExpr::bool_word(&mut cx, y_is_zero);
    let either_word = SymExpr::binop(&mut cx, SymBinOp::Or, x_word, y_word);
    let neither_is_zero = SymBoolExpr::eq(&mut cx, either_word, zero);

    let constraints = normalize_constraints_for_solver(&mut cx, &[neither_is_zero, x_is_zero]);
    assert!(constraints_are_directly_unsat(&mut cx, &constraints));
}

#[test]
fn direct_contradiction_does_not_expand_derived_negated_conjunction() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let zero = SymExpr::zero(&mut cx);
    let x_is_zero = SymBoolExpr::eq(&mut cx, x, zero.clone());
    let y_is_zero = SymBoolExpr::eq(&mut cx, y, zero.clone());
    let x_word = SymExpr::bool_word(&mut cx, x_is_zero.clone());
    let y_word = SymExpr::bool_word(&mut cx, y_is_zero);
    let both_word = SymExpr::binop(&mut cx, SymBinOp::And, x_word, y_word);
    let not_both = SymBoolExpr::eq(&mut cx, both_word, zero);

    let constraints = normalize_constraints_for_solver(&mut cx, &[not_both, x_is_zero]);
    assert!(!constraints_are_directly_unsat(&mut cx, &constraints));
}

#[test]
fn shared_arithmetic_dag_interval_analysis_is_memoized() {
    let mut cx = SymCx::new();
    let source = SymExpr::var(&mut cx, "source");
    let one = SymExpr::one(&mut cx);
    let mut shared = SymExpr::binop(&mut cx, SymBinOp::And, source, one);
    for _ in 0..64 {
        shared = SymExpr::binop(&mut cx, SymBinOp::Add, shared.clone(), shared);
    }

    assert!(ConstraintContext::default().mul_cannot_overflow_256(&shared, &shared));
}

#[test]
fn contextual_normalization_substitutes_direct_exact_values() {
    let mut cx = SymCx::new();
    let denominator = SymExpr::var(&mut cx, "denominator");
    let numerator = SymExpr::var(&mut cx, "numerator");
    let scale = U256::from(1_000_000_000_000_000_000u64);
    let scale_expr = SymExpr::constant(&mut cx, scale);
    let fixed = SymBoolExpr::eq(&mut cx, denominator.clone(), scale_expr);
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, denominator);
    let zero = SymExpr::zero(&mut cx);
    let quotient_nonzero = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, quotient, zero);

    let normalized = normalize_constraints_for_solver(&mut cx, &[fixed.clone(), quotient_nonzero]);

    assert!(normalized.contains(&fixed));
    assert!(normalized.iter().any(|constraint| {
        matches!(
            constraint.kind(),
            SymBoolExprKind::Cmp(SymCmpOp::Ule, left, _) if left.as_const() == Some(scale)
        )
    }));
}

#[test]
fn contextual_normalization_revisits_generated_overflow_comparison() {
    let mut cx = SymCx::new();
    let byte_mask = SymExpr::constant(&mut cx, U256::from(0xff));
    let left = SymExpr::var(&mut cx, "left");
    let left = SymExpr::binop(&mut cx, SymBinOp::And, left, byte_mask.clone());
    let denominator = SymExpr::var(&mut cx, "denominator");
    let denominator = SymExpr::binop(&mut cx, SymBinOp::And, denominator, byte_mask);
    let zero = SymExpr::zero(&mut cx);
    let denominator_nonzero =
        SymBoolExpr::eq(&mut cx, denominator.clone(), zero.clone()).not(&mut cx);
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Add, left, denominator.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, numerator, denominator);
    let quotient_zero = SymBoolExpr::eq(&mut cx, quotient, zero);

    let normalized =
        normalize_constraints_for_solver(&mut cx, &[denominator_nonzero, quotient_zero]);

    assert!(constraints_are_directly_unsat(&mut cx, &normalized));
}

#[test]
fn contextual_normalization_rewrites_bounded_mul_div_subexpressions() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let expected = SymExpr::var(&mut cx, "expected");
    let scale = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u64));
    let uint128_max = SymExpr::constant(&mut cx, U256::from(u128::MAX));
    let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value.clone(), uint128_max);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value, scale.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, scale);
    let one = SymExpr::one(&mut cx);
    let incremented = SymExpr::binop(&mut cx, SymBinOp::Add, quotient, one);
    let comparison = SymBoolExpr::eq(&mut cx, incremented, expected);

    let normalized = normalize_constraints_for_solver(&mut cx, &[bounded, comparison]);

    assert!(
        !normalized.iter().any(|constraint| constraint.visit_bool(|expr| {
            matches!(expr.kind(), SymExprKind::BinOp(SymBinOp::UDiv, _, _))
        }))
    );
}

#[test]
fn contextual_normalization_rewrites_exact_round_up_scale_conversion() {
    let mut cx = SymCx::new();
    let credits = SymExpr::var(&mut cx, "credits");
    let uint128_max = SymExpr::constant(&mut cx, U256::from(u128::MAX));
    let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, credits.clone(), uint128_max);
    let scale = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u64));
    let credits_per_token = SymExpr::constant(
        &mut cx,
        U256::from(1_000_000_000_000_000_000u64) * U256::from(1_000_000_000u64),
    );
    let product =
        SymExpr::binop(&mut cx, SymBinOp::Mul, credits.clone(), credits_per_token.clone());
    let rounded = SymExpr::binop(&mut cx, SymBinOp::Add, product, scale.clone());
    let one = SymExpr::one(&mut cx);
    let rounded = SymExpr::binop(&mut cx, SymBinOp::Sub, rounded, one);
    let converted = SymExpr::binop(&mut cx, SymBinOp::UDiv, rounded, scale.clone());
    let converted = SymExpr::binop(&mut cx, SymBinOp::Mul, converted, scale);
    let converted = SymExpr::binop(&mut cx, SymBinOp::UDiv, converted, credits_per_token);
    let round_trip = SymBoolExpr::eq(&mut cx, converted, credits);

    let normalized = normalize_constraints_for_solver(&mut cx, &[bounded.clone(), round_trip]);

    assert_eq!(normalized, vec![bounded]);
}

#[test]
fn subtraction_underflow_normalization_preserves_modular_boundaries() {
    let mut cx = SymCx::new();
    let base = SymExpr::var(&mut cx, "base");
    let delta = SymExpr::var(&mut cx, "delta");
    let difference = SymExpr::binop(&mut cx, SymBinOp::Sub, base.clone(), delta.clone());
    let values =
        [U256::ZERO, U256::ONE, U256::from(2), U256::ONE << 255, U256::MAX - U256::ONE, U256::MAX];
    for (op, left, right) in [
        (SymCmpOp::Ult, base.clone(), difference.clone()),
        (SymCmpOp::Ugt, difference.clone(), base.clone()),
        (SymCmpOp::Ule, difference.clone(), base.clone()),
        (SymCmpOp::Uge, base.clone(), difference),
    ] {
        let original = SymBoolExpr::cmp(&mut cx, op, left, right);
        let normalized = normalize_bool_for_solver(&mut cx, original.clone());
        assert_ne!(normalized, original);
        for a in values {
            for b in values {
                let mut model = SymbolicModel::default();
                assert!(base.assign_model_value(&mut model, a));
                assert!(delta.assign_model_value(&mut model, b));
                assert_eq!(
                    original.eval_model(&model).unwrap(),
                    normalized.eval_model(&model).unwrap()
                );
            }
        }
    }
}

#[test]
fn complemented_constant_comparisons_preserve_word_boundaries() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let complement = SymExpr::not(&mut cx, value.clone());
    let samples = [
        U256::ZERO,
        U256::ONE,
        U256::from(36),
        U256::from(37),
        (U256::ONE << 255) - U256::ONE,
        U256::ONE << 255,
        U256::MAX - U256::from(36),
        U256::MAX,
    ];
    for limit in samples {
        let constant = SymExpr::constant(&mut cx, limit);
        for op in [
            SymCmpOp::Eq,
            SymCmpOp::Ult,
            SymCmpOp::Ule,
            SymCmpOp::Ugt,
            SymCmpOp::Uge,
            SymCmpOp::Slt,
            SymCmpOp::Sgt,
        ] {
            for (left, right) in
                [(complement.clone(), constant.clone()), (constant.clone(), complement.clone())]
            {
                let comparison = SymBoolExpr::cmp(&mut cx, op, left, right);
                for original in [comparison.clone(), comparison.not(&mut cx)] {
                    let normalized = normalize_bool_for_solver(&mut cx, original.clone());
                    for input in samples {
                        let mut model = SymbolicModel::default();
                        assert!(value.assign_model_value(&mut model, input));
                        assert_eq!(
                            original.eval_model(&model).unwrap(),
                            normalized.eval_model(&model).unwrap(),
                            "op={op:?}, limit={limit}, input={input}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn scaled_zero_check_requires_non_wrapping_positive_scale() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "balance");
    let max_value = SymExpr::constant(&mut cx, U256::from(u128::MAX));
    let bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, value.clone(), max_value);
    let context = ConstraintContext::from_constraints(std::iter::once(&bound), 1);
    for scale_value in
        [U256::ZERO, U256::from(3), U256::from(1_000_000_000_000_000_000u64), U256::MAX]
    {
        let scale = SymExpr::constant(&mut cx, scale_value);
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), scale.clone());
        let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, product, scale);
        let recognized = context.bounded_zero_check_operand(&condition);
        if !scale_value.is_zero() && U256::from(u128::MAX).checked_mul(scale_value).is_some() {
            assert_eq!(recognized, Some(&value));
            for balance in [U256::ZERO, U256::ONE, U256::from(u128::MAX)] {
                let mut model = SymbolicModel::default();
                assert!(value.assign_model_value(&mut model, balance));
                assert_eq!(condition.eval_model(&model).unwrap(), balance.is_zero());
            }
        } else {
            assert!(recognized.is_none());
        }
        assert!(ConstraintContext::default().bounded_zero_check_operand(&condition).is_none());
    }
}

#[test]
fn checked_multiplication_accepts_scaled_zero_guard_with_independent_bounds() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "balance");
    let rate = SymExpr::var(&mut cx, "rate");
    let scale_value = U256::from(1_000_000_000_000_000_000u64);
    let scale = SymExpr::constant(&mut cx, scale_value);
    let max_rate = SymExpr::constant(&mut cx, scale_value * U256::from(1_000_000_000u64));
    let max_value = SymExpr::constant(&mut cx, U256::from(u128::MAX));
    let mut bounds = vec![
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, value.clone(), max_value),
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, scale.clone(), rate.clone()),
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, rate.clone(), max_rate),
    ];
    let scaled = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), scale.clone());
    let is_zero = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, scaled, scale);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), rate.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, value.clone());
    let zero = SymExpr::zero(&mut cx);
    let guarded = SymExpr::ite(&mut cx, is_zero.clone(), zero.clone(), quotient);
    let exact = SymBoolExpr::eq(&mut cx, guarded, rate.clone());
    let zero_word = SymExpr::bool_word(&mut cx, is_zero);
    let exact_word = SymExpr::bool_word(&mut cx, exact);
    let safe = SymExpr::binop(&mut cx, SymBinOp::Or, zero_word, exact_word);
    let overflow = SymBoolExpr::eq(&mut cx, safe, zero);
    bounds.push(overflow);
    let normalized = normalize_constraints_for_solver(&mut cx, &bounds);
    assert!(constraints_are_directly_unsat(&mut cx, &normalized));

    // Without the independent value bound, modular multiplication can overflow.
    bounds.remove(0);
    let normalized = normalize_constraints_for_solver(&mut cx, &bounds);
    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, (U256::ONE << 255) + U256::ONE));
    assert!(rate.assign_model_value(&mut model, scale_value + U256::ONE));
    assert!(bounds.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    assert!(normalized.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
}

#[test]
fn quotient_zero_check_requires_matching_unsigned_positive_divisor() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let other = SymExpr::var(&mut cx, "other");
    let scale = SymExpr::constant(&mut cx, U256::from(37));
    let zero = SymExpr::zero(&mut cx);
    let context = ConstraintContext::default();
    // Keep raw division nodes for zero and symbolic denominators so the matcher itself is tested,
    // independently of constructor simplification.
    for denominator in [scale.clone(), zero, other.clone()] {
        let quotient = SymExpr::from_kind(
            &mut cx,
            SymExprKind::BinOp(SymBinOp::UDiv, value.clone(), denominator.clone()),
        );
        for numerator in [value.clone(), other.clone()] {
            for limit in [scale.clone(), denominator.clone()] {
                for op in [SymCmpOp::Ult, SymCmpOp::Ule, SymCmpOp::Slt] {
                    let condition = SymBoolExpr::cmp(&mut cx, op, numerator.clone(), limit.clone());
                    assert_eq!(
                        context.zero_check_for_operand(&condition, &quotient),
                        denominator == scale
                            && numerator == value
                            && limit == scale
                            && op == SymCmpOp::Ult
                    );
                }
            }
        }
    }
}

#[test]
fn checked_multiplication_preserves_normalized_quotient_zero_guards() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let rate = SymExpr::var(&mut cx, "rate");
    let scale = SymExpr::constant(&mut cx, U256::from(37));
    let operand = SymExpr::binop(&mut cx, SymBinOp::UDiv, value.clone(), scale.clone());
    let zero = SymExpr::zero(&mut cx);
    let is_zero = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value.clone(), scale.clone());
    let wrong_zero = SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &value, U256::from(38));
    let direct_zero = SymBoolExpr::eq(&mut cx, operand.clone(), zero.clone());
    let mut cache = HashMap::default();
    for factor in [scale.clone(), rate.clone()] {
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, operand.clone(), factor.clone());
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, operand.clone());
        for outer in [&is_zero, &direct_zero, &wrong_zero] {
            for inner in [&is_zero, &direct_zero, &wrong_zero] {
                let guarded = SymExpr::ite(&mut cx, inner.clone(), zero.clone(), quotient.clone());
                let exact = SymBoolExpr::eq(&mut cx, guarded, factor.clone());
                let zero_word = SymExpr::bool_word(&mut cx, outer.clone());
                let exact_word = SymExpr::bool_word(&mut cx, exact.clone());
                let safe = SymExpr::binop(&mut cx, SymBinOp::Or, zero_word, exact_word);
                let failure = SymBoolExpr::eq(&mut cx, safe, zero.clone());
                let valid = outer != &wrong_zero && inner != &wrong_zero;
                if factor == scale && valid {
                    let normalized =
                        normalize_constraints_for_solver(&mut cx, std::slice::from_ref(&failure));
                    assert!(constraints_are_directly_unsat(&mut cx, &normalized));
                }
                // Retained guard inference and guard elimination share the same zero matching, but
                // still need independent support.
                let mut context = ConstraintContext::default();
                let guard = failure.clone().not(&mut cx);
                assert_eq!(context.record_non_wrapping_product(&mut cx, &guard), valid);
                // A third alternative must not establish product safety.
                let extra = SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Eq, &value, U256::MAX);
                let loose = SymBoolExpr::or(&mut cx, vec![outer.clone(), exact, extra]);
                assert!(!ConstraintContext::default().record_non_wrapping_product(&mut cx, &loose));
                for predicate in [guard, failure] {
                    let normalized = normalize_constraints_for_solver_cached(
                        &mut cx,
                        std::slice::from_ref(&predicate),
                        &mut cache,
                    );
                    for input in [
                        U256::ZERO,
                        U256::ONE,
                        U256::from(36),
                        U256::from(37),
                        U256::from(38),
                        U256::ONE << 255,
                        U256::MAX,
                    ] {
                        for multiplier in
                            [U256::ZERO, U256::ONE, U256::from(37), U256::from(38), U256::MAX]
                        {
                            let mut model = SymbolicModel::default();
                            assert!(value.assign_model_value(&mut model, input));
                            assert!(rate.assign_model_value(&mut model, multiplier));
                            assert_eq!(
                                predicate.eval_model(&model).unwrap(),
                                normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                                "valid={valid}, input={input}, multiplier={multiplier}"
                            );
                        }
                    }
                }
            }
        }
    }
}

fn rounded_conversion(cx: &mut SymCx, value: SymExpr, rate: SymExpr, scale: U256) -> SymExpr {
    let scale = SymExpr::constant(cx, scale);
    let product = SymExpr::binop(cx, SymBinOp::Mul, value, rate.clone());
    let sum = SymExpr::binop(cx, SymBinOp::Add, product, scale.clone());
    let one = SymExpr::one(cx);
    let numerator = SymExpr::binop(cx, SymBinOp::Sub, sum, one);
    let credits = SymExpr::binop(cx, SymBinOp::UDiv, numerator, scale.clone());
    let scaled = SymExpr::binop(cx, SymBinOp::Mul, credits, scale);
    SymExpr::binop(cx, SymBinOp::UDiv, scaled, rate)
}

#[test]
fn quotient_bounds_do_not_require_bounded_numerators() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let scale = SymExpr::constant(&mut cx, U256::from(1_000_000_000_000_000_000u64));
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, value.clone(), scale.clone());
    let context = ConstraintContext::default();
    let interval = context.interval(&quotient).expect("uint256 quotient range");
    assert_eq!(interval.min, U256::ZERO);
    assert_eq!(interval.max, U256::MAX / scale.as_const().unwrap());
    let scaled = SymExpr::binop(&mut cx, SymBinOp::Mul, quotient.clone(), scale.clone());
    let round_trip = SymExpr::binop(&mut cx, SymBinOp::UDiv, scaled, scale);
    let failure = SymBoolExpr::eq(&mut cx, round_trip, quotient).not(&mut cx);
    let normalized = normalize_constraints_for_solver(&mut cx, &[failure]);
    assert!(constraints_are_directly_unsat(&mut cx, &normalized));

    // A divisor that may be zero must not acquire a positive minimum from this fallback.
    let divisor = SymExpr::var(&mut cx, "divisor");
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, value, divisor);
    assert!(context.interval(&quotient).is_none());
}

#[test]
fn constant_mul_div_guard_becomes_exact_overflow_bound() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    for factor in
        [U256::from(10), U256::from(3), U256::from(1_000_000_000_000_000_000u64), U256::MAX]
    {
        let scale = SymExpr::constant(&mut cx, factor);
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), scale.clone());
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, scale);
        let guard = SymBoolExpr::eq(&mut cx, quotient, value.clone());
        let expected =
            SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX / factor);
        for (original, expected) in
            [(guard.clone(), expected.clone()), (guard.not(&mut cx), expected.not(&mut cx))]
        {
            let normalized = normalize_bool_for_solver(&mut cx, original.clone());
            assert_eq!(normalized, expected, "factor={factor}");
            for input in [
                U256::ZERO,
                U256::ONE,
                U256::from(2),
                U256::MAX / factor,
                U256::MAX / factor + U256::ONE,
                U256::ONE << 255,
                U256::MAX,
            ] {
                let mut model = SymbolicModel::default();
                assert!(value.assign_model_value(&mut model, input));
                assert_eq!(
                    original.eval_model(&model).unwrap(),
                    normalized.eval_model(&model).unwrap(),
                    "factor={factor}, input={input}",
                );
            }
        }
    }
}

#[test]
fn constant_mul_div_guard_rewrite_excludes_unsound_shapes() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let other = SymExpr::var(&mut cx, "other");
    let three = SymExpr::constant(&mut cx, U256::from(3));
    let ten_value = U256::from(10);
    let ten = SymExpr::constant(&mut cx, ten_value);
    let rewritten =
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX / ten_value);

    // (name, factor, division, divisor, expected side of the equality)
    let cases = [
        ("signed division", ten.clone(), SymBinOp::SDiv, ten.clone(), value.clone()),
        ("mismatched divisor", ten.clone(), SymBinOp::UDiv, three, value.clone()),
        ("mismatched expected value", ten.clone(), SymBinOp::UDiv, ten, other.clone()),
        ("symbolic factor", other.clone(), SymBinOp::UDiv, other.clone(), value.clone()),
    ];
    for (name, factor, division, divisor, expected) in cases {
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), factor);
        let quotient = SymExpr::binop(&mut cx, division, product, divisor);
        let guard = SymBoolExpr::eq(&mut cx, quotient, expected);
        for original in [guard.clone(), guard.not(&mut cx)] {
            let normalized = normalize_bool_for_solver(&mut cx, original.clone());
            assert_ne!(normalized, rewritten, "{name}");
            assert_ne!(normalized, rewritten.clone().not(&mut cx), "{name}");
            for value_input in [
                U256::ZERO,
                U256::ONE,
                U256::MAX / ten_value,
                U256::MAX / ten_value + U256::ONE,
                U256::ONE << 255,
                U256::MAX,
            ] {
                for other_input in [U256::ZERO, U256::ONE, U256::from(3), U256::MAX] {
                    let mut model = SymbolicModel::default();
                    assert!(value.assign_model_value(&mut model, value_input));
                    assert!(other.assign_model_value(&mut model, other_input));
                    assert_eq!(
                        original.eval_model(&model).unwrap(),
                        normalized.eval_model(&model).unwrap(),
                        "{name}: value={value_input}, other={other_input}",
                    );
                }
            }
        }
    }
}

#[test]
fn scaled_zero_branch_proves_full_width_round_trip() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "credits");
    let rate = SymExpr::var(&mut cx, "rate");
    let w = U256::from(1_000_000_000_000_000_000u64);
    let scale = SymExpr::constant(&mut cx, w);
    let scaled = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), scale.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, scaled.clone(), scale);
    let guard = SymBoolExpr::eq(&mut cx, quotient, value.clone());
    let converted = rounded_conversion(&mut cx, value.clone(), rate.clone(), w);
    let identity = SymBoolExpr::eq(&mut cx, converted, value.clone());
    let mut constraints = vec![
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &rate, w),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &scaled, w),
        identity.not(&mut cx),
    ];
    // Without the successful multiplication guard a wrapped zero is not a zero operand.
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, U256::ONE << 255));
    assert!(rate.assign_model_value(&mut model, w));
    assert!(constraints.iter().all(|c| c.eval_model(&model).unwrap()));
    assert!(normalized.iter().all(|c| c.eval_model(&model).unwrap()));
    constraints.push(guard);
    let mut context = ConstraintContext::new(&constraints);
    for constraint in &constraints {
        context.record_non_wrapping_product(&mut cx, constraint);
    }
    for constraint in &constraints {
        context.record_scaled_zero_fact(&mut cx, constraint);
    }
    assert_eq!(context.exact_value(&value), Some(U256::ZERO));
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    assert!(constraints_are_directly_unsat(&mut cx, &normalized), "{normalized:?}");
    for nonzero in [false, true] {
        let mut predicates = constraints.clone();
        if nonzero {
            predicates[1] = predicates[1].clone().not(&mut cx);
        }
        for negate_identity in [false, true] {
            if negate_identity {
                predicates[2] = predicates[2].clone().not(&mut cx);
            }
            let normalized = normalize_constraints_for_solver(&mut cx, &predicates);
            for x in [U256::ZERO, U256::ONE, U256::MAX / w, U256::ONE << 255, U256::MAX] {
                for p in [U256::ZERO, U256::ONE, w, w + U256::ONE, U256::MAX] {
                    let mut model = SymbolicModel::default();
                    assert!(value.assign_model_value(&mut model, x));
                    assert!(rate.assign_model_value(&mut model, p));
                    assert_eq!(
                        predicates.iter().all(|c| c.eval_model(&model).unwrap()),
                        normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                        "nonzero={nonzero}, value={x}, rate={p}"
                    );
                }
            }
        }
    }
}

#[test]
fn product_operand_bounds_preserve_full_width_models() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "credits");
    let factor = SymExpr::var(&mut cx, "rate");
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), factor.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, factor.clone());
    let exact = SymBoolExpr::eq(&mut cx, quotient, value.clone());
    let zero = SymExpr::zero(&mut cx);
    let is_zero = SymBoolExpr::eq(&mut cx, factor.clone(), zero);
    let guard = SymBoolExpr::or(&mut cx, vec![is_zero, exact]);
    let w = U256::from(1_000_000_000_000_000_000u64);
    for minimum in [U256::ZERO, U256::ONE, w] {
        let lower = SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &factor, minimum);
        let mut context = ConstraintContext::new(std::slice::from_ref(&lower));
        context.record_non_wrapping_product(&mut cx, &guard);
        assert_eq!(context.upper_bound(&value), (!minimum.is_zero()).then(|| U256::MAX / minimum));
        let bound = SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX / w);
        for predicate in [bound.clone(), bound.not(&mut cx)] {
            // Include both orders: a derived bound must never erase its own support.
            let constraints = vec![lower.clone(), guard.clone(), predicate];
            for constraints in [constraints.clone(), constraints.into_iter().rev().collect()] {
                let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
                for x in [
                    U256::ZERO,
                    U256::ONE,
                    U256::ONE << 128,
                    U256::MAX / w,
                    U256::MAX / w + U256::ONE,
                    U256::MAX,
                ] {
                    for y in [U256::ZERO, U256::ONE, w, w + U256::ONE, U256::MAX] {
                        let mut model = SymbolicModel::default();
                        assert!(value.assign_model_value(&mut model, x));
                        assert!(factor.assign_model_value(&mut model, y));
                        assert_eq!(
                            constraints.iter().all(|c| c.eval_model(&model).unwrap()),
                            normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                            "minimum={minimum}, value={x}, factor={y}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_round_trip_revisits_late_guard_rewrites() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "credits");
    let rate = SymExpr::var(&mut cx, "rate");
    let w = U256::from(1_000_000_000_000_000_000u64);
    let scale = SymExpr::constant(&mut cx, w);
    let scaled = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), scale.clone());
    let balance = SymExpr::binop(&mut cx, SymBinOp::UDiv, scaled, scale);
    let balance_guard = SymBoolExpr::eq(&mut cx, balance.clone(), value.clone());
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, balance.clone(), rate.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product.clone(), balance.clone());
    let exact = SymBoolExpr::eq(&mut cx, quotient, rate.clone());
    let zero = SymExpr::zero(&mut cx);
    let empty = SymBoolExpr::eq(&mut cx, balance.clone(), zero);
    let product_guard = SymBoolExpr::or(&mut cx, vec![empty, exact]);
    let converted = rounded_conversion(&mut cx, balance, rate.clone(), w);
    let identity = SymBoolExpr::eq(&mut cx, converted, value);
    let constraints = vec![
        balance_guard,
        product_guard,
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &rate, w),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &product, U256::MAX - w),
        identity.not(&mut cx),
    ];
    let mut cache = HashMap::default();
    for constraints in [constraints.clone(), constraints.into_iter().rev().collect()] {
        let normalized = normalize_constraints_for_solver_cached(&mut cx, &constraints, &mut cache);
        assert!(constraints_are_directly_unsat(&mut cx, &normalized), "{normalized:?}");
    }
}

#[test]
fn successful_mul_guards_prove_round_trip_without_rate_cap() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "balance");
    let rate = SymExpr::var(&mut cx, "rate");
    let w = U256::from(1_000_000_000_000_000_000u64);
    let scale = SymExpr::constant(&mut cx, w);
    let zero = SymExpr::zero(&mut cx);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), rate.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product.clone(), value.clone());
    let direct_guard = SymBoolExpr::eq(&mut cx, quotient.clone(), rate.clone());
    let scaled = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), scale);
    let is_zero = SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &scaled, w);
    let guarded = SymExpr::ite(&mut cx, is_zero.clone(), zero.clone(), quotient);
    let exact = SymBoolExpr::eq(&mut cx, guarded, rate.clone());
    let zero_word = SymExpr::bool_word(&mut cx, is_zero);
    let exact_word = SymExpr::bool_word(&mut cx, exact);
    let guard_word = SymExpr::binop(&mut cx, SymBinOp::Or, zero_word, exact_word);
    let word_guard = SymBoolExpr::eq(&mut cx, guard_word, zero).not(&mut cx);
    let converted = rounded_conversion(&mut cx, value.clone(), rate.clone(), w);
    let identity = SymBoolExpr::eq(&mut cx, converted, value.clone());
    let bounds = vec![
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::from(u128::MAX)),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &rate, w),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &product, U256::MAX - w),
    ];
    for guard in [direct_guard, word_guard] {
        // A successful multiplication does not establish safety of the rounding addition.
        let mut missing_add_guard = bounds[..2].to_vec();
        missing_add_guard.push(guard.clone());
        missing_add_guard.push(identity.clone().not(&mut cx));
        let normalized = normalize_constraints_for_solver(&mut cx, &missing_add_guard);
        let mut model = SymbolicModel::default();
        assert!(value.assign_model_value(&mut model, U256::ONE));
        assert!(rate.assign_model_value(&mut model, U256::MAX));
        assert!(missing_add_guard.iter().all(|c| c.eval_model(&model).unwrap()));
        assert!(normalized.iter().all(|c| c.eval_model(&model).unwrap()));

        let mut constraints = bounds.clone();
        constraints.push(guard);
        constraints.push(identity.clone().not(&mut cx));
        let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
        assert!(constraints_are_directly_unsat(&mut cx, &normalized));

        // Retained premises must still reject wrapping multiplication or rounding addition.
        constraints.pop();
        constraints.push(identity.clone());
        let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
        for b in [U256::ZERO, U256::ONE, U256::from(2), U256::from(u128::MAX)] {
            for p in [U256::ZERO, w - U256::ONE, w, w + U256::ONE, U256::MAX - w, U256::MAX] {
                let mut model = SymbolicModel::default();
                assert!(value.assign_model_value(&mut model, b));
                assert!(rate.assign_model_value(&mut model, p));
                assert_eq!(
                    constraints.iter().all(|c| c.eval_model(&model).unwrap()),
                    normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                    "balance={b}, rate={p}"
                );
            }
        }
    }

    // A bound on the modular product is insufficient without its successful overflow guard.
    let mut constraints = bounds;
    constraints.push(identity.not(&mut cx));
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, U256::from(2)));
    assert!(rate.assign_model_value(&mut model, U256::ONE << 255));
    assert!(constraints.iter().all(|c| c.eval_model(&model).unwrap()));
    assert!(normalized.iter().all(|c| c.eval_model(&model).unwrap()));
}

#[test]
fn multiplication_facts_require_exact_success_guards() {
    let mut cx = SymCx::new();
    let a = SymExpr::var(&mut cx, "a");
    let b = SymExpr::var(&mut cx, "b");
    let unrelated = SymExpr::var(&mut cx, "unrelated");
    let zero = SymExpr::zero(&mut cx);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, a.clone(), b.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, a.clone());
    let exact = SymBoolExpr::eq(&mut cx, quotient.clone(), b.clone());
    let a_zero = SymBoolExpr::eq(&mut cx, a.clone(), zero.clone());
    let unrelated_zero = SymBoolExpr::eq(&mut cx, unrelated.clone(), zero.clone());
    let valid_or = SymBoolExpr::or(&mut cx, vec![a_zero.clone(), exact.clone()]);
    let extra_or =
        SymBoolExpr::or(&mut cx, vec![a_zero.clone(), exact.clone(), unrelated_zero.clone()]);
    let wrong_or = SymBoolExpr::or(&mut cx, vec![unrelated_zero.clone(), exact.clone()]);
    let wrong_ite = SymExpr::ite(&mut cx, unrelated_zero, zero.clone(), quotient.clone());
    let wrong_exact = SymBoolExpr::eq(&mut cx, wrong_ite, b.clone());
    let wrong_guard = SymBoolExpr::or(&mut cx, vec![a_zero.clone(), wrong_exact]);
    let guarded = SymExpr::ite(&mut cx, a_zero.clone(), zero, quotient);
    let guarded_exact = SymBoolExpr::eq(&mut cx, guarded, b.clone());
    let valid_guard = SymBoolExpr::or(&mut cx, vec![a_zero, guarded_exact.clone()]);
    for (predicate, recognized) in [
        (exact.clone(), true),
        (valid_or, true),
        (valid_guard, true),
        (exact.not(&mut cx), false),
        (extra_or, false),
        (wrong_or, false),
        (wrong_guard, false),
        (guarded_exact, false),
    ] {
        let mut context = ConstraintContext::default();
        context.record_non_wrapping_product(&mut cx, &predicate);
        assert_eq!(context.mul_cannot_overflow_256(&a, &b), recognized);
        // A guard cannot establish itself; even recognized guards must stay constrained.
        let normalized =
            normalize_constraints_for_solver(&mut cx, std::slice::from_ref(&predicate));
        for av in [U256::ZERO, U256::ONE, U256::from(2), U256::ONE << 255, U256::MAX] {
            for bv in [U256::ZERO, U256::ONE, U256::from(3), U256::MAX] {
                for uv in [U256::ZERO, U256::ONE] {
                    let mut model = SymbolicModel::default();
                    assert!(a.assign_model_value(&mut model, av));
                    assert!(b.assign_model_value(&mut model, bv));
                    assert!(unrelated.assign_model_value(&mut model, uv));
                    assert_eq!(
                        predicate.eval_model(&model).unwrap(),
                        normalized.iter().all(|c| c.eval_model(&model).unwrap())
                    );
                }
            }
        }
    }
}

#[test]
fn successful_product_guards_preserve_lower_bounds() {
    let mut cx = SymCx::new();
    let balance = SymExpr::var(&mut cx, "balance");
    let rate = SymExpr::var(&mut cx, "rate");
    let w = U256::from(1_000_000_000_000_000_000u64);
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, balance.clone(), rate.clone());
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product.clone(), balance.clone());
    let guard = SymBoolExpr::eq(&mut cx, quotient, rate.clone());
    let scale = SymExpr::constant(&mut cx, w);
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, product.clone(), scale);
    let one = SymExpr::one(&mut cx);
    let numerator = SymExpr::binop(&mut cx, SymBinOp::Sub, sum, one);
    let mut constraints = vec![
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ugt, &balance, U256::ZERO),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &balance, U256::from(u128::MAX)),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &rate, w),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &product, U256::MAX - w),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &numerator, w),
    ];
    let mut model = SymbolicModel::default();
    assert!(balance.assign_model_value(&mut model, U256::from(2)));
    assert!(rate.assign_model_value(&mut model, U256::ONE << 255));
    // Without the guard, wrapping can turn a positive balance into zero credits.
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    assert!(constraints.iter().all(|c| c.eval_model(&model).unwrap()));
    assert!(normalized.iter().all(|c| c.eval_model(&model).unwrap()));
    constraints.push(guard);
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    assert!(constraints_are_directly_unsat(&mut cx, &normalized));
}

#[test]
fn rounded_conversion_does_not_assume_its_own_validity() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "balance");
    let rate = SymExpr::var(&mut cx, "rate");
    let converted = rounded_conversion(&mut cx, value.clone(), rate.clone(), U256::from(2));
    let equality = SymBoolExpr::eq(&mut cx, converted, value.clone());
    let normalized = normalize_constraints_for_solver(&mut cx, std::slice::from_ref(&equality));
    for (b, p) in [
        (U256::ONE, U256::ONE),
        (U256::ONE << 255, U256::from(2)),
        (U256::MAX, U256::from(2)),
        (U256::ONE, U256::ZERO),
    ] {
        let mut model = SymbolicModel::default();
        assert!(value.assign_model_value(&mut model, b));
        assert!(rate.assign_model_value(&mut model, p));
        let expected = equality.eval_model(&model).unwrap();
        assert!(!expected);
        assert_eq!(normalized.iter().all(|c| c.eval_model(&model).unwrap()), expected);
    }
}

#[test]
fn bounded_comparisons_retain_their_own_assumptions() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), y.clone());
    let ten = SymExpr::constant(&mut cx, U256::from(10));
    let constraint = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, product, ten);
    let normalized = normalize_constraints_for_solver(&mut cx, std::slice::from_ref(&constraint));
    for a in 0..16 {
        for b in 0..16 {
            let mut model = SymbolicModel::default();
            assert!(x.assign_model_value(&mut model, U256::from(a)));
            assert!(y.assign_model_value(&mut model, U256::from(b)));
            assert_eq!(
                normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                constraint.eval_model(&model).unwrap()
            );
        }
    }
}

#[test]
fn mutually_supporting_bounds_retain_one_constraint() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let constraints = [
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &x, U256::from(5)),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &x, U256::from(6)),
    ];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);

    for (value, expected) in [(U256::from(5), true), (U256::from(6), false)] {
        let mut model = SymbolicModel::default();
        assert!(x.assign_model_value(&mut model, value));
        assert_eq!(normalized.iter().all(|c| c.eval_model(&model).unwrap()), expected);
    }
}

#[test]
fn rounded_conversion_rejects_bounds_on_wrapped_products() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "balance");
    let rate = SymExpr::var(&mut cx, "rate");
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), rate.clone());
    let zero = SymExpr::zero(&mut cx);
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let wrapped_bound = SymBoolExpr::eq(&mut cx, product, zero);
    let fixed_rate = SymBoolExpr::eq(&mut cx, rate.clone(), two);
    let converted = rounded_conversion(&mut cx, value.clone(), rate.clone(), U256::from(2));
    let equality = SymBoolExpr::eq(&mut cx, converted, value.clone());
    let constraints = [wrapped_bound, fixed_rate, equality];
    let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, U256::ONE << 255));
    assert!(rate.assign_model_value(&mut model, U256::from(2)));
    assert!(!constraints.iter().all(|c| c.eval_model(&model).unwrap()));
    assert!(!normalized.iter().all(|c| c.eval_model(&model).unwrap()));
}

#[test]
fn nonnegative_signed_addition_preserves_overflow_paths() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let signed_max = U256::MAX >> 1;
    let maximum = SymExpr::constant(&mut cx, signed_max);
    let bounds = [
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, x.clone(), maximum.clone()),
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, y.clone(), maximum),
    ];
    let context = ConstraintContext::new(&bounds);
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, x.clone(), y.clone());
    let guard = SymBoolExpr::cmp(&mut cx, SymCmpOp::Slt, sum.clone(), x.clone());
    let zero = SymExpr::zero(&mut cx);
    let cast_guard = SymBoolExpr::cmp(&mut cx, SymCmpOp::Slt, sum, zero);
    let normalized = context.normalize_signed_add_comparison(&mut cx, &guard).unwrap();
    assert!(
        ConstraintContext::default().normalize_signed_add_comparison(&mut cx, &guard).is_none()
    );
    assert_eq!(
        context.normalize_signed_add_comparison(&mut cx, &cast_guard),
        Some(normalized.clone())
    );
    for a in [U256::ZERO, U256::ONE, signed_max - U256::ONE, signed_max] {
        for b in [U256::ZERO, U256::ONE, signed_max - U256::ONE, signed_max] {
            let mut model = SymbolicModel::default();
            assert!(x.assign_model_value(&mut model, a));
            assert!(y.assign_model_value(&mut model, b));
            assert_eq!(guard.eval_model(&model).unwrap(), normalized.eval_model(&model).unwrap());
            assert_eq!(
                cast_guard.eval_model(&model).unwrap(),
                normalized.eval_model(&model).unwrap()
            );
        }
    }
}

#[test]
fn opposite_sign_addition_and_unsigned_cast_preserve_boundaries() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let zero = SymExpr::zero(&mut cx);
    let neg_y = SymExpr::binop(&mut cx, SymBinOp::Sub, zero.clone(), y.clone());
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, x.clone(), neg_y.clone());
    let maximum = U256::MAX >> 1;
    let bounds = [
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &x, maximum),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &y, maximum),
        SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ugt, &y, U256::ZERO),
    ];
    let context = ConstraintContext::new(&bounds);
    for compared in [neg_y, x.clone(), zero] {
        let guard = SymBoolExpr::cmp(&mut cx, SymCmpOp::Slt, sum.clone(), compared);
        let rewritten = context.normalize_signed_add_comparison(&mut cx, &guard).unwrap();
        assert!(
            ConstraintContext::default().normalize_signed_add_comparison(&mut cx, &guard).is_none()
        );
        for a in [U256::ZERO, U256::ONE, maximum - U256::ONE, maximum] {
            for b in [U256::ONE, maximum - U256::ONE, maximum] {
                let mut model = SymbolicModel::default();
                assert!(x.assign_model_value(&mut model, a));
                assert!(y.assign_model_value(&mut model, b));
                assert_eq!(
                    guard.eval_model(&model).unwrap(),
                    rewritten.eval_model(&model).unwrap()
                );
            }
        }
    }
    // A subtraction interval spanning both wrapping and non-wrapping results stays unknown.
    let context = ConstraintContext::new(&bounds[..2]);
    let zero = SymExpr::zero(&mut cx);
    let neg_y = SymExpr::binop(&mut cx, SymBinOp::Sub, zero, y);
    assert!(context.interval(&neg_y).is_none());
}

#[test]
fn signed_interval_checks_do_not_cross_the_sign_boundary() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let zero = SymExpr::zero(&mut cx);
    let sign = SymExpr::constant(&mut cx, U256::ONE << 255);
    let maximum = SymExpr::constant(&mut cx, U256::MAX >> 1);
    let negative = SymBoolExpr::cmp(&mut cx, SymCmpOp::Slt, x.clone(), zero);
    let positive_bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, x.clone(), maximum);
    let negative_bound = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, sign, x);
    assert_eq!(ConstraintContext::default().bounded_bool_value(&negative), None);
    assert_eq!(
        ConstraintContext::new(&[positive_bound]).bounded_bool_value(&negative),
        Some(false)
    );
    assert_eq!(ConstraintContext::new(&[negative_bound]).bounded_bool_value(&negative), Some(true));
}

#[test]
fn contextual_boolean_word_mask_keeps_the_condition() {
    let mut cx = SymCx::new();
    let x = SymExpr::var(&mut cx, "x");
    let y = SymExpr::var(&mut cx, "y");
    let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), y.clone());
    let word = SymExpr::bool_word(&mut cx, condition.clone());
    let one = SymExpr::one(&mut cx);
    let masked = SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::And, word, one));
    let zero = SymExpr::zero(&mut cx);
    let constraint = SymBoolExpr::eq(&mut cx, masked, zero);
    let normalized = normalize_constraints_for_solver(&mut cx, &[constraint]);
    for a in [U256::ZERO, U256::ONE, U256::MAX] {
        for b in [U256::ZERO, U256::ONE, U256::MAX] {
            let mut model = SymbolicModel::default();
            assert!(x.assign_model_value(&mut model, a));
            assert!(y.assign_model_value(&mut model, b));
            assert_eq!(
                normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                !condition.eval_model(&model).unwrap()
            );
        }
    }
    assert_eq!(normalized, vec![condition.not(&mut cx)]);
}

#[test]
fn round_trip_keeps_independent_lower_bounds_in_both_constraint_orders() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let rate = SymExpr::var(&mut cx, "rate");
    let scale = U256::from(1_000_000_000_000_000_000u64);
    let maximum_rate = scale * U256::from(1_000_000_000u64);
    let converted = rounded_conversion(&mut cx, value.clone(), rate.clone(), scale);
    let identity = SymBoolExpr::eq(&mut cx, converted, value.clone());
    let mut cache = HashMap::default();
    for include_lower in [false, true] {
        for negate in [false, true] {
            let mut constraints = vec![
                SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::from(u128::MAX)),
                SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &rate, maximum_rate),
                if negate { identity.clone().not(&mut cx) } else { identity.clone() },
            ];
            if include_lower {
                constraints.push(SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &rate, scale));
            }
            for _ in 0..2 {
                let normalized =
                    normalize_constraints_for_solver_cached(&mut cx, &constraints, &mut cache);
                for input in
                    [U256::ZERO, U256::ONE, U256::from(u128::MAX), U256::ONE << 255, U256::MAX]
                {
                    for input_rate in [
                        U256::ZERO,
                        U256::ONE,
                        scale - U256::ONE,
                        scale,
                        scale + U256::ONE,
                        maximum_rate,
                        U256::MAX,
                    ] {
                        let mut model = SymbolicModel::default();
                        assert!(value.assign_model_value(&mut model, input));
                        assert!(rate.assign_model_value(&mut model, input_rate));
                        assert_eq!(
                            constraints.iter().all(|c| c.eval_model(&model).unwrap()),
                            normalized.iter().all(|c| c.eval_model(&model).unwrap()),
                            "lower={include_lower}, negate={negate}, value={input}, rate={input_rate}"
                        );
                    }
                }
                constraints.reverse();
            }
        }
    }
}

#[test]
fn contextual_normalization_keeps_wrapping_scaled_division() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), two.clone());
    let zero = SymExpr::zero(&mut cx);
    let wrapped_product_is_zero = SymBoolExpr::eq(&mut cx, product.clone(), zero);
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, two);
    let identity = SymBoolExpr::eq(&mut cx, quotient, value.clone());

    let normalized =
        normalize_constraints_for_solver(&mut cx, &[wrapped_product_is_zero, identity]);

    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, U256::ONE << 255));
    assert!(normalized.iter().any(|constraint| !constraint.eval_model(&model).unwrap()));
}

#[test]
fn contextual_normalization_keeps_wrapping_round_up_division() {
    let mut cx = SymCx::new();
    let value = SymExpr::var(&mut cx, "value");
    let two = SymExpr::constant(&mut cx, U256::from(2));
    let product = SymExpr::binop(&mut cx, SymBinOp::Mul, value.clone(), two.clone());
    let sum = SymExpr::binop(&mut cx, SymBinOp::Add, product, two.clone());
    let one = SymExpr::one(&mut cx);
    let rounded = SymExpr::binop(&mut cx, SymBinOp::Sub, sum, one.clone());
    let wrapped_numerator_is_bounded =
        SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, rounded.clone(), one);
    let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, rounded, two);
    let identity = SymBoolExpr::eq(&mut cx, quotient, value.clone());

    let normalized =
        normalize_constraints_for_solver(&mut cx, &[wrapped_numerator_is_bounded, identity]);

    let mut model = SymbolicModel::default();
    assert!(value.assign_model_value(&mut model, U256::ONE << 255));
    assert!(normalized.iter().any(|constraint| !constraint.eval_model(&model).unwrap()));
}

#[test]
fn unique_arithmetic_chains_stop_at_analysis_budget() {
    let mut cx = SymCx::new();
    let zero = SymExpr::zero(&mut cx);
    let one = SymExpr::one(&mut cx);
    let mut shifted = one.clone();
    let mut divided = one.clone();
    for _ in 0..MAX_LOCAL_ANALYSIS_NODES {
        shifted =
            SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::Shr, shifted, zero.clone()));
        divided =
            SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::UDiv, divided, one.clone()));
    }

    let context = ConstraintContext::default();
    assert!(context.interval(&shifted).is_none());
    assert_eq!(context.unsigned_bits(&divided), 256);
    assert!(!context.mul_cannot_overflow_256(&divided, &divided));
}
