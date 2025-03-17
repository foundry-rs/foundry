use crate::mutation::mutant::{Mutant, MutationType};
use super::{MutationContext, Mutator};
use eyre::{Context, Result};

pub struct IdentifierMutator;

impl Mutator for IdentifierMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        todo!()
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        todo!()
    }

    fn name(&self) ->  &'static str {
        todo!()
    }
}