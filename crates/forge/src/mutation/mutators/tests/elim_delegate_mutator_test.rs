use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{
        elim_delegate_mutator::ElimDelegateMutator, tests::helper::*, MutationContext, Mutator,
    },
    visitor::AssignVarTypes,
    Session,
};
use solar_parse::{
    ast::{
        Arena, BinOp, BinOpKind, CallArgs, ElementaryType, Expr, ExprKind, Ident, Lit, LitKind,
        Span, Symbol, Type, TypeKind, VariableDefinition,
    },
    interface::BytePos,
};

use super::*;

#[test]
fn test_is_applicable_for_delegate_expr() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

        let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

        let ident = arena.alloc(Ident { name: Symbol::intern("delegatecall"), span });

        let member = arena.alloc(Expr { kind: ExprKind::Member(left, *ident), span });

        let expr = arena.alloc(Expr { kind: ExprKind::Call(member, CallArgs::default()), span });

        let context = MutationContext { expr: Some(expr), var_definition: None, span };

        let mutator = ElimDelegateMutator;
        assert!(mutator.is_applicable(&context));
        Ok(())
    });
}

#[test]
fn test_generate_delete_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

        let left = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

        let ident = arena.alloc(Ident { name: Symbol::intern("delegatecall"), span });

        let member = arena.alloc(Expr { kind: ExprKind::Member(left, *ident), span });

        let expr = arena.alloc(Expr { kind: ExprKind::Call(member, CallArgs::default()), span });

        let context = MutationContext { expr: Some(expr), var_definition: None, span };

        let mutator = ElimDelegateMutator;
        let mutants = mutator.generate_mutants(&context).unwrap();

        assert_eq!(mutants.len(), 1);

        if let MutationType::ElimDelegate = &mutants[0].mutation {
            assert!(true);
        } else {
            panic!("Expected delegatecall mutation");
        }

        Ok(())
    });
}
