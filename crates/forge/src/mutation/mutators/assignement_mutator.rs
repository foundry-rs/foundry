use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{MutationContext, Mutator},
    visitor::AssignVarTypes,
};

use eyre::Result;
use solar_parse::ast::{ExprKind, LitKind};
use std::path::PathBuf;

pub struct AssignmentMutator;

impl Mutator for AssignmentMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let assign_type = match determinate_type(context) {
            Some(t) => t,
            None => return Ok(vec![]),
        };

        let span = if let Some(var_definition) = context.var_definition {
            var_definition.initializer.as_ref().unwrap().span
        } else {
            context.span
        };

        match assign_type {
            AssignVarTypes::Literal(lit) => match lit {
                LitKind::Bool(val) => Ok(vec![Mutant {
                    span,
                    mutation: MutationType::Assignment(AssignVarTypes::Literal(LitKind::Bool(
                        !val,
                    ))),
                    path: PathBuf::default(),
                }]),
                LitKind::Number(val) => Ok(vec![
                    Mutant {
                        span,
                        mutation: MutationType::Assignment(AssignVarTypes::Literal(
                            LitKind::Number(num_bigint::BigInt::ZERO),
                        )),
                        path: PathBuf::default(),
                    },
                    Mutant {
                        span,
                        mutation: MutationType::Assignment(AssignVarTypes::Literal(
                            LitKind::Number(-val),
                        )),
                        path: PathBuf::default(),
                    },
                ]),
                _ => {
                    eyre::bail!("AssignementMutator: unexpected literal kind: {:?}", lit)
                }
            },
            AssignVarTypes::Identifier(ident) => Ok(vec![
                Mutant {
                    span,
                    mutation: MutationType::Assignment(AssignVarTypes::Literal(LitKind::Number(
                        num_bigint::BigInt::ZERO,
                    ))),
                    path: PathBuf::default(),
                },
                Mutant {
                    span,
                    mutation: MutationType::Assignment(AssignVarTypes::Identifier(format!(
                        "-{ident}"
                    ))),
                    path: PathBuf::default(),
                },
            ]),
        }
    }

    // Only match Assign expr (ie global var) and the var definition with a litteral initializer (ie
    // x = 6)
    fn is_applicable(&self, context: &MutationContext<'_>) -> bool {
        if let Some(expr) = context.expr {
            matches!(expr.kind, ExprKind::Assign(..))
        } else if let Some(var_definition) = context.var_definition {
            if let Some(init) = &var_definition.initializer {
                matches!(init.kind, ExprKind::Lit(..))
            } else {
                false
            }
        } else {
            false
        }
    }
}

/// Starting from a solar Expr, creates an AssignVarTypes enum (used for mutation)
fn determinate_type(context: &MutationContext<'_>) -> Option<AssignVarTypes> {
    let expr = if let Some(var_definition) = context.var_definition {
        var_definition.initializer.as_ref().unwrap()
    } else {
        context.expr.unwrap()
    };

    match &expr.kind {
        ExprKind::Lit(kind, _) => Some(AssignVarTypes::Literal(kind.kind.clone())),
        ExprKind::Ident(val) => Some(AssignVarTypes::Identifier(val.to_string())),
        _ => None,
    }
}
