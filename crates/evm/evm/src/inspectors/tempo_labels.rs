use alloy_primitives::map::AddressMap;
use foundry_evm_core::backend::DatabaseError;
use revm::{
    Database, Inspector,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{CallInputs, CallOutcome, interpreter::EthInterpreter},
};
use tempo_primitives::TempoAddressExt;

/// Inspector that labels TIP20 token precompile addresses with their on-chain names.
///
/// During execution, when a call targets a TIP20 address, this inspector reads the token's
/// name from storage and records the `address -> name` mapping. These labels are later merged
/// into trace output for better readability.
#[derive(Default, Clone, Debug)]
pub struct TempoLabels {
    pub(crate) labels: AddressMap<String>,
}

impl<CTX, D> Inspector<CTX, EthInterpreter> for TempoLabels
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
    CTX::Journal: JournalExt,
{
    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address.is_tip20() && !self.labels.contains_key(&inputs.target_address) {
            let bytes = ctx
                .db_mut()
                .storage(inputs.target_address, tempo_precompiles::tip20::slots::NAME)
                .unwrap_or_default()
                .to_be_bytes::<32>();
            // Only short strings store their data inline, with length * 2 in the last byte.
            let len = bytes[31] as usize / 2;
            let name = if bytes[31] & 1 != 0 || len == 0 || len > 31 {
                "TIP20".to_string()
            } else {
                String::from_utf8_lossy(&bytes[..len]).to_string()
            };
            self.labels.insert(inputs.target_address, name);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256, address};
    use foundry_evm_core::backend::FoundryEvmInMemoryDB;
    use revm::{
        Context, MainContext,
        interpreter::{CallInput, CallScheme, CallValue},
    };

    #[test]
    fn labels_only_decode_valid_inline_names() {
        let address = address!("20C0000000000000000000000000000000000001");
        let mut inputs = CallInputs {
            input: CallInput::Bytes(Default::default()),
            return_memory_offset: 0..0,
            gas_limit: 0,
            reservoir: 0,
            bytecode_address: address,
            known_bytecode: Default::default(),
            target_address: address,
            caller: Address::ZERO,
            value: CallValue::Transfer(U256::ZERO),
            scheme: CallScheme::StaticCall,
            is_static: true,
            charged_new_account_state_gas: false,
        };
        let mut ctx = Context::mainnet().with_db(FoundryEvmInMemoryDB::default());

        // Every possible marker must either decode a valid short name or use the fallback.
        for marker in 0..=u8::MAX {
            let mut bytes = [b'a'; 32];
            bytes[31] = marker;
            ctx.db_mut()
                .insert_account_storage(
                    address,
                    tempo_precompiles::tip20::slots::NAME,
                    U256::from_be_bytes(bytes),
                )
                .unwrap();
            let mut inspector = TempoLabels::default();
            assert!(inspector.call(&mut ctx, &mut inputs).is_none());
            let expected = if marker != 0 && marker <= 62 && marker % 2 == 0 {
                "a".repeat(usize::from(marker / 2))
            } else {
                "TIP20".to_string()
            };
            assert_eq!(inspector.labels[&address], expected, "marker {marker}");
        }
    }
}
