use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{assignement_mutator::AssignmentMutator, MutationContext, Mutator},
    visitor::AssignVarTypes,
    Session,
};
use solar_parse::{
    ast::{
        Arena, ElementaryType, Expr, ExprKind, Ident, Lit, LitKind, Span, Symbol, Type, TypeKind,
        VariableDefinition,
    },
    interface::BytePos,
};

use super::*;

fn create_span(start: u32, end: u32) -> Span {
    Span::new(BytePos(start), BytePos(end))
}

#[test]
fn test_is_applicable_for_assign_expr() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    // x = 23
    let left = arena.alloc(Expr {
        kind: ExprKind::Ident(Ident { name: Symbol::DUMMY, span }), /* we use dummy symbol to
                                                                     * avoid having to enter a
                                                                     * session */
        span,
    });

    let mut val = Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(23.into()) };

    let right = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let expr = arena.alloc(Expr { kind: ExprKind::Assign(left, None, right), span });

    let context = MutationContext { expr: Some(expr), var_definition: None, span };

    let mutator = AssignmentMutator;
    assert!(mutator.is_applicable(&context));
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

#[test]
fn test_generate_bool_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let mut val = Lit { span, symbol: Symbol::default(), kind: LitKind::Bool(true) };

        let lit = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

        let context = MutationContext { expr: Some(lit), var_definition: None, span };

        let mutator = AssignmentMutator;
        let mutants = mutator.generate_mutants(&context).unwrap();

        assert_eq!(mutants.len(), 1);

        if let MutationType::Assignment(AssignVarTypes::Literal(LitKind::Bool(val))) =
            &mutants[0].mutation
        {
            assert_eq!(*val, false);
        } else {
            panic!("Expected boolean mutation");
        }
        Ok(())
    });
}

#[test]
fn test_generate_number_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let mut val = Lit { span, symbol: Symbol::default(), kind: LitKind::Number(42.into()) };

    let lit = arena.alloc(Expr { kind: ExprKind::Lit(&mut val, None), span });

    let context = MutationContext { expr: Some(lit), var_definition: None, span };

    let mutator = AssignmentMutator;
    let mutants = mutator.generate_mutants(&context).unwrap();

    assert_eq!(mutants.len(), 2);

    // First mutant should set to zero
    if let MutationType::Assignment(AssignVarTypes::Literal(LitKind::Number(val))) =
        &mutants[0].mutation
    {
        assert_eq!(*val, num_bigint::BigInt::ZERO);
    } else {
        panic!("Expected number mutation to zero");
    }

    // Second mutant should negate the value
    if let MutationType::Assignment(AssignVarTypes::Literal(LitKind::Number(val))) =
        &mutants[1].mutation
    {
        assert_eq!(*val, -num_bigint::BigInt::from(42));
    } else {
        panic!("Expected negated number mutation");
    }
}

#[test]
fn test_generate_identifier_mutants() {
    let arena = Arena::new();
    let span = create_span(10, 20);

    let sess = Session::builder().with_silent_emitter(None).build();

    let _ = sess.enter(|| -> solar_parse::interface::Result<()> {
        let expr = arena.alloc(Expr {
            kind: ExprKind::Ident(Ident { name: Symbol::intern("varia"), span }),
            span,
        });

        let context = MutationContext { expr: Some(expr), var_definition: None, span };

        let mutator = AssignmentMutator;
        let mutants = mutator.generate_mutants(&context).unwrap();

        assert_eq!(mutants.len(), 2);

        // First mutant should set to zero
        if let MutationType::Assignment(AssignVarTypes::Literal(LitKind::Number(val))) =
            &mutants[0].mutation
        {
            assert_eq!(*val, num_bigint::BigInt::ZERO);
        } else {
            panic!("Expected number mutation to zero");
        }

        // Second mutant should negate the identifier
        if let MutationType::Assignment(AssignVarTypes::Identifier(val)) = &mutants[1].mutation {
            assert_eq!(val, "-variable");
        } else {
            panic!("Expected negated identifier mutation");
        }

        Ok(())
    });
}
