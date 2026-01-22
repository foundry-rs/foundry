pub mod assignment_mutator;
pub mod binary_op_mutator;
pub mod delete_expression_mutator;
pub mod elim_delegate_mutator;
pub mod unary_op_mutator;

pub mod mutator_registry;

use eyre::Result;
use solar::ast::{Expr, Span, VariableDefinition};
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

    /// The full source code (used to extract original text for mutations)
    pub source: Option<&'a str>,
}

impl MutationContext<'_> {
    /// Extract the original source text covered by this context's span
    pub fn original_text(&self) -> String {
        self.source
            .and_then(|src| {
                let lo = self.span.lo().0 as usize;
                let hi = self.span.hi().0 as usize;
                src.get(lo..hi).map(|s| s.to_string())
            })
            .unwrap_or_default()
    }

    /// Get the line number (1-indexed) for this context's span
    pub fn line_number(&self) -> usize {
        self.source
            .map(|src| {
                let pos = self.span.lo().0 as usize;
                src.get(..pos).map(|s| s.lines().count()).unwrap_or(0).max(1)
            })
            .unwrap_or(1)
    }

    /// Get the full source line containing this span
    pub fn source_line(&self) -> String {
        self.source
            .and_then(|src| {
                let pos = self.span.lo().0 as usize;
                // Find line start
                let line_start = src[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                // Find line end
                let line_end = src[pos..].find('\n').map(|i| pos + i).unwrap_or(src.len());
                src.get(line_start..line_end).map(|s| s.trim().to_string())
            })
            .unwrap_or_default()
    }
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
    source: Option<&'a str>,
}

impl<'a> MutationContextBuilder<'a> {
    // Create a new empty builder
    pub fn new() -> Self {
        MutationContextBuilder {
            path: None,
            span: None,
            expr: None,
            var_definition: None,
            source: None,
        }
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

    // Optional - provide source code for extracting original text
    pub fn with_source(mut self, source: &'a str) -> Self {
        self.source = Some(source);
        self
    }

    pub fn build(self) -> Result<MutationContext<'a>, &'static str> {
        let span = self.span.ok_or("Span is required for MutationContext")?;
        let path = self.path.ok_or("Path is required for MutationContext")?;

        Ok(MutationContext {
            path,
            span,
            expr: self.expr,
            var_definition: self.var_definition,
            source: self.source,
        })
    }
}

#[cfg(test)]
mod tests;
