//! Types for opcode tracing.

use alloy_primitives::{BlockHash, TxHash};
use serde::{Deserialize, Serialize};

/// Opcode gas usage for a transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOpcodeGas {
    /// The block hash
    pub block_hash: BlockHash,
    /// The block number
    pub block_number: u64,
    /// All executed transactions in the block in the order they were executed, with their opcode
    /// gas usage.
    pub transactions: Vec<TransactionOpcodeGas>,
}

impl BlockOpcodeGas {
    /// Returns true if the block contains the given opcode.
    pub fn contains(&self, opcode: &str) -> bool {
        self.transactions.iter().any(|tx| tx.contains(opcode))
    }
}

/// Opcode gas usage for a transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[doc(alias = "TxOpcodeGas")]
pub struct TransactionOpcodeGas {
    /// The transaction hash
    #[doc(alias = "tx_hash")]
    pub transaction_hash: TxHash,
    /// The gas used by each opcode in the transaction
    pub opcode_gas: Vec<OpcodeGas>,
}

impl TransactionOpcodeGas {
    /// Returns true if the transaction contains the given opcode.
    pub fn contains(&self, opcode: &str) -> bool {
        self.opcode_gas.iter().any(|op| op.opcode.eq_ignore_ascii_case(opcode))
    }
}

/// Gas information for a single opcode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpcodeGas {
    /// The name of the opcode
    pub opcode: String,
    /// How many times the opcode was executed
    pub count: u64,
    /// Combined gas used by all instances of the opcode
    ///
    /// For opcodes with constant gas costs, this is the constant opcode gas cost times the count.
    pub gas_used: u64,
}
