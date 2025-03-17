use super::{MutationContext, Mutator};
use crate::mutation::mutant::{Mutant, MutationType};
use eyre::{Context, Result};

pub struct UnaryOperatorMutator;

impl Mutator for UnaryOperatorMutator {
    fn generate_mutants(&self, context: &MutationContext<'_>) -> Result<Vec<Mutant>> {
        todo!()
    }

    fn is_applicable(&self, ctxt: &MutationContext<'_>) -> bool {
        todo!()
    }

    fn name(&self) -> &'static str {
        todo!()
    }
}
