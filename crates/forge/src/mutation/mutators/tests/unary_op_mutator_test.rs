use crate::mutation::{
    mutant::{Mutant, MutationType, UnaryOpMutated},
    mutators::{
        tests::helper::*, unary_op_mutator::UnaryOperatorMutator, MutationContext, Mutator,
    },
    visitor::AssignVarTypes,
    Session,
};
use solar_parse::{
    ast::{
        Arena, BinOp, BinOpKind, ElementaryType, Expr, ExprKind, Ident, Lit, LitKind, Span, Symbol,
        Type, TypeKind, UnOp, UnOpKind, VariableDefinition,
    },
    interface::BytePos,
};

use super::*;

#[test]
fn test_is_applicable_for_unary_expr() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let target = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let op = UnOp { span, kind: UnOpKind::Neg };

    let expr = arena.alloc(Expr { kind: ExprKind::Unary(op, target), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = UnaryOperatorMutator;
    assert!(mutator.is_applicable(&context));
}

#[test]
fn test_generate_prefixed_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);
    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let target = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let op = UnOp { span, kind: UnOpKind::Neg };

    let expr = arena.alloc(Expr { kind: ExprKind::Unary(op, target), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = UnaryOperatorMutator;
    let mutants = mutator.generate_mutants(&context).unwrap();

    let operations = vec![
        UnOpKind::PreInc,
        UnOpKind::PreDec,
        UnOpKind::Neg,
        UnOpKind::BitNot,
        UnOpKind::PostInc,
        UnOpKind::PostDec,
    ];

    assert_eq!(mutants.len(), operations.len() - 1);

    let mutants_kind = mutants
        .iter()
        .map(|m| match &m.mutation {
            MutationType::UnaryOperator(mutated) => mutated.resulting_op_kind,
            _ => panic!("Expected binary op mutant"),
        })
        .collect::<Vec<_>>();

    assert!(all_but_one(&operations, &mutants_kind));
}

#[test]
fn test_generate_bool_op_mutant() {
    let arena = Arena::new();
    let span = create_span(10, 20);
    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Bool(true) };

    let expr = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = UnaryOperatorMutator;
    let mutants = mutator.generate_mutants(&context).unwrap();

    assert_eq!(mutants.len(), 1);

    if let MutationType::UnaryOperator(mutated) = &mutants[0].mutation {
        assert_eq!(mutated.resulting_op_kind, UnOpKind::Not);
    } else {
        panic!("Expected negated identifier mutation");
    }
}
