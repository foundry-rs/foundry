//! HyperEVM read precompiles reconstructed from archive-node block data.
//!
//! HyperEVM's read precompiles expose HyperCore state, so their results cannot be derived from the
//! EVM state alone. Archive nodes based on `nanoreth` expose the calls made while executing a block
//! through `eth_blockPrecompileData`. This module installs those recorded calls as exact
//! `(calldata, gas limit)` lookups for historical replay.

use alloy_evm::precompiles::{DynPrecompile, PrecompilesMap};
use alloy_primitives::{Address, B256, Bytes, U256};
use revm::precompile::{PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap};

/// HyperEVM mainnet chain ID.
pub const HYPEREVM_MAINNET_CHAIN_ID: u64 = 999;

/// HyperEVM testnet chain ID.
pub const HYPEREVM_TESTNET_CHAIN_ID: u64 = 998;

const READ_PRECOMPILE_START: u16 = 0x800;
const DEFAULT_READ_PRECOMPILE_END: u16 = 0x80d;
const READ_PRECOMPILE_PAGE_END: u16 = 0x8ff;
const WARM_PRECOMPILES_BLOCK_NUMBER: u64 = 8_197_684;

/// Block-scoped HyperEVM precompile inputs returned by `eth_blockPrecompileData`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperEvmBlockPrecompileData {
    /// Calls to HyperEVM read precompiles made while executing the block.
    pub read_precompile_calls: Option<HyperEvmReadPrecompileCalls>,
    /// Highest active read precompile address reported by the node.
    pub highest_precompile_address: Option<Address>,
}

/// Recorded HyperEVM read precompile calls grouped by address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HyperEvmReadPrecompileCalls(
    pub Vec<(Address, Vec<(HyperEvmReadPrecompileInput, HyperEvmReadPrecompileResult)>)>,
);

/// The input key for one recorded HyperEVM read precompile call.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HyperEvmReadPrecompileInput {
    /// Call data supplied to the precompile.
    pub input: Bytes,
    /// Gas supplied to the precompile.
    pub gas_limit: u64,
}

/// The recorded outcome of a HyperEVM read precompile call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HyperEvmReadPrecompileResult {
    /// The call succeeded with the recorded gas cost and return bytes.
    Ok { gas_used: u64, bytes: Bytes },
    /// The call exhausted all supplied gas.
    OutOfGas,
    /// The HyperCore query rejected the input.
    Error,
    /// The source node encountered an unexpected query error.
    UnexpectedError,
}

/// Returns whether the chain ID identifies HyperEVM mainnet or testnet.
pub const fn is_hyperevm_chain(chain_id: u64) -> bool {
    matches!(chain_id, HYPEREVM_MAINNET_CHAIN_ID | HYPEREVM_TESTNET_CHAIN_ID)
}

impl HyperEvmBlockPrecompileData {
    /// Installs the recorded block's HyperEVM read precompiles.
    ///
    /// Inputs absent from the recording halt with out-of-gas, matching HyperEVM's behavior for an
    /// invalid query and preventing replay from inventing HyperCore state.
    pub fn inject(&self, precompiles: &mut PrecompilesMap, block_number: u64) {
        // Clear the complete page first in case the map was reused with older block-scoped data.
        let addresses = precompiles.addresses().copied().collect::<Vec<_>>();
        for address in addresses {
            if read_precompile_number(address).is_some() {
                precompiles.apply_precompile(&address, |_| None);
            }
        }

        for (address, calls) in
            self.read_precompile_calls.as_ref().map(|calls| calls.0.as_slice()).unwrap_or_default()
        {
            if read_precompile_number(*address).is_none() {
                continue;
            }
            let calls = calls.iter().cloned().collect::<HashMap<_, _>>();
            let id = PrecompileId::Custom(Cow::Owned(format!("HyperEVM read {address}")));
            precompiles.apply_precompile(address, |_| {
                Some(DynPrecompile::new_stateful(id, move |input| {
                    execute_recorded_call(&calls, input.data, input.gas, input.reservoir)
                }))
            });
        }

        if block_number < WARM_PRECOMPILES_BLOCK_NUMBER {
            return;
        }

        let highest = self
            .highest_precompile_address
            .and_then(read_precompile_number)
            .unwrap_or(DEFAULT_READ_PRECOMPILE_END)
            .clamp(READ_PRECOMPILE_START, READ_PRECOMPILE_PAGE_END);
        for number in READ_PRECOMPILE_START..=highest {
            let address = read_precompile_address(number);
            precompiles.apply_precompile(&address, |existing| {
                existing.or_else(|| {
                    let id = PrecompileId::Custom(Cow::Owned(format!("HyperEVM read {address}")));
                    Some(DynPrecompile::new_stateful(id, |input| {
                        Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, input.reservoir))
                    }))
                })
            });
        }
    }
}

fn execute_recorded_call(
    calls: &HashMap<HyperEvmReadPrecompileInput, HyperEvmReadPrecompileResult>,
    data: &[u8],
    gas_limit: u64,
    reservoir: u64,
) -> PrecompileResult {
    let input = HyperEvmReadPrecompileInput { input: Bytes::copy_from_slice(data), gas_limit };
    match calls.get(&input) {
        Some(HyperEvmReadPrecompileResult::Ok { gas_used, bytes }) => {
            Ok(PrecompileOutput::new(*gas_used, bytes.clone(), reservoir))
        }
        Some(
            HyperEvmReadPrecompileResult::OutOfGas
            | HyperEvmReadPrecompileResult::Error
            | HyperEvmReadPrecompileResult::UnexpectedError,
        )
        | None => Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir)),
    }
}

fn read_precompile_address(number: u16) -> Address {
    Address::from_word(B256::from(U256::from(number)))
}

fn read_precompile_number(address: Address) -> Option<u16> {
    let bytes = address.as_slice();
    (bytes[..18] == [0; 18] && bytes[18] == 8).then(|| u16::from_be_bytes([8, bytes[19]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::precompile::{PrecompileStatus, Precompiles};

    const FIXTURE: &str = r#"{
        "read_precompile_calls": [[
            "0x000000000000000000000000000000000000080e",
            [[
                {
                    "input": "0x000000000000000000000000000000000000000000000000000000000000277b",
                    "gas_limit": 120530
                },
                {
                    "Ok": {
                        "gas_used": 4168,
                        "bytes": "0x0000000000000000000000000000000000000000000000000000000004f8d3c00000000000000000000000000000000000000000000000000000000004f8d7a8"
                    }
                }
            ]]
        ]],
        "highest_precompile_address": "0x0000000000000000000000000000000000000814"
    }"#;

    #[test]
    fn deserializes_archive_node_response() {
        let data: HyperEvmBlockPrecompileData = serde_json::from_str(FIXTURE).unwrap();
        let calls = &data.read_precompile_calls.unwrap().0;

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, read_precompile_address(0x80e));
        assert_eq!(calls[0].1[0].0.gas_limit, 120_530);
        assert_eq!(data.highest_precompile_address, Some(read_precompile_address(0x814)));
    }

    #[test]
    fn returns_only_exact_recorded_call() {
        let input =
            HyperEvmReadPrecompileInput { input: Bytes::from_static(b"input"), gas_limit: 10_000 };
        let calls = HashMap::from([(
            input.clone(),
            HyperEvmReadPrecompileResult::Ok {
                gas_used: 2_325,
                bytes: Bytes::from_static(b"output"),
            },
        )]);

        let output = execute_recorded_call(&calls, &input.input, input.gas_limit, 7).unwrap();
        assert_eq!(output.status, PrecompileStatus::Success);
        assert_eq!(output.gas_used, 2_325);
        assert_eq!(output.bytes, Bytes::from_static(b"output"));
        assert_eq!(output.reservoir, 7);

        let output = execute_recorded_call(&calls, &input.input, input.gas_limit + 1, 0).unwrap();
        assert_eq!(output.status, PrecompileStatus::Halt(PrecompileHalt::OutOfGas));
    }

    #[test]
    fn installs_warm_precompile_range_and_rejects_invalid_highest_address() {
        let mut precompiles = PrecompilesMap::from_static(Precompiles::prague());
        HyperEvmBlockPrecompileData {
            read_precompile_calls: None,
            highest_precompile_address: Some(Address::ZERO),
        }
        .inject(&mut precompiles, WARM_PRECOMPILES_BLOCK_NUMBER);

        for number in READ_PRECOMPILE_START..=DEFAULT_READ_PRECOMPILE_END {
            assert!(
                precompiles.addresses().any(|address| *address == read_precompile_address(number))
            );
        }
        assert!(!precompiles
            .addresses()
            .any(|address| *address == read_precompile_address(DEFAULT_READ_PRECOMPILE_END + 1)));
    }

    #[test]
    fn does_not_warm_unrecorded_precompiles_before_activation() {
        let mut precompiles = PrecompilesMap::from_static(Precompiles::prague());
        HyperEvmBlockPrecompileData::default()
            .inject(&mut precompiles, WARM_PRECOMPILES_BLOCK_NUMBER - 1);

        assert!(
            !precompiles
                .addresses()
                .any(|address| *address == read_precompile_address(READ_PRECOMPILE_START))
        );
    }
}
