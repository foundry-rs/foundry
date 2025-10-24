pub mod assignment_mutator;
pub mod binary_op_mutator;
pub mod delete_expression_mutator;
pub mod elim_delegate_mutator;
pub mod unary_op_mutator;

pub mod mutator_registry;

use eyre::Result;
use solar_parse::ast::{Expr, Span, VariableDefinition};
use std::path::PathBuf;

use crate::mutation::Mutant;

pub trait Mutator: Send + Sync {
    /// Generate all mutant corresponding to a given context
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>>;
    /// True if a mutator can be applied to an expression/node
    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool;
}

#[derive(Debug)]
pub struct MutationContext<'a> {
    pub path: PathBuf,
    pub span: Span,
    /// The expression to mutate
    pub expr: Option<&'a Expr<'a>>,

    pub var_definition: Option<&'a VariableDefinition<'a>>,
}

impl<'a> MutationContext<'a> {
    pub fn builder() -> MutationContextBuilder<'a> {
        MutationContextBuilder::new()
    }
}

pub struct MutationContextBuilder<'a> {
    path: Option<PathBuf>,
    span: Option<Span>,
    expr: Option<&'a Expr<'a>>,
    var_definition: Option<&'a VariableDefinition<'a>>,
}

impl<'a> MutationContextBuilder<'a> {
    // Create a new empty builder
    pub fn new() -> Self {
        MutationContextBuilder { path: None, span: None, expr: None, var_definition: None }
    }

    // Required
    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    // Required
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    // Optional
    pub fn with_expr(mut self, expr: &'a Expr<'a>) -> Self {
        self.expr = Some(expr);
        self
    }

    // Optional
    pub fn with_var_definition(mut self, var_definition: &'a VariableDefinition<'a>) -> Self {
        self.var_definition = Some(var_definition);
        self
    }

    pub fn build(self) -> Result<MutationContext<'a>, &'static str> {
        let span = self.span.ok_or("Span is required for MutationContext")?;
        let path = self.path.ok_or("Path is required for MutationContext")?;

        Ok(MutationContext { path, span, expr: self.expr, var_definition: self.var_definition })
    }
}

#[cfg(test)]
mod tests;
