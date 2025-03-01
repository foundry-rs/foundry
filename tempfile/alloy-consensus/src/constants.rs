//! Ethereum protocol-related constants
use alloy_primitives::{b256, B256};

pub use alloy_eips::eip2718::{
    EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID, EIP4844_TX_TYPE_ID, EIP7702_TX_TYPE_ID,
    LEGACY_TX_TYPE_ID,
};
pub use alloy_trie::{EMPTY_ROOT_HASH, KECCAK_EMPTY};

/// The first four bytes of the call data for a function call specifies the function to be called.
pub const SELECTOR_LEN: usize = 4;

/// Maximum extra data size in a block after genesis
pub const MAXIMUM_EXTRA_DATA_SIZE: usize = 32;

/// Multiplier for converting gwei to wei.
pub const GWEI_TO_WEI: u64 = 1_000_000_000;

/// Multiplier for converting finney (milliether) to wei.
pub const FINNEY_TO_WEI: u128 = (GWEI_TO_WEI as u128) * 1_000_000;

/// Multiplier for converting ether to wei.
pub const ETH_TO_WEI: u128 = FINNEY_TO_WEI * 1000;

/// Multiplier for converting mgas to gas.
pub const MGAS_TO_GAS: u64 = 1_000_000u64;

/// The Ethereum mainnet genesis hash.
pub const MAINNET_GENESIS_HASH: B256 =
    b256!("d4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3");

/// Goerli genesis hash.
pub const GOERLI_GENESIS_HASH: B256 =
    b256!("bf7e331f7f7c1dd2e05159666b3bf8bc7a8a3a9eb1d518969eab529dd9b88c1a");

/// Sepolia genesis hash.
pub const SEPOLIA_GENESIS_HASH: B256 =
    b256!("25a5cc106eea7138acab33231d7160d69cb777ee0c2c553fcddf5138993e6dd9");

/// Holesky genesis hash.
pub const HOLESKY_GENESIS_HASH: B256 =
    b256!("b5f7f912443c940f21fd611f12828d75b534364ed9e95ca4e307729a4661bde4");

/// Testnet genesis hash.
pub const DEV_GENESIS_HASH: B256 =
    b256!("2f980576711e3617a5e4d83dd539548ec0f7792007d505a3d2e9674833af2d7c");

/// Optimism goerli genesis hash.
pub const GOERLI_OP_GENESIS: B256 =
    b256!("c1fc15cd51159b1f1e5cbc4b82e85c1447ddfa33c52cf1d98d14fba0d6354be1");

/// Base goerli genesis hash.
pub const GOERLI_BASE_GENESIS: B256 =
    b256!("a3ab140f15ea7f7443a4702da64c10314eb04d488e72974e02e2d728096b4f76");

/// Ommer root of empty list.
pub const EMPTY_OMMER_ROOT_HASH: B256 =
    b256!("1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347");

/// Transactions root of empty receipts set.
pub const EMPTY_RECEIPTS: B256 = EMPTY_ROOT_HASH;

/// Transactions root of empty transactions set.
pub const EMPTY_TRANSACTIONS: B256 = EMPTY_ROOT_HASH;

/// Withdrawals root of empty withdrawals set.
pub const EMPTY_WITHDRAWALS: B256 = EMPTY_ROOT_HASH;
