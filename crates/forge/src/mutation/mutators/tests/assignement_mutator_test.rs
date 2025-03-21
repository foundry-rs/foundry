use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{assignement_mutator::AssignmentMutator, MutationContext, Mutator},
    visitor::AssignVarTypes,
};
use solar_parse::{
    ast::{
        Arena, ElementaryType, Expr, ExprKind, Ident, Lit, LitKind, Span, Symbol, Type, TypeKind,
        VariableDefinition,
    },
    interface::BytePos,
};

use num_bigint::BigInt;
use std::path::PathBuf;

use crate::mutation::Session;

fn create_span(start: u32, end: u32) -> Span {
    Span::new(BytePos(start), BytePos(end))
}

fn create_ident<'ident>(ident: String) -> ExprKind<'ident> {
    ExprKind::Ident(Ident::from_str(&ident))
}

#[test]
fn test_is_applicable_for_assign_expr() {
    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let arena = Arena::new();
        let span = create_span(10, 20);

        // x = 23
        let left = arena.alloc(Expr { kind: create_ident("x".into()), span });

        let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

        let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

        let expr = arena.alloc(Expr { kind: ExprKind::Assign(left, None, right), span });

        let context = MutationContext { expr: Some(expr), var_definition: None, span };

        let mutator = AssignmentMutator;
        assert!(mutator.is_applicable(&context));

        Ok(())
    });
}

#[test]
fn test_is_applicable_for_var_definition() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let lit = arena.alloc(Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) });

    let init = arena.alloc(Expr { kind: ExprKind::Lit(lit, None), span });

    let var_def = VariableDefinition {
        name: None,
        initializer: Some(init),
        span,
        ty: Type { kind: TypeKind::Elementary(ElementaryType::Bool), span },
        visibility: None,
        mutability: None,
        data_location: None,
        override_: None,
        indexed: false,
    };

    let context = MutationContext { expr: None, var_definition: Some(&var_def), span };

    let mutator = AssignmentMutator;
    assert!(mutator.is_applicable(&context));
}

#[test]
fn test_is_not_applicable_no_initializer() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let var_def = VariableDefinition {
        name: None,
        initializer: None,
        span,
        ty: Type { kind: TypeKind::Elementary(ElementaryType::Bool), span },
        visibility: None,
        mutability: None,
        data_location: None,
        override_: None,
        indexed: false,
    };

    let context = MutationContext { expr: None, var_definition: Some(&var_def), span };

    let mutator = AssignmentMutator;
    assert!(!mutator.is_applicable(&context));
}
