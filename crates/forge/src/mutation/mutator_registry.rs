use crate::mutation::mutation::{Mutator, MutationContext, Mutant};

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
        self.mutators.iter()
            .filter(|mutator| mutator.is_applicable(context))
            .flat_map(|mutator| mutator.generate_mutants(context))
            .collect()
    }
}