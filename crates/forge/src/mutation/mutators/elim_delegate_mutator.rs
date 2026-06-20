use std::fmt::Display;

use eyre::Result;
use solar::ast::ExprKind;

use super::{MutationContext, Mutator};

use crate::mutation::mutant::{Mutant, MutationType};

pub struct ElimDelegateMutator;

impl Mutator for ElimDelegateMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        // Narrow the span to just the `delegatecall` identifier so the replacement
        // text ("call") does not clobber the surrounding call expression
        // (e.g. `target.delegatecall(data)`).
        let ident_span = context
            .expr
            .as_ref()
            .and_then(|expr| match &expr.kind {
                ExprKind::Call(callee, _) => Some(callee),
                _ => None,
            })
            .and_then(|callee| match &callee.kind {
                ExprKind::Member(_, ident) => Some(ident.span),
                _ => None,
            })
            .unwrap_or(context.span);

        Ok(vec![Mutant {
            span: ident_span,
            mutation: MutationType::ElimDelegate,
            path: context.path.clone(),
            // Use the narrowed identifier as the "original" text so the diff line
            // ("- delegatecall" / "+ call") matches the actual textual replacement.
            original: "delegatecall".to_string(),
            source_line: context.source_line(),
            line_number: context.line_number(),
            column_number: context.column_number(),
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
