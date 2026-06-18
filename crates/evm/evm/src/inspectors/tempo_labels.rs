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
            let len = bytes[31] as usize / 2; // Last byte stores length * 2 for short strings
            let name = if len == 0 {
                "TIP20".to_string()
            } else {
                String::from_utf8_lossy(&bytes[..len]).to_string()
            };
            self.labels.insert(inputs.target_address, name);
        }

        None
    }
}
