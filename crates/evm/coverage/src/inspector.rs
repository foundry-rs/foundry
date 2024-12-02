use crate::{HitMap, HitMaps};
use alloy_primitives::B256;
use revm::{interpreter::Interpreter, Database, EvmContext, Inspector};

/// Inspector implementation for collecting coverage information.
#[derive(Clone, Debug, Default)]
pub struct CoverageCollector {
    maps: HitMaps,
}

impl<DB: Database> Inspector<DB> for CoverageCollector {
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, _context: &mut EvmContext<DB>) {
        self.maps
            .entry(get_contract_hash(interpreter))
            .or_insert_with(|| HitMap::new(interpreter.contract.bytecode.original_bytes()));
    }

    #[inline]
    fn step(&mut self, interpreter: &mut Interpreter, _context: &mut EvmContext<DB>) {
        self.maps
            .entry(get_contract_hash(interpreter))
            .and_modify(|map| map.hit(interpreter.program_counter()));
    }
}

impl CoverageCollector {
    /// Finish collecting coverage information and return the [`HitMaps`].
    pub fn finish(self) -> HitMaps {
        self.maps
    }
}

/// Helper function for extracting contract hash used to record coverage hit map.
/// If contract hash available in interpreter contract is zero (contract not yet created but going
/// to be created in current tx) then it hash is calculated from contract bytecode.
fn get_contract_hash(interpreter: &mut Interpreter) -> B256 {
    let hash = interpreter.contract.hash.as_mut().expect("coverage does not support EOF");
    if *hash == B256::ZERO {
        *hash = interpreter.contract.bytecode.hash_slow();
    }
    *hash
}
