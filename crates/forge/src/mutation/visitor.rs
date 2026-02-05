use std::{ops::ControlFlow, path::PathBuf};

use solar::ast::{
    Block, Expr, ItemFunction, Span, StmtKind, VariableDefinition, visit::Visit, yul,
};

#[cfg(test)]
use crate::mutation::mutators::Mutator;
use crate::mutation::{
    mutant::{Mutant, OwnedLiteral},
    mutators::{MutationContext, mutator_registry::MutatorRegistry},
};
use foundry_config::MutatorType;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AssignVarTypes {
    Literal(OwnedLiteral),
    Identifier(String),
}

/// A visitor which collect all expression to mutate as well as the mutation types
pub struct MutantVisitor<'src> {
    pub mutation_to_conduct: Vec<Mutant>,
    pub mutator_registry: MutatorRegistry,
    pub path: PathBuf,
    pub span_filter: Option<Box<dyn Fn(Span) -> bool>>,
    pub skipped_count: usize,
    pub source: Option<&'src str>,
}

impl<'src> MutantVisitor<'src> {
    /// Create a visitor with the specified mutator operators enabled
    pub fn with_operators(path: PathBuf, operators: &[MutatorType]) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            mutator_registry: MutatorRegistry::from_enabled(operators),
            path,
            span_filter: None,
            skipped_count: 0,
            source: None,
        }
    }

    /// Use all mutators from registry (all operators enabled)
    #[cfg(test)]
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

        let context = builder
            .build()
            .expect("MutationContext requires both path and span for variable definition");

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_variable_definition(var)
    }

    fn visit_item_function(
        &mut self,
        func: &'ast ItemFunction<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        if let Some(ref body) = func.body {
            let body_span = body.span;

            if let Some(ref filter) = self.span_filter
                && filter(body_span)
            {
                self.skipped_count += 1;
                return self.walk_item_function(func);
            }

            let mut builder = MutationContext::builder()
                .with_path(self.path.clone())
                .with_span(body_span)
                .with_fn_body_span(body_span)
                .with_fn_kind(func.kind)
                .with_fn_has_assembly(block_contains_assembly(body));

            if let Some(vis) = func.header.visibility() {
                builder = builder.with_fn_visibility(vis);
            }

            if let Some(src) = self.source {
                builder = builder.with_source(src);
            }

            let context = builder.build().unwrap();
            self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        }

        self.walk_item_function(func)
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

        let context =
            builder.build().expect("MutationContext requires both path and span for expression");

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_expr(expr)
    }

    fn visit_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let Some(ref filter) = self.span_filter
            && filter(expr.span)
        {
            self.skipped_count += 1;
            return self.walk_yul_expr(expr);
        }

        let mut builder = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(expr.span)
            .with_yul_expr(expr);

        if let Some(src) = self.source {
            builder = builder.with_source(src);
        }

        let context = builder
            .build()
            .expect("MutationContext requires both path and span for yul expression");

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_yul_expr(expr)
    }
}

fn block_contains_assembly(block: &Block<'_>) -> bool {
    block.stmts.iter().any(|stmt| stmt_contains_assembly(&stmt.kind))
}

fn stmt_contains_assembly(kind: &StmtKind<'_>) -> bool {
    match kind {
        StmtKind::Assembly(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => block_contains_assembly(block),
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_contains_assembly(&then_stmt.kind)
                || else_stmt.as_ref().is_some_and(|s| stmt_contains_assembly(&s.kind))
        }
        StmtKind::While(_, body) | StmtKind::DoWhile(body, _) => {
            stmt_contains_assembly(&body.kind)
        }
        StmtKind::For { body, .. } => stmt_contains_assembly(&body.kind),
        StmtKind::Try(try_stmt) => try_stmt
            .clauses
            .iter()
            .any(|clause| block_contains_assembly(&clause.block)),
        _ => false,
    }
}
