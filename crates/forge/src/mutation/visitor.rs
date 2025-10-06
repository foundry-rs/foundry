use crate::mutation::{mutant::OwnedLiteral, mutators::Mutator};
use solar_parse::ast::{Expr, VariableDefinition, visit::Visit};
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
pub struct MutantVisitor {
    pub mutation_to_conduct: Vec<Mutant>,
    pub mutator_registry: MutatorRegistry,
    pub path: PathBuf,
}

impl MutantVisitor {
    /// Use all mutator from registry::default
    pub fn default(path: PathBuf) -> Self {
        Self { mutation_to_conduct: Vec::new(), mutator_registry: MutatorRegistry::default(), path }
    }

    /// Use only a set of mutators
    pub fn new_with_mutators(path: PathBuf, mutators: Vec<Box<dyn Mutator>>) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            mutator_registry: MutatorRegistry::new_with_mutators(mutators),
            path,
        }
    }
}

impl<'ast> Visit<'ast> for MutantVisitor {
    type BreakValue = ();

    fn visit_variable_definition(
        &mut self,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        let context = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(var.span)
            .with_var_definition(var)
            .build()
            .unwrap();

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_variable_definition(var)
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        let context = MutationContext::builder()
            .with_path(self.path.clone())
            .with_span(expr.span)
            .with_expr(expr)
            .build()
            .unwrap();

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));
        self.walk_expr(expr)
    }
}
