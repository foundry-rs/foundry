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
        _: bool,
    ) -> InstructionResult {
        self.maps.entry(b256_to_h256(interpreter.contract.bytecode.hash())).or_insert_with(|| {
            HitMap::new(Bytes::copy_from_slice(
                interpreter.contract.bytecode.original_bytecode_slice(),
            ))
        });

        Return::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> InstructionResult {
        self.maps
            .entry(b256_to_h256(interpreter.contract.bytecode.hash()))
            .and_modify(|map| map.hit(interpreter.program_counter()));

        Return::Continue
    }
}
