//! 'eth_simulateV1' Request / Response types: <https://github.com/ethereum/execution-apis/pull/484>

use crate::{state::StateOverride, Block, BlockOverrides, Log, TransactionRequest};
use alloc::{string::String, vec::Vec};
use alloy_primitives::Bytes;

/// The maximum number of blocks that can be simulated in a single request,
pub const MAX_SIMULATE_BLOCKS: u64 = 256;

/// Represents a batch of calls to be simulated sequentially within a block.
/// This struct includes block and state overrides as well as the transaction requests to be
/// executed.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SimBlock {
    /// Modifications to the default block characteristics.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub block_overrides: Option<BlockOverrides>,
    /// State modifications to apply before executing the transactions.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub state_overrides: Option<StateOverride>,
    /// A vector of transactions to be simulated.
    #[cfg_attr(feature = "serde", serde(default))]
    pub calls: Vec<TransactionRequest>,
}

impl SimBlock {
    /// Enables state overrides
    pub fn with_state_overrides(mut self, overrides: StateOverride) -> Self {
        self.state_overrides = Some(overrides);
        self
    }

    /// Enables block overrides
    pub fn with_block_overrides(mut self, overrides: BlockOverrides) -> Self {
        self.block_overrides = Some(overrides);
        self
    }

    /// Adds a call to the block.
    pub fn call(mut self, call: TransactionRequest) -> Self {
        self.calls.push(call);
        self
    }

    /// Adds multiple calls to the block.
    pub fn extend_calls(mut self, calls: impl IntoIterator<Item = TransactionRequest>) -> Self {
        self.calls.extend(calls);
        self
    }
}

/// Represents the result of simulating a block.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SimulatedBlock<B = Block> {
    /// The simulated block.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub inner: B,
    /// A vector of results for each call in the block.
    pub calls: Vec<SimCallResult>,
}

/// Captures the outcome of a transaction simulation.
/// It includes the return value, logs produced, gas used, and the status of the transaction.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SimCallResult {
    /// The raw bytes returned by the transaction.
    pub return_data: Bytes,
    /// Logs generated during the execution of the transaction.
    #[cfg_attr(feature = "serde", serde(default))]
    pub logs: Vec<Log>,
    /// The amount of gas used by the transaction.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub gas_used: u64,
    /// The final status of the transaction, typically indicating success or failure.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub status: bool,
    /// Error in case the call failed
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub error: Option<SimulateError>,
}

/// Simulation options for executing multiple blocks and transactions.
///
/// This struct configures how simulations are executed, including whether to trace token transfers,
/// validate transaction sequences, and whether to return full transaction objects.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SimulatePayload {
    /// Array of block state calls to be executed at specific, optional block/state.
    #[cfg_attr(feature = "serde", serde(default))]
    pub block_state_calls: Vec<SimBlock>,
    /// Flag to determine whether to trace ERC20/ERC721 token transfers within transactions.
    #[cfg_attr(feature = "serde", serde(default))]
    pub trace_transfers: bool,
    /// Flag to enable or disable validation of the transaction sequence in the blocks.
    #[cfg_attr(feature = "serde", serde(default))]
    pub validation: bool,
    /// Flag to decide if full transactions should be returned instead of just their hashes.
    #[cfg_attr(feature = "serde", serde(default))]
    pub return_full_transactions: bool,
}

impl SimulatePayload {
    /// Adds a block to the simulation payload.
    pub fn extend(mut self, block: SimBlock) -> Self {
        self.block_state_calls.push(block);
        self
    }

    /// Adds multiple blocks to the simulation payload.
    pub fn extend_blocks(mut self, blocks: impl IntoIterator<Item = SimBlock>) -> Self {
        self.block_state_calls.extend(blocks);
        self
    }

    /// Enables tracing of token transfers.
    pub const fn with_trace_transfers(mut self) -> Self {
        self.trace_transfers = true;
        self
    }

    /// Enables validation of the transaction sequence.
    pub const fn with_validation(mut self) -> Self {
        self.validation = true;
        self
    }

    /// Enables returning full transactions.
    pub const fn with_full_transactions(mut self) -> Self {
        self.return_full_transactions = true;
        self
    }
}

/// The error response returned by the `eth_simulateV1` method.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SimulateError {
    /// Code error
    /// -3200: Execution reverted
    /// -32015: VM execution error
    pub code: i32,
    /// Message error
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, TxKind};
    use serde_json::json;
    use similar_asserts::assert_eq;

    #[test]
    #[cfg(feature = "serde")]
    fn test_eth_simulate_v1_account_not_precompile() {
        let request_json = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_simulateV1",
            "params": [{
                "blockStateCalls": [
                    {
                        "blockOverrides": {},
                        "stateOverrides": {
                            "0xc000000000000000000000000000000000000000": {
                                "nonce": "0x5"
                            }
                        },
                        "calls": []
                    },
                    {
                        "blockOverrides": {},
                        "stateOverrides": {
                            "0xc000000000000000000000000000000000000000": {
                                "code": "0x600035600055"
                            }
                        },
                        "calls": [
                            {
                                "from": "0xc000000000000000000000000000000000000000",
                                "to": "0xc000000000000000000000000000000000000000",
                                "nonce": "0x0"
                            },
                            {
                                "from": "0xc100000000000000000000000000000000000000",
                                "to": "0xc100000000000000000000000000000000000000",
                                "nonce": "0x5"
                            }
                        ]
                    }
                ],
                "traceTransfers": false,
                "validation": true,
                "returnFullTransactions": false
            }, "latest"]
        });

        let sim_opts: SimulatePayload =
            serde_json::from_value(request_json["params"][0].clone()).unwrap();

        let address_1: Address = "0xc000000000000000000000000000000000000000".parse().unwrap();
        let address_2: Address = "0xc100000000000000000000000000000000000000".parse().unwrap();

        assert!(sim_opts.validation);
        assert_eq!(sim_opts.block_state_calls.len(), 2);

        let block_state_call_1 = &sim_opts.block_state_calls[0];
        assert!(block_state_call_1.state_overrides.as_ref().unwrap().contains_key(&address_1));
        assert_eq!(
            block_state_call_1
                .state_overrides
                .as_ref()
                .unwrap()
                .get(&address_1)
                .unwrap()
                .nonce
                .unwrap(),
            5
        );

        let block_state_call_2 = &sim_opts.block_state_calls[1];
        assert!(block_state_call_2.state_overrides.as_ref().unwrap().contains_key(&address_1));

        assert_eq!(block_state_call_2.calls.len(), 2);
        assert_eq!(block_state_call_2.calls[0].from.unwrap(), address_1);
        assert_eq!(block_state_call_2.calls[0].to.unwrap(), TxKind::Call(address_1));
        assert_eq!(block_state_call_2.calls[0].nonce.unwrap(), 0);
        assert_eq!(block_state_call_2.calls[1].from.unwrap(), address_2);
        assert_eq!(block_state_call_2.calls[1].to.unwrap(), TxKind::Call(address_2));
        assert_eq!(block_state_call_2.calls[1].nonce.unwrap(), 5);
    }
}
