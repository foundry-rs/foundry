use crate::mutation::mutation::{Mutator, MutationContext, Mutant};

pub struct BinaryOpMutator;

impl Mutator for BinaryOpMutator {
    fn generate_mutants(&self, ctxt: &MutationContext<'_>) -> Vec<Mutant> {
        todo!()
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        todo!()
    }

    fn name(&self) ->  &'static str {
        todo!()
    }
}