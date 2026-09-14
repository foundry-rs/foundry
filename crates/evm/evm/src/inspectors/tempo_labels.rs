use alloy_primitives::{Address, U256, keccak256, map::AddressMap};
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
            let name = read_tip20_name(ctx, inputs.target_address);
            self.labels.insert(inputs.target_address, name);
        }

        None
    }
}

/// Reads and decodes a TIP20 token's `name` storage slot.
///
/// This can't call `tempo_precompiles`' own `TIP20Token::name()` (which already decodes both
/// layouts correctly): that function reads through `StorageCtx`, a thread-local that must be
/// entered via `StorageCtx::enter_ctx`, which requires a `ContextTr` with `Block =
/// TempoBlockEnv` and `Cfg = CfgEnv<TempoHardfork>`. `TempoLabels` is instantiated generically
/// for every `FoundryEvmNetwork` (`InspectorStack<FEN>` calls `self.tempo_labels.call(ecx, _)`
/// unconditionally, with `tempo_labels` simply `None` on non-Tempo networks), so its `Inspector`
/// impl can't add those bounds without breaking every other network. The decode logic below is
/// ported from `tempo_precompiles`' storage layer (`bytes_like.rs`: `is_long_string` /
/// `calc_string_length` / long-string chunk addressing at `keccak256(base_slot) + i`) instead.
fn read_tip20_name<CTX, D>(ctx: &mut CTX, address: Address) -> String
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
{
    let base_slot = tempo_precompiles::tip20::slots::NAME;
    let base_value = ctx.db_mut().storage(address, base_slot).unwrap_or_default();
    let data_slot = U256::from_be_bytes(keccak256(base_slot.to_be_bytes::<32>()).0);

    decode_name(base_value, |chunk_index| {
        ctx.db_mut().storage(address, data_slot + U256::from(chunk_index)).unwrap_or_default()
    })
}

/// Decodes a Solidity-style `string` storage value given its base slot value and a loader for
/// the long-string data chunks (each chunk is read lazily, so short strings never touch it).
///
/// Mirrors `tempo_precompiles::storage::types::bytes_like`'s encoding: bit 0 of the base slot's
/// low byte selects short (`<= 31` bytes, inline) vs. long (`>= 32` bytes, `keccak256`-addressed)
/// storage. Falls back to the generic `"TIP20"` label for an empty, corrupt, or unreadable name
/// rather than decoding garbage or panicking.
fn decode_name(base_value: U256, mut load_chunk: impl FnMut(usize) -> U256) -> String {
    let is_long = (base_value.byte(0) & 1) != 0;

    if !is_long {
        let bytes = base_value.to_be_bytes::<32>();
        let len = (bytes[31] / 2) as usize;
        return if len == 0 || len > 31 {
            "TIP20".to_string()
        } else {
            String::from_utf8_lossy(&bytes[..len]).to_string()
        };
    }

    // Long string: base slot stores `length * 2 + 1`; bytes live at `keccak256(base_slot) + i`
    // for each 32-byte chunk `i`.
    let Some(length) = base_value
        .checked_sub(U256::from(1))
        .map(|doubled| doubled >> 1)
        .filter(|len| *len <= U256::from(u32::MAX))
        .map(|len: U256| len.to::<usize>())
    else {
        return "TIP20".to_string();
    };

    let chunks = length.div_ceil(32);
    let mut data = Vec::with_capacity(length);
    for i in 0..chunks {
        let chunk_bytes = load_chunk(i).to_be_bytes::<32>();
        let bytes_to_take = if i == chunks - 1 { length - i * 32 } else { 32 };
        data.extend_from_slice(&chunk_bytes[..bytes_to_take]);
    }

    String::from_utf8_lossy(&data).to_string()
}

#[cfg(test)]
mod audit_repro_tests {
    use super::*;

    /// Encodes a long-string base-slot value (`length * 2 + 1`, bit0 = 1).
    fn long_string_base_value(len: usize) -> U256 {
        U256::from(len * 2 + 1)
    }

    /// Splits `bytes` into the 32-byte chunks a long string's data slots would hold.
    fn chunks_of(bytes: &[u8]) -> Vec<U256> {
        bytes
            .chunks(32)
            .map(|chunk| {
                let mut buf = [0u8; 32];
                buf[..chunk.len()].copy_from_slice(chunk);
                U256::from_be_bytes(buf)
            })
            .collect()
    }

    #[test]
    fn long_string_name_decodes_correctly() {
        // Regression test for the >= 33-byte panic in issue #16842.
        let name = "D".repeat(40);
        let base_value = long_string_base_value(name.len());
        let chunks = chunks_of(name.as_bytes());

        let decoded = decode_name(base_value, |i| chunks[i]);
        assert_eq!(decoded, name);
    }

    #[test]
    fn boundary_32_byte_name_decodes_correctly() {
        // Previously decoded as 32 bytes of slot metadata instead of the real name.
        let name = "E".repeat(32);
        let base_value = long_string_base_value(name.len());
        let chunks = chunks_of(name.as_bytes());

        let decoded = decode_name(base_value, |i| chunks[i]);
        assert_eq!(decoded, name);
    }

    #[test]
    fn short_string_happy_path_unchanged() {
        // Same 8-byte name PR #16324's integration test covers; behavior must be unchanged.
        let name = "AlphaUSD";
        let mut bytes = [0u8; 32];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        bytes[31] = (name.len() * 2) as u8;
        let base_value = U256::from_be_bytes(bytes);

        let decoded = decode_name(base_value, |_| U256::ZERO);
        assert_eq!(decoded, name);
    }

    #[test]
    fn empty_name_falls_back_to_generic_label() {
        let decoded = decode_name(U256::ZERO, |_| U256::ZERO);
        assert_eq!(decoded, "TIP20");
    }
}
