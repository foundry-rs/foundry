//! Native Ethereum coverage observations using the shared bytecode hit maps.

use crate::{CallData, HitMap, HitMaps};
use evm2::{
    Inspector,
    interpreter::{Interpreter, Message, MessageResult},
};
use foundry_evm_core::ethereum::FoundryEvmTypes;

/// Coverage collected by an owned native execution inspector.
#[derive(Clone, Debug, Default)]
pub struct EthereumCoverageCollector {
    maps: HitMaps,
}

impl EthereumCoverageCollector {
    /// Returns observations for the existing source coverage reporters.
    pub fn finish(self) -> HitMaps {
        self.maps
    }

    fn map(&mut self, interp: &Interpreter<'_, '_, FoundryEvmTypes>) -> &mut HitMap {
        self.maps
            .entry(interp.original_bytecode_hash())
            .or_insert_with(|| HitMap::new(interp.original_bytecode()))
    }
}

impl Inspector<FoundryEvmTypes> for EthereumCoverageCollector {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        let message = interp.message();
        let call = (!message.kind.is_create())
            .then(|| (CallData::new(&message.input), !message.value.is_zero()));
        let map = self.map(interp);
        if let Some((call, with_value)) = call {
            map.call(call, with_value);
        }
        map.reserve(8192.min(interp.original_bytecode_slice().len()));
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        self.map(interp).hit(interp.pc() as u32);
    }

    fn create_end(
        &mut self,
        _interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        if result.stop.is_success()
            && let Some(map) = self.maps.get_mut(&alloy_primitives::keccak256(&message.input))
        {
            map.creation();
        }
    }
}
