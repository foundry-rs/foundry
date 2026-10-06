use crate::mutation::mutant::Mutant;
use eyre::Report;
use foundry_config::MutatorType;

use super::{
    MutationContext, Mutator, assembly_mutator, assignment_mutator, binary_op_mutator,
    delete_expression_mutator, elim_delegate_mutator, require_mutator, unary_op_mutator,
};

/// Registry of all available mutators (ie implementing the Mutator trait)
pub struct MutatorRegistry {
    mutators: Vec<Box<dyn Mutator>>,
}

pub struct MutationGenerationResult {
    pub mutations: Vec<Mutant>,
    pub errors: Vec<Report>,
}

impl MutatorRegistry {
    #[cfg(test)]
    pub fn default() -> Self {
        Self::from_enabled(&MutatorType::all())
    }

    pub fn from_enabled(enabled: &[MutatorType]) -> Self {
        let mut registry = Self { mutators: Vec::new() };

        for ty in enabled {
            match ty {
                MutatorType::Assembly => {
                    registry.mutators.push(Box::new(assembly_mutator::AssemblyMutator::new()));
                }
                MutatorType::Assignment => {
                    registry.mutators.push(Box::new(assignment_mutator::AssignmentMutator));
                }
                MutatorType::BinaryOp => {
                    registry.mutators.push(Box::new(binary_op_mutator::BinaryOpMutator));
                }
                MutatorType::DeleteExpression => {
                    registry
                        .mutators
                        .push(Box::new(delete_expression_mutator::DeleteExpressionMutator));
                }
                MutatorType::ElimDelegate => {
                    registry.mutators.push(Box::new(elim_delegate_mutator::ElimDelegateMutator));
                }
                MutatorType::Require => {
                    registry.mutators.push(Box::new(require_mutator::RequireMutator));
                }
                MutatorType::UnaryOp => {
                    registry.mutators.push(Box::new(unary_op_mutator::UnaryOpMutator));
                }
            }
        }

        registry
    }

    #[cfg(test)]
    pub fn new_with_mutators(mutators: Vec<Box<dyn Mutator>>) -> Self {
        Self { mutators }
    }

    /// Find all applicable mutators for a given context and return the corresponding mutations
    /// and any mutator errors encountered while generating them.
    pub fn generate_mutations(&self, context: &MutationContext<'_>) -> MutationGenerationResult {
        let mut mutations = Vec::new();
        let mut errors = Vec::new();
        for mutator in self.mutators.iter().filter(|mutator| mutator.is_applicable(context)) {
            match mutator.generate_mutants(context) {
                Ok(generated) => mutations.extend(generated),
                Err(err) => errors.push(err),
            }
        }
        MutationGenerationResult { mutations, errors }
    }
}

#[cfg(test)]
mod tests {
    use eyre::{Result, eyre};
    use solar::ast::Span;

    use super::*;

    struct FailingMutator;

    impl Mutator for FailingMutator {
        fn generate_mutants(&self, _ctxt: &MutationContext<'_>) -> Result<Vec<Mutant>> {
            Err(eyre!("synthetic mutator failure"))
        }

        fn is_applicable(&self, _ctxt: &MutationContext<'_>) -> bool {
            true
        }
    }

    #[test]
    fn generate_mutations_collects_mutator_errors() {
        let registry = MutatorRegistry::new_with_mutators(vec![Box::new(FailingMutator)]);
        let context = MutationContext::builder()
            .with_path("test.sol".into())
            .with_span(Span::DUMMY)
            .build()
            .unwrap();

        let result = registry.generate_mutations(&context);
        assert!(result.mutations.is_empty());
        let err = result.errors.into_iter().next().unwrap();
        assert!(err.to_string().contains("synthetic mutator failure"));
    }
}
