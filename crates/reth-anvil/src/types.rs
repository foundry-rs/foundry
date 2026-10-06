use alloy_primitives::{B256, Bytes};
use alloy_rpc_types_eth::TransactionRequest;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// How the pool orders transactions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionOrder {
    /// In order of arrival.
    Fifo,
    /// By fee, highest first.
    #[default]
    Fees,
}

impl FromStr for TransactionOrder {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fees" => Ok(Self::Fees),
            "fifo" => Ok(Self::Fifo),
            _ => Err(format!("unknown transaction order: `{s}`")),
        }
    }
}

impl fmt::Display for TransactionOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fifo => "fifo",
            Self::Fees => "fees",
        })
    }
}

/// Where to fork from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkChoice {
    /// A block number. A negative value counts back from the latest block.
    Block(i128),
    /// A transaction hash: fork at the block before the transaction's block.
    Transaction(B256),
}

impl ForkChoice {
    /// Returns the block number, if this is a block choice.
    pub const fn block_number(&self) -> Option<i128> {
        match self {
            Self::Block(number) => Some(*number),
            Self::Transaction(_) => None,
        }
    }

    /// Returns the transaction hash, if this is a transaction choice.
    pub const fn transaction_hash(&self) -> Option<B256> {
        match self {
            Self::Block(_) => None,
            Self::Transaction(hash) => Some(*hash),
        }
    }
}

/// A fork URL with an optional `@block` suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkUrl {
    /// The RPC endpoint.
    pub url: String,
    /// The block to fork from.
    pub block: Option<u64>,
}

impl fmt::Display for ForkUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.url.fmt(f)?;
        if let Some(block) = self.block {
            write!(f, "@{block}")?;
        }
        Ok(())
    }
}

impl FromStr for ForkUrl {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((url, block)) = s.rsplit_once('@') {
            if block == "latest" {
                return Ok(Self { url: url.to_string(), block: None });
            }
            if let Ok(block) = block.parse() {
                return Ok(Self { url: url.to_string(), block: Some(block) });
            }
        }
        Ok(Self { url: s.to_string(), block: None })
    }
}

/// Parameters of `anvil_reorg`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReorgOptions {
    /// The number of blocks to rewind and mine again.
    pub depth: u64,
    /// Transactions to include in the mined blocks, by block offset from the common ancestor.
    pub tx_block_pairs: Vec<(TransactionData, u64)>,
}

/// A transaction given to `anvil_reorg`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
#[expect(clippy::large_enum_variant)]
pub enum TransactionData {
    /// A transaction request the node signs for.
    JSON(TransactionRequest),
    /// A signed transaction.
    Raw(Bytes),
}
