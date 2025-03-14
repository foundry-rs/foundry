
use crate::mutation::mutation::{Mutator, MutationContext, Mutant, MutationType};
use crate::mutation::visitor::AssignVarTypes;

use solar_parse::ast::{BinOpKind, Expr, ExprKind, LitKind, Span, UnOpKind};
use std::path::PathBuf;


pub struct AssignmentMutator;

impl Mutator for AssignmentMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Vec<Mutant> {
        todo!()
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
