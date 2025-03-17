use super::{MutationContext, Mutator};
use crate::mutation::mutant::Mutant;
use eyre::{Context, Result};

use crate::mutation::mutators::assignement_mutator::AssignmentMutator;

/// Registry of all available mutators (ie implementing the Mutator trait)
pub struct MutatorRegistry {
    mutators: Vec<Box<dyn Mutator>>,
}

impl MutatorRegistry {
    pub fn new() -> Self {
        let mut registry = MutatorRegistry { mutators: Vec::new() };

        registry
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
