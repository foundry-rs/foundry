use crate::mutation::mutators::Mutator;
use solar_parse::ast::{visit::Visit, Expr, LitKind, VariableDefinition};
use std::ops::ControlFlow;

use crate::mutation::{
    mutant::Mutant,
    mutators::{mutator_registry::MutatorRegistry, MutationContext},
};

#[derive(Debug, Clone)]
pub enum AssignVarTypes {
    Literal(LitKind),
    Identifier(String), /* not using Ident as the symbol is slow to convert as to_str() <--
                         * maybe will have to switch back if validating more aggressively */
}

/// A wrapper around the Solar macro-generated visitor, in order to use the default implementation
/// of the fn we override in MutantVisitor
pub struct SolarVisitorWrapper {}

impl Visit<'_> for SolarVisitorWrapper {
    type BreakValue = ();
}

/// A visitor which collect all expression to mutate as well as the mutation types
pub struct MutantVisitor {
    pub mutation_to_conduct: Vec<Mutant>,
    pub mutator_registry: MutatorRegistry,
    default_visitor: SolarVisitorWrapper,
}

impl MutantVisitor {
    /// Use all mutator from registry::default
    pub fn default() -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            mutator_registry: MutatorRegistry::default(),
            default_visitor: SolarVisitorWrapper {},
        }
    }

    /// Use only a set of mutators
    pub fn new_with_mutators(mutators: Vec<Box<dyn Mutator>>) -> Self {
        Self {
            mutation_to_conduct: Vec::new(),
            mutator_registry: MutatorRegistry::new_with_mutators(mutators),
            default_visitor: SolarVisitorWrapper {},
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
            .with_span(var.span)
            .with_var_definition(var)
            .build()
            .unwrap();

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));

        self.default_visitor.visit_variable_definition(var)
    }

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        let context =
            MutationContext::builder().with_span(expr.span).with_expr(expr).build().unwrap();

        self.mutation_to_conduct.extend(self.mutator_registry.generate_mutations(&context));

        self.default_visitor.visit_expr(expr)
    }
}
