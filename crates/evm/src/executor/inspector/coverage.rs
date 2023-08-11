use crate::{
    coverage::{HitMap, HitMaps},
    utils::b256_to_h256,
};
use bytes::Bytes;
use revm::{
    interpreter::{InstructionResult, Interpreter},
    Database, EVMData, Inspector,
};

#[derive(Default, Debug)]
pub struct CoverageCollector {
    /// Maps that track instruction hit data.
    pub maps: HitMaps,
}

impl<DB> Inspector<DB> for CoverageCollector
where
    DB: Database,
{
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
    ) -> InstructionResult {
        let hash = b256_to_h256(interpreter.contract.bytecode.clone().unlock().hash_slow());
        self.maps.entry(hash).or_insert_with(|| {
            HitMap::new(Bytes::copy_from_slice(
                interpreter.contract.bytecode.original_bytecode_slice(),
            ))
        });

        InstructionResult::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
    ) -> InstructionResult {
        let hash = b256_to_h256(interpreter.contract.bytecode.clone().unlock().hash_slow());
        self.maps.entry(hash).and_modify(|map| map.hit(interpreter.program_counter()));

        InstructionResult::Continue
    }
}
