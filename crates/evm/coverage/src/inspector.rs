use crate::{HitMap, HitMaps};
use revm::{interpreter::Interpreter, Database, EvmContext, Inspector};

#[derive(Clone, Debug, Default)]
pub struct CoverageCollector {
    /// Maps that track instruction hit data.
    pub maps: HitMaps,
}

impl<DB: Database> Inspector<DB> for CoverageCollector {
    #[inline]
    fn initialize_interp(&mut self, interp: &mut Interpreter, _context: &mut EvmContext<DB>) {
        let hash = interp.contract.hash.expect("Contract hash is None");
        self.maps
            .entry(hash)
            .or_insert_with(|| HitMap::new(interp.contract.bytecode.original_bytes()));
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter, _context: &mut EvmContext<DB>) {
        let hash = interp.contract.hash.expect("Contract hash is None");
        self.maps.entry(hash).and_modify(|map| map.hit(interp.program_counter()));
    }
}
