use crate::coverage::{HitMap, HitMaps};
use bytes::Bytes;
use revm::{Database, EVMData, Inspector};
use revm::interpreter::{InstructionResult, Interpreter};

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
        self.maps.entry(interpreter.contract.bytecode.hash().into()).or_insert_with(|| {
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
        _is_static: bool,
    ) -> InstructionResult {
        self.maps
            .entry(interpreter.contract.bytecode.hash().into())
            .and_modify(|map| map.hit(interpreter.program_counter()));

        InstructionResult::Continue
    }
}
