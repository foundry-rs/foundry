use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{
        delete_expression_mutator::DeleteExpressionMutator, tests::helper::*, MutationContext,
        Mutator,
    },
    visitor::AssignVarTypes,
    Session,
};
use solar_parse::{
    ast::{
        Arena, BinOp, BinOpKind, ElementaryType, Expr, ExprKind, Ident, Lit, LitKind, Span, Symbol,
        Type, TypeKind, VariableDefinition,
    },
    interface::BytePos,
};

use super::*;

#[test]
fn test_is_applicable_for_delete_expr() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let expr = arena.alloc(Expr { kind: ExprKind::Delete(left), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = DeleteExpressionMutator;
    assert!(mutator.is_applicable(&context));
}

#[test]
fn test_generate_delete_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let expr = arena.alloc(Expr { kind: ExprKind::Delete(left), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = DeleteExpressionMutator;
    let mutants = mutator.generate_mutants(&context).unwrap();

    assert_eq!(mutants.len(), 1);
    // assert_eq!(mutants[0].mutation, MutationType::DeleteExpression);

    if let MutationType::DeleteExpression = &mutants[0].mutation {
        assert!(true);
    } else {
        panic!("Expected delete mutation");
    }
}
