use crate::mutation::mutant::Mutant;
use foundry_config::MutatorType;

use super::{
    MutationContext, Mutator, assembly_mutator, assignment_mutator, binary_op_mutator,
    brutalizer_mutator, delete_expression_mutator, elim_delegate_mutator, require_mutator,
    unary_op_mutator,
};

/// Registry of all available mutators (ie implementing the Mutator trait)
pub struct MutatorRegistry {
    mutators: Vec<Box<dyn Mutator>>,
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
                MutatorType::Brutalizer => {
                    registry.mutators.push(Box::new(brutalizer_mutator::BrutalizerMutator));
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
    pub fn generate_mutations(&self, context: &MutationContext<'_>) -> Vec<Mutant> {
        self.mutators
            .iter()
            .filter(|mutator| mutator.is_applicable(context))
            .filter_map(|mutator| mutator.generate_mutants(context).ok())
            .flatten()
            .collect()
    }
}
