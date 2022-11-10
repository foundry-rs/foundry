use crate::coverage::{HitMap, HitMaps};
use bytes::Bytes;
use revm::{Database, EVMData, Inspector, Interpreter, Return};

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
    ) -> Return {
        self.maps.entry(interpreter.contract.bytecode.hash()).or_insert_with(|| {
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
    ) -> Return {
        self.maps
            .entry(interpreter.contract.bytecode.hash())
            .and_modify(|map| map.hit(interpreter.program_counter()));

        Return::Continue
    }
}
