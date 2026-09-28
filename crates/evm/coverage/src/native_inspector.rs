//! Bytecode hit collection for evm2 execution.

use crate::{CallData, HitMap, HitMaps};
use evm2::{
    EvmTypesHost, Inspector,
    interpreter::{Interpreter, Message, MessageResult},
};

/// Line coverage observations collected from evm2 interpreter frames.
#[derive(Clone, Debug, Default)]
pub struct NativeLineCoverageCollector {
    maps: HitMaps,
}

impl NativeLineCoverageCollector {
    /// Returns and clears the observations collected so far.
    pub fn take(&mut self) -> HitMaps {
        std::mem::take(&mut self.maps)
    }
}

impl<T: EvmTypesHost> Inspector<T> for NativeLineCoverageCollector {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let message = interp.message();
        let map = self
            .maps
            .entry(interp.original_bytecode_hash())
            .or_insert_with(|| HitMap::new(interp.original_bytecode()));
        if !message.kind.is_create() {
            let call = CallData::new(&message.input);
            map.call(call, !message.value.is_zero());
        }
        map.reserve(8192.min(interp.original_bytecode().len()));
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let map = self
            .maps
            .entry(interp.original_bytecode_hash())
            .or_insert_with(|| HitMap::new(interp.original_bytecode()));
        map.hit(interp.pc() as u32);
    }

    fn create_end(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        if result.is_success()
            && let Some(map) = self.maps.get_mut(&message.code.hash_slow())
        {
            map.creation();
        }
    }
}
