pub mod assignement_mutator;
pub mod binary_op_mutator;
pub mod delete_expression_mutator;
pub mod elim_delegate_mutator;
pub mod ident_mutator;
pub mod unary_op_mutator;

pub mod mutator_registry;

use solar_parse::ast::{Expr, Span, UnOpKind};

use crate::mutation::Mutant;

pub trait Mutator {
    /// Generate all mutant corresponding to a given context
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Vec<Mutant>;
    /// True if a mutator can be applied to an expression/node
    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool;
    fn name(&self) -> &'static str;
}

pub struct MutationContext<'a> {
    pub span: Span,
    /// The expression to mutate
    pub expr: Option<&'a Expr<'a>>,
    /// The operation (in unary or binary-op mutations)
    pub op_kind: Option<UnOpKind>, 
}