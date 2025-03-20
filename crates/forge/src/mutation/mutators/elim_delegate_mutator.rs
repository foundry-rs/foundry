use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

use eyre::{Context, Result};
use solar_parse::ast::{BinOpKind, Expr, ExprKind, LitKind, Span, UnOpKind};
use std::path::PathBuf;

pub struct ElimDelegateMutator;

impl Mutator for ElimDelegateMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        Ok(vec![Mutant {
            span: context.span,
            mutation: MutationType::ElimDelegateMutation,
            path: PathBuf::default(),
        }])
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        ctxt.expr
            .as_ref()
            .and_then(|expr| match &expr.kind {
                ExprKind::Call(callee, _) => Some(callee),
                _ => None,
            })
            .and_then(|callee| match &callee.kind {
                ExprKind::Member(_, ident) => Some(ident),
                _ => None,
            })
            .map_or(false, |ident| ident.to_string() == "delegatecall")
    }

    fn name(&self) -> &'static str {
        "ElimDelegateMutation"
    }
}

impl ToString for ElimDelegateMutator {
    fn to_string(&self) -> String {
        "".to_string()
    }
}
