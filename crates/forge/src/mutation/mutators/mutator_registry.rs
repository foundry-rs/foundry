use crate::mutation::mutant::Mutant;

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
    pub fn default() -> Self {
        let mut registry = Self { mutators: Vec::new() };

        registry.mutators.push(Box::new(assembly_mutator::AssemblyMutator::new()));
        registry.mutators.push(Box::new(assignment_mutator::AssignmentMutator));
        registry.mutators.push(Box::new(binary_op_mutator::BinaryOpMutator));
        registry.mutators.push(Box::new(brutalizer_mutator::BrutalizerMutator));
        registry.mutators.push(Box::new(delete_expression_mutator::DeleteExpressionMutator));
        registry.mutators.push(Box::new(elim_delegate_mutator::ElimDelegateMutator));
        registry.mutators.push(Box::new(require_mutator::RequireMutator));
        registry.mutators.push(Box::new(unary_op_mutator::UnaryOperatorMutator));

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
