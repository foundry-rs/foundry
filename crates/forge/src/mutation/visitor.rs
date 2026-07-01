use std::{ops::ControlFlow, path::PathBuf};

use eyre::Report;
use solar::ast::{Expr, ItemContract, VariableDefinition, visit::Visit, yul};

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
#[allow(clippy::type_complexity)]
pub struct MutantVisitor<'src> {
    pub mutation_to_conduct: Vec<Mutant>,
    errors: Vec<Report>,
    pub mutator_registry: MutatorRegistry,
    pub path: PathBuf,
    pub source: Option<&'src str>,
    /// Optional per-contract name filter. When `Some`, mutations are only collected
    /// from contracts whose name matches the predicate.
    pub contract_filter: Option<Box<dyn Fn(&str) -> bool>>,
    /// Whether the currently-visited contract is allowed by `contract_filter`.
    /// `true` when no filter is set or when we are visiting a contract whose name
    /// matched the filter. Top-level items (outside any contract) are always
    /// considered "allowed".
    in_allowed_contract: bool,
}

impl<'src> MutantVisitor<'src> {
    /// Create a visitor with the specified mutator operators enabled
    pub fn with_operators(path: PathBuf, operators: &[MutatorType]) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            errors: Vec::new(),
            mutator_registry: MutatorRegistry::from_enabled(operators),
            path,
            source: None,
            contract_filter: None,
            in_allowed_contract: true,
        }
    }

    /// Use all mutators from registry (all operators enabled)
    #[cfg(test)]
    pub fn default(path: PathBuf) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            errors: Vec::new(),
            mutator_registry: MutatorRegistry::default(),
            path,
            source: None,
            contract_filter: None,
            in_allowed_contract: true,
        }
    }

    /// Use only a set of mutators
    #[cfg(test)]
    pub fn new_with_mutators(path: PathBuf, mutators: Vec<Box<dyn Mutator>>) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            errors: Vec::new(),
            mutator_registry: MutatorRegistry::new_with_mutators(mutators),
            path,
            source: None,
            contract_filter: None,
            in_allowed_contract: true,
        }
    }

    /// Set the source code for extracting original text
    pub const fn with_source(mut self, source: &'src str) -> Self {
        self.source = Some(source);
        self
    }

    /// Set a contract-name filter; only contracts whose name matches the
    /// predicate will have their bodies mutated.
    pub fn with_contract_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&str) -> bool + 'static,
    {
        self.contract_filter = Some(Box::new(filter));
        self
    }

    pub fn take_errors(&mut self) -> Vec<Report> {
        std::mem::take(&mut self.errors)
    }

    fn collect_mutations(&mut self, context: &MutationContext<'_>) {
        let result = self.mutator_registry.generate_mutations(context);
        self.mutation_to_conduct.extend(result.mutations);

        for err in result.errors {
            self.errors.push(err.wrap_err(format!(
                "failed to generate mutations for {}:{}:{}",
                self.path.display(),
                context.line_number(),
                context.column_number()
            )));
        }
    }
}

impl<'ast> Visit<'ast> for MutantVisitor<'ast> {
    type BreakValue = ();

    fn visit_item_contract(
        &mut self,
        contract: &'ast ItemContract<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // When a contract name filter is configured, only descend into matching
        // contracts. We toggle `in_allowed_contract` for the duration of the
        // walk so nested visit_expr / visit_variable_definition calls can gate
        // mutant collection accordingly.
        let prev = self.in_allowed_contract;
        self.in_allowed_contract = match &self.contract_filter {
            Some(filter) => filter(contract.name.as_str()),
            None => true,
        };
        let res = self.walk_item_contract(contract);
        self.in_allowed_contract = prev;
        res
    }

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        // Skip entirely when the surrounding contract is filtered out.
        if !self.in_allowed_contract {
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

        self.collect_mutations(&context);
        self.walk_variable_definition(var)
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        // Skip entirely when the surrounding contract is filtered out.
        if !self.in_allowed_contract {
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

        self.collect_mutations(&context);
        self.walk_expr(expr)
    }

    fn visit_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        // Skip entirely when the surrounding contract is filtered out.
        if !self.in_allowed_contract {
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

        self.collect_mutations(&context);
        self.walk_yul_expr(expr)
    }
}

#[cfg(test)]
mod tests {
    use eyre::{Result, eyre};
    use solar::{
        ast::{Arena, interface::source_map::FileName},
        parse::Parser,
    };

    use super::*;
    use crate::mutation::{Session, mutant::MutationType};

    struct FailingExprMutator;

    impl Mutator for FailingExprMutator {
        fn generate_mutants(&self, _ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
            Err(eyre!("synthetic visitor failure"))
        }

        fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
            ctxt.expr.is_some()
        }
    }

    struct PassingExprMutator;

    impl Mutator for PassingExprMutator {
        fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
            Ok(vec![Mutant {
                path: ctxt.path.clone(),
                span: ctxt.span,
                mutation: MutationType::DeleteExpression,
                original: ctxt.original_text(),
                source_line: ctxt.source_line(),
                line_number: ctxt.line_number(),
                column_number: ctxt.column_number(),
            }])
        }

        fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
            ctxt.expr.is_some()
        }
    }

    #[test]
    fn visitor_collects_mutations_and_surfaces_mutator_errors() {
        let source = "\
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Test {
    function test() public {
        uint256 x = 1 + 2;
    }
}
";
        let path = PathBuf::from("test.sol");
        let sess = Session::builder().with_silent_emitter(None).build();

        sess.enter(|| {
            let arena = Arena::new();
            let mut parser =
                Parser::from_lazy_source_code(&sess, &arena, FileName::Real(path.clone()), || {
                    Ok(source.to_string())
                })
                .unwrap();
            let ast = parser.parse_file().map_err(|e| e.emit()).unwrap();
            drop(parser);
            let mut visitor = MutantVisitor::new_with_mutators(
                path,
                vec![Box::new(FailingExprMutator), Box::new(PassingExprMutator)],
            )
            .with_source(source);

            let _ = visitor.visit_source_unit(&ast);
            let errors = visitor.take_errors();

            assert!(!visitor.mutation_to_conduct.is_empty());
            assert!(!errors.is_empty());

            let err = format!("{:?}", errors[0]);
            assert!(err.contains("failed to generate mutations for test.sol:"));
            assert!(err.contains("synthetic visitor failure"));
        });
    }
}
