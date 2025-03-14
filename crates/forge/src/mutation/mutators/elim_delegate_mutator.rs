
use crate::mutation::mutation::{Mutant, MutationType};
use super::{MutationContext, Mutator};

use solar_parse::ast::{BinOpKind, Expr, ExprKind, LitKind, Span, UnOpKind};
use std::path::PathBuf;

pub struct ElimDelegateMutator;

impl Mutator for ElimDelegateMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Vec<Mutant> {
        vec![
            Mutant { span: ctxt.span, mutation: MutationType::ElimDelegateMutation, path: PathBuf::default() }
        ]
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(expr) = ctxt.expr {
            if let ExprKind::Call(expr, _) = &expr.kind {
                if let ExprKind::Member(_, ident) = expr.kind {
                    return ident.to_string() == "delegatecall";
                }
            }
        }

        return false;
    }

    fn name(&self) ->  &'static str {
        "ElimDelegateMutation"
    }
}