use alloy_primitives::{U256, keccak256, map::AddressMap};
use evm2::{
    EvmTypes, Inspector,
    interpreter::{Interpreter, Message, MessageResult},
};
use tempo_primitives::TempoAddressExt;

// Limit labels to 256 bytes so long names require at most eight additional storage reads.
const MAX_NAME_BYTES: usize = 256;

/// Inspector that labels TIP20 token precompile addresses with their on-chain names.
///
/// During execution, when a call targets a TIP20 address, this inspector reads the token's
/// name from storage and records the `address -> name` mapping. These labels are later merged
/// into trace output for better readability.
#[derive(Default, Clone, Debug)]
pub struct TempoLabels {
    pub(crate) labels: AddressMap<String>,
}

impl<T: EvmTypes> Inspector<T> for TempoLabels {
    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        let address = message.destination;
        if address.is_tip20() && !self.labels.contains_key(&address) {
            let state = interp.host().state_mut();
            let name = 'decode: {
                let slot = tempo_precompiles::tip20::slots::NAME;
                let Ok(value) = state.storage_slot_untracked(&address, &slot) else {
                    break 'decode None;
                };
                let bytes = value.to_be_bytes::<32>();
                if bytes[31] & 1 == 0 {
                    let len = usize::from(bytes[31] / 2);
                    break 'decode (1..=31)
                        .contains(&len)
                        .then(|| String::from_utf8_lossy(&bytes[..len]).into_owned());
                }

                // Long strings store length * 2 + 1 in the base slot and data at keccak256(slot).
                let len = value >> 1usize;
                if len < U256::from(32) || len > U256::from(MAX_NAME_BYTES) {
                    break 'decode None;
                }
                let len = len.to::<usize>();
                let start = Into::<U256>::into(keccak256(slot.to_be_bytes::<32>()));
                let mut data = Vec::with_capacity(len);
                for i in 0..len.div_ceil(32) {
                    let Ok(chunk) =
                        state.storage_slot_untracked(&address, &(start + U256::from(i)))
                    else {
                        break 'decode None;
                    };
                    let chunk = chunk.to_be_bytes::<32>();
                    data.extend_from_slice(&chunk[..(len - data.len()).min(32)]);
                }
                Some(String::from_utf8_lossy(&data).into_owned())
            }
            .unwrap_or_else(|| "TIP20".to_string());
            self.labels.insert(address, name);
        }

        None
    }
}
