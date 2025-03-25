use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{binary_op_mutator::BinaryOpMutator, tests::helper::*, MutationContext, Mutator},
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
fn test_is_applicable_for_binary_expr() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };
    let mut val2 = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(45.into()) };

    let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val2, None), span });

    let bin_op = BinOp { span, kind: BinOpKind::Add };

    let expr = arena.alloc(Expr { kind: ExprKind::Binary(left, bin_op, right), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = BinaryOpMutator;
    assert!(mutator.is_applicable(&context));
}

#[test]
fn test_is_applicable_for_assign_expr() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let left =
        arena.alloc(Expr { kind: ExprKind::Ident(Ident { name: Symbol::DUMMY, span }), span });

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let bin_op = BinOp { span, kind: BinOpKind::Add };

    let expr = arena.alloc(Expr { kind: ExprKind::Assign(left, Some(bin_op), right), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = BinaryOpMutator;
    assert!(mutator.is_applicable(&context));
}

#[test]
fn test_is_not_applicable_assign_without_binary_op() {
    let arena = Arena::new();

    let span = create_span(10, 20);

    let left =
        arena.alloc(Expr { kind: ExprKind::Ident(Ident { name: Symbol::DUMMY, span }), span });

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let expr = arena.alloc(Expr { kind: ExprKind::Assign(left, None, right), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = BinaryOpMutator;
    assert!(!mutator.is_applicable(&context));
}

#[test]
fn test_generate_arithmetic_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };
        let mut val2 = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(45.into()) };

        let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

        let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val2, None), span });

        let bin_op = BinOp { span, kind: BinOpKind::Add };

        let expr = arena.alloc(Expr { kind: ExprKind::Binary(left, bin_op, right), span });

        let context = MutationContext { expr: Some(expr), var_definition: None, span };

        let mutator = BinaryOpMutator;
        let mutants = mutator.generate_mutants(&context).unwrap();

        let operations_num_bitwise = vec![
            // Arithm
            BinOpKind::Shr,
            BinOpKind::Shl,
            BinOpKind::Sar,
            BinOpKind::BitAnd,
            BinOpKind::BitOr,
            BinOpKind::BitXor,
            BinOpKind::Add,
            BinOpKind::Sub,
            BinOpKind::Pow,
            BinOpKind::Mul,
            BinOpKind::Div,
            BinOpKind::Rem,
        ];

        assert_eq!(mutants.len(), operations_num_bitwise.len() - 1);

        let mutants_kind = mutants
            .iter()
            .map(|m| match m.mutation {
                MutationType::BinaryOp(kind) => kind,
                _ => panic!("Expected binary op mutant"),
            })
            .collect::<Vec<_>>();

        assert!(all_but_one(&operations_num_bitwise, &mutants_kind));

        Ok(())
    });
}

#[test]
fn test_generate_bool_op_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };
        let mut val2 = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(45.into()) };

        let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

        let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val2, None), span });

        let bin_op = BinOp { span, kind: BinOpKind::Lt };

        let expr = arena.alloc(Expr { kind: ExprKind::Binary(left, bin_op, right), span });

        let context = MutationContext { expr: Some(expr), var_definition: None, span };

        let mutator = BinaryOpMutator;
        let mutants = mutator.generate_mutants(&context).unwrap();

        let operations_bools = vec![
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
            BinOpKind::Or,
            BinOpKind::And,
        ];

        assert_eq!(mutants.len(), operations_bools.len() - 1);

        let mutants_kind = mutants
            .iter()
            .map(|m| match m.mutation {
                MutationType::BinaryOp(kind) => kind,
                _ => panic!("Expected binary op mutant"),
            })
            .collect::<Vec<_>>();

        assert!(all_but_one(&operations_bools, &mutants_kind));

        Ok(())
    });
}
