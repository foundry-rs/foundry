use crate::mutation::mutation::{Mutant, MutationType};
use super::{MutationContext, Mutator};
pub struct IdentifierMutator;

impl Mutator for IdentifierMutator {
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