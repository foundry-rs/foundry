//! Fork test helpers. The fork tests themselves follow anvil's `fork.rs` as they are ported.

use foundry_test_utils::rpc;
use reth_anvil::NodeConfig;

pub const BLOCK_NUMBER: u64 = 14_608_400u64;

/// A mainnet fork at [`BLOCK_NUMBER`].
pub fn fork_config() -> NodeConfig {
    NodeConfig::test()
        .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_url()))
        .with_fork_block_number(Some(BLOCK_NUMBER))
}
