use crate::mutation::{
    mutant::{Mutant, MutationType, OwnedLiteral},
    mutators::{MutationContext, Mutator},
    visitor::AssignVarTypes,
};

use alloy_primitives::U256;
use eyre::Result;
use solar::ast::{ExprKind, Span};

pub struct AssignmentMutator;

impl Mutator for AssignmentMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let (assign_var_type, replacement_span) = match extract_rhs_info(context) {
            Some(info) => info,
            None => return Ok(vec![]), // is_applicable should filter this
        };

        let original = context.original_text();
        let source_line = context.source_line();
        let line_number = context.line_number();

        match assign_var_type {
            AssignVarTypes::Literal(lit) => match lit {
                OwnedLiteral::Bool(val) => Ok(vec![Mutant {
                    span: replacement_span,
                    mutation: MutationType::Assignment(AssignVarTypes::Literal(
                        OwnedLiteral::Bool(!val),
                    )),
                    path: context.path.clone(),
                    original,
                    source_line,
                    line_number,
                }]),
                OwnedLiteral::Number(val) => Ok(vec![
                    Mutant {
                        span: replacement_span,
                        mutation: MutationType::Assignment(AssignVarTypes::Literal(
                            OwnedLiteral::Number(U256::ZERO),
                        )),
                        path: context.path.clone(),
                        original: original.clone(),
                        source_line: source_line.clone(),
                        line_number,
                    },
                    Mutant {
                        span: replacement_span,
                        mutation: MutationType::Assignment(AssignVarTypes::Literal(
                            OwnedLiteral::Number(-val),
                        )),
                        path: context.path.clone(),
                        original,
                        source_line,
                        line_number,
                    },
                ]),
                // todo: should we bail instead of returning an empty vec?
                OwnedLiteral::Str { .. } => Ok(vec![]),
                OwnedLiteral::Rational(_) => Ok(vec![]),
                OwnedLiteral::Address(_) => Ok(vec![]),
                OwnedLiteral::Err(_) => Ok(vec![]),
            },
            AssignVarTypes::Identifier(ident) => Ok(vec![
                Mutant {
                    span: replacement_span,
                    mutation: MutationType::Assignment(AssignVarTypes::Literal(
                        OwnedLiteral::Number(U256::ZERO),
                    )),
                    path: context.path.clone(),
                    original: original.clone(),
                    source_line: source_line.clone(),
                    line_number,
                },
                Mutant {
                    span: replacement_span,
                    mutation: MutationType::Assignment(AssignVarTypes::Identifier(format!(
                        "-{ident}"
                    ))),
                    path: context.path.clone(),
                    original,
                    source_line,
                    line_number,
                },
            ]),
        }
    }

    /// Match is the expr is an assign with a var definition having a literal or identifier as
    /// initializer
    fn is_applicable(&self, context: &MutationContext<'_>) -> bool {
        if let Some(expr) = context.expr {
            if let ExprKind::Assign(_lhs, _op_opt, rhs_actual_expr) = &expr.kind {
                matches!(rhs_actual_expr.kind, ExprKind::Lit(..) | ExprKind::Ident(..))
            } else {
                false // Not an assign
            }
        } else if let Some(var_definition) = context.var_definition {
            if let Some(init) = &var_definition.initializer {
                matches!(&init.kind, ExprKind::Lit(..) | ExprKind::Ident(..))
            } else {
                false // No initializer
            }
        } else {
            false // Not an expression or var_definition
        }
    }
}

fn extract_rhs_info<'ast>(context: &MutationContext<'ast>) -> Option<(AssignVarTypes, Span)> {
    let relevant_expr_for_rhs = if let Some(var_definition) = context.var_definition {
        var_definition.initializer.as_ref()?
    } else if let Some(expr) = context.expr {
        match &expr.kind {
            ExprKind::Assign(_lhs, _op_opt, rhs_actual_expr) => &**rhs_actual_expr,
            // If the context.expr is already what we want to get the type from
            // (e.g. a simple Lit or Ident being passed directly, though is_applicable filters this)
            ExprKind::Lit(..) | ExprKind::Ident(..) => expr,
            _ => return None,
        }
    } else {
        return None; // No var_definition or expr in context (shouldn't happen?)
    };

    match &relevant_expr_for_rhs.kind {
        ExprKind::Lit(kind, _) => {
            let owned = OwnedLiteral::from(&kind.kind);
            Some((AssignVarTypes::Literal(owned), relevant_expr_for_rhs.span))
        }
        ExprKind::Ident(val) => {
            Some((AssignVarTypes::Identifier(val.to_string()), relevant_expr_for_rhs.span))
        }
        _ => None,
    }
}
