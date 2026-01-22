use crate::mutation::mutant::OwnedLiteral;
#[cfg(test)]
use crate::mutation::mutators::Mutator;
use solar::ast::{Expr, Span, VariableDefinition, visit::Visit};
use std::{ops::ControlFlow, path::PathBuf};

use crate::mutation::{
    mutant::Mutant,
    mutators::{MutationContext, mutator_registry::MutatorRegistry},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AssignVarTypes {
    Literal(OwnedLiteral),
    Identifier(String), /* not using Ident as the symbol is slow to convert as to_str() <--
                         * maybe will have to switch back if validating more aggressively */
}

/// A visitor which collect all expression to mutate as well as the mutation types
pub struct MutantVisitor<'src> {
    pub mutation_to_conduct: Vec<Mutant>,
    pub mutator_registry: MutatorRegistry,
    pub path: PathBuf,
    pub span_filter: Option<Box<dyn Fn(Span) -> bool>>,
    pub skipped_count: usize,
    /// Source code for extracting original text
    pub source: Option<&'src str>,
}

impl<'src> MutantVisitor<'src> {
    /// Use all mutator from registry::default
    pub fn default(path: PathBuf) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            mutator_registry: MutatorRegistry::default(),
            path,
            span_filter: None,
            skipped_count: 0,
            source: None,
        }
    }

    /// Use only a set of mutators
    #[cfg(test)]
    pub fn new_with_mutators(path: PathBuf, mutators: Vec<Box<dyn Mutator>>) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            mutator_registry: MutatorRegistry::new_with_mutators(mutators),
            path,
            span_filter: None,
            skipped_count: 0,
            source: None,
        }
    }

    /// Set a filter function to skip certain spans (for adaptive mutation testing)
    pub fn with_span_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(Span) -> bool + 'static,
    {
        self.span_filter = Some(Box::new(filter));
        self
    }

    /// Set the source code for extracting original text
    pub fn with_source(mut self, source: &'src str) -> Self {
        self.source = Some(source);
        self
    }
}

impl<'ast> Visit<'ast> for MutantVisitor<'ast> {
    type BreakValue = ();

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Check if we should skip this span (adaptive mutation testing)
        if let Some(ref filter) = self.span_filter
            && filter(var.span)
        {
            self.skipped_count += 1;
            return self.walk_variable_definition(var);
        }

        let mut builder = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(var.span)
            .with_var_definition(var);

        if let Some(src) = self.source {
            builder = builder.with_source(src);
        }

        let context = builder.build().unwrap();

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_variable_definition(var)
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        // Check if we should skip this span (adaptive mutation testing)
        if let Some(ref filter) = self.span_filter
            && filter(expr.span)
        {
            self.skipped_count += 1;
            return self.walk_expr(expr);
        }

        let mut builder = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(expr.span)
            .with_expr(expr);

        if let Some(src) = self.source {
            builder = builder.with_source(src);
        }

        let context = builder.build().unwrap();

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_expr(expr)
    }
}
