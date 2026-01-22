use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};

use eyre::Result;
use solar::ast::ExprKind;
use std::fmt::Display;

pub struct ElimDelegateMutator;

impl Mutator for ElimDelegateMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        Ok(vec![Mutant {
            span: context.span,
            mutation: MutationType::ElimDelegate,
            path: context.path.clone(),
            original: context.original_text(),
            source_line: context.source_line(),
            line_number: context.line_number(),
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
            .is_some_and(|ident| ident.to_string() == "delegatecall")
    }
}

impl Display for ElimDelegateMutator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "")
    }
}
