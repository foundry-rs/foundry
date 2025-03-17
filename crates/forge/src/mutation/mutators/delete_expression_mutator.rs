use crate::mutation::mutant::{Mutant, MutationType};
use super::{MutationContext, Mutator};
use solar_parse::ast::{BinOpKind, Expr, ExprKind, LitKind, Span, UnOpKind};

use std::path::PathBuf;
use eyre::{Context, Result};


pub struct DeleteExpressionMutator;

impl Mutator for DeleteExpressionMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        Ok(vec![ Mutant { span: ctxt.span, mutation: MutationType::DeleteExpressionMutation, path: PathBuf::default() }])
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(expr) = ctxt.expr {
            matches!(expr.kind, ExprKind::Delete(_))
        } else { false }
    }

    fn name(&self) ->  &'static str {
        "DeleteExpression"
    }
}