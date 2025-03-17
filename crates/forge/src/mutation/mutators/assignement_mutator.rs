use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{MutationContext, Mutator},
    visitor::AssignVarTypes,
};

use eyre::{Context, Result};
use solar_parse::ast::{BinOpKind, Expr, ExprKind, LitKind, Span, UnOpKind};
use std::path::PathBuf;

pub struct AssignmentMutator;

impl Mutator for AssignmentMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        let assign_type = determinate_type(context.expr.unwrap())
            .context("AssignementMutator: unexpected expression kind");

        match assign_type.unwrap() {
            AssignVarTypes::Literal(lit) => match lit {
                LitKind::Bool(val) => Ok(vec![Mutant {
                    span: context.span,
                    mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                        LitKind::Bool(!val),
                    )),
                    path: PathBuf::default(),
                }]),
                LitKind::Number(val) => Ok(vec![
                    Mutant {
                        span: context.span,
                        mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                            LitKind::Number(num_bigint::BigInt::ZERO),
                        )),
                        path: PathBuf::default(),
                    },
                    Mutant {
                        span: context.span,
                        mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                            LitKind::Number(-val),
                        )),
                        path: PathBuf::default(),
                    },
                ]),
                _ => {
                    eyre::bail!("AssignementMutator: unexpected literal kind: {:?}", lit)
                }
            },
            AssignVarTypes::Identifier(ident) => {
                let inner = ident.to_string();

                Ok(vec![
                    Mutant {
                        span: context.span,
                        mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                            LitKind::Number(num_bigint::BigInt::ZERO),
                        )),
                        path: PathBuf::default(),
                    },
                    Mutant {
                        span: context.span,
                        mutation: MutationType::AssignmentMutation(AssignVarTypes::Identifier(
                            format!("-{}", inner),
                        )),
                        path: PathBuf::default(),
                    },
                ])
            }
        }
    }

    fn name(&self) -> &'static str {
        "AssignmentMutator"
    }

    fn is_applicable(&self, context: &MutationContext<'_>) -> bool {
        if let Some(expr) = context.expr {
            matches!(expr.kind, ExprKind::Assign(..))
        } else {
            false
        }
    }
}

fn determinate_type(expr: &Expr<'_>) -> Result<AssignVarTypes> {
    match &expr.kind {
        ExprKind::Lit(kind, _) => Ok(AssignVarTypes::Literal(kind.kind.clone())),
        ExprKind::Ident(val) => Ok(AssignVarTypes::Identifier(val.to_string())),
        _ => {
            eyre::bail!("AssignementMutator: unexpected expression kind: {:?}", expr.kind)
        }
    }
}
