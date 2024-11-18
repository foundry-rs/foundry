use crate::{HitMap, HitMaps};
use alloy_primitives::B256;
use revm::{interpreter::Interpreter, Database, EvmContext, Inspector};

#[derive(Clone, Debug, Default)]
pub struct CoverageCollector {
    /// Maps that track instruction hit data.
    pub maps: HitMaps,
}

impl<DB: Database> Inspector<DB> for CoverageCollector {
    #[inline]
    fn initialize_interp(&mut self, interp: &mut Interpreter, _context: &mut EvmContext<DB>) {
        self.maps
            .entry(get_contract_hash(interp))
            .or_insert_with(|| HitMap::new(interp.contract.bytecode.original_bytes()));
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter, _context: &mut EvmContext<DB>) {
        self.maps
            .entry(get_contract_hash(interp))
            .and_modify(|map| map.hit(interp.program_counter()));
    }
}

/// Helper function for extracting contract hash used to record coverage hit map.
/// If contract hash available in interpreter contract is zero (contract not yet created but going
/// to be created in current tx) then it hash is calculated from contract bytecode.
fn get_contract_hash(interp: &mut Interpreter) -> B256 {
    let mut hash = interp.contract.hash.expect("Contract hash is None");
    if hash == B256::ZERO {
        hash = interp.contract.bytecode.hash_slow();
    }
    hash
}
