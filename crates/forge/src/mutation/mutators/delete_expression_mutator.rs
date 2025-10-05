use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};
use solar_parse::ast::ExprKind;

use eyre::Result;

pub struct DeleteExpressionMutator;

impl Mutator for DeleteExpressionMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        Ok(vec![Mutant {
            span: ctxt.span,
            mutation: MutationType::DeleteExpression,
            path: ctxt.path.clone(),
        }])
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        if let Some(expr) = ctxt.expr { matches!(expr.kind, ExprKind::Delete(_)) } else { false }
    }
}
