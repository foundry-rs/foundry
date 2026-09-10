//! Canary tests replaying Tempo mainnet transactions under the newest hardfork anvil knows.
//!
//! Every test forks Tempo mainnet at the parent of a pinned block, forces the hardfork to
//! [`TempoHardfork::latest`], and re-submits the block's raw transactions at the block's original
//! timestamp. The local receipts must reproduce mainnet: a transaction that succeeded keeps
//! succeeding and, unless a case relaxes it, burns the same gas.
//!
//! The pinned blocks come from the services that keep Tempo busy: Relay's solver and router,
//! Tempo AA payout senders, an ERC-4337 bundler, an ERC-7821 relayer for EIP-7702 accounts, and
//! end users sending through a frontend that appends an ERC-8021 attribution suffix. T11 activated
//! strict ABI decoding for precompile calls and broke the first and last of those on mainnet,
//! because both append bytes to TIP20 `transfer` and `approve` calldata. Replaying the same
//! transactions under T11 before it activated would have shown that, which is what these tests do
//! for every hardfork the pinned `tempo` revision adds; see
//! <https://github.com/tempoxyz/tempo/pull/7598> for the fix that ships in T12.
//!
//! Gas is compared exactly, so the pinned blocks must have been executed under the hardfork that
//! is active on mainnet: a hardfork may change gas accounting, and T11 did for precompile calldata.
//! When the newest hardfork changes it again, relax the affected case with [`GasCheck::Within`] or
//! [`GasCheck::Unchecked`] and record the hardfork and the reason next to it, then re-pin the block
//! past the activation once it is live and restore the exact check.
//!
//! The upstream defaults to the public endpoint and honours `TEMPO_MAINNET_RPC_URL`, see
//! [`next_tempo_mainnet_rpc_endpoint`].

use crate::utils::http_provider;
use alloy_network::ReceiptResponse;
use alloy_primitives::B256;
use alloy_provider::{Provider, ext::DebugApi};
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::next_tempo_mainnet_rpc_endpoint;
use std::fmt;
use tempo_hardfork::TempoHardfork;

/// Relay's solver settles fills with a plain TIP20 `transfer` of USDC.e.
///
/// Until T11 activated the solver appended the 32-byte request id to the calldata; the strict
/// decoding T11 introduced rejected those bytes and every fill failed, see
/// [`test_tempo_canary_fork_relay_transfer_trailing_bytes_across_hardforks`]. The pinned block
/// holds a fill sent after Relay dropped the suffix.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_usdce_transfer() {
    replay_block(38_917_407, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// The Relay fill reported during the T11 incident, replayed under the hardforks around the
/// change: T10 accepts the trailing request id, T11 rejects it exactly as mainnet did, and the
/// latest hardfork accepts it again.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_transfer_trailing_bytes_across_hardforks() {
    assert_trailing_bytes_regression(38_915_456).await;
}

/// Relay's solver also pays out in pathUSD, the fee token every account holds.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_path_usd_transfer() {
    replay_block(38_928_039, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// Relay's ERC20 router pulls the user's funds through a Permit2 signature and forwards them in
/// one call, the entry point of a cross-chain swap into Tempo.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_router_permit2_multicall() {
    replay_block(38_892_542, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// An end user approving USDC.e through a frontend that appends an ERC-8021 attribution suffix to
/// the calldata. T11 rejected the suffix like Relay's request id, taking every approval from that
/// frontend down with it; the pinned block holds one of those failed approvals.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_erc8021_attribution_suffix_across_hardforks() {
    assert_trailing_bytes_regression(38_917_602).await;
}

/// A payout sender submitting Tempo AA transactions whose single call is a TIP20 `transfer`, paid
/// for in USDC.e.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_aa_tip20_transfer() {
    replay_block(38_891_824, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// A sponsored Tempo AA transaction carrying a fee payer signature and a validity window, whose
/// call is a TIP20 `transferWithMemo`.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_aa_sponsored_transfer_with_memo() {
    replay_block(38_891_826, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// An ERC-4337 bundler submitting `handleOps` to the v0.7 entry point.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_erc4337_bundler_handle_ops() {
    replay_block(38_891_903, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// A relayer executing an ERC-7821 batch on an EIP-7702 delegated account.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_erc7821_execute_on_delegated_account() {
    replay_block(38_891_835, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// Replays a block holding a precompile call with trailing calldata bytes that failed on mainnet
/// under T11: T10 still accepts the bytes, T11 reproduces the mainnet failure, and the latest
/// hardfork accepts them again.
async fn assert_trailing_bytes_regression(number: u64) {
    let before = replay_block(number, TempoHardfork::T10).await;
    before.assert_all_succeed();

    let regression = replay_block(number, TempoHardfork::T11).await;
    regression.assert_matches_mainnet(GasCheck::Exact);
    assert!(
        regression.transactions.iter().all(|tx| !tx.mainnet_success),
        "{regression}: expected the block to hold the failed call"
    );

    let fixed = replay_block(number, TempoHardfork::latest()).await;
    fixed.assert_all_succeed();
}

/// How the gas a replayed transaction used is compared against its mainnet receipt.
#[derive(Clone, Copy)]
enum GasCheck {
    /// The replay must use exactly the gas mainnet recorded.
    Exact,
    /// The replay may deviate from mainnet by up to this many gas units.
    #[expect(dead_code)]
    Within(u64),
    /// Gas is reported in failures but not compared.
    #[expect(dead_code)]
    Unchecked,
}

/// A mainnet block replayed on a fork of its parent under a forced hardfork.
struct ReplayedBlock {
    number: u64,
    hardfork: TempoHardfork,
    transactions: Vec<ReplayedTransaction>,
}

/// A replayed transaction next to the outcome mainnet recorded for it.
struct ReplayedTransaction {
    hash: B256,
    mainnet_success: bool,
    mainnet_gas_used: u64,
    success: bool,
    gas_used: u64,
}

impl ReplayedBlock {
    /// Asserts every transaction reproduced its mainnet status and, per `gas`, its gas usage.
    fn assert_matches_mainnet(&self, gas: GasCheck) {
        for tx in &self.transactions {
            assert_eq!(
                tx.success, tx.mainnet_success,
                "{self}: {} diverged from mainnet: success {} vs {} (gas {} vs {})",
                tx.hash, tx.success, tx.mainnet_success, tx.gas_used, tx.mainnet_gas_used
            );
            match gas {
                GasCheck::Exact => assert_eq!(
                    tx.gas_used, tx.mainnet_gas_used,
                    "{self}: {} used different gas than on mainnet",
                    tx.hash
                ),
                GasCheck::Within(tolerance) => assert!(
                    tx.gas_used.abs_diff(tx.mainnet_gas_used) <= tolerance,
                    "{self}: {} used {} gas, mainnet used {}, tolerance {tolerance}",
                    tx.hash,
                    tx.gas_used,
                    tx.mainnet_gas_used
                ),
                GasCheck::Unchecked => {}
            }
        }
    }

    /// Asserts every transaction succeeded, whatever mainnet recorded for it.
    fn assert_all_succeed(&self) {
        for tx in &self.transactions {
            assert!(
                tx.success,
                "{self}: {} failed (gas {}, mainnet success {} with gas {})",
                tx.hash, tx.gas_used, tx.mainnet_success, tx.mainnet_gas_used
            );
        }
    }
}

impl fmt::Display for ReplayedBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block {} under {}", self.number, self.hardfork)
    }
}

/// Forks Tempo mainnet at the parent of `number`, forces `hardfork`, and replays the block's raw
/// transactions in order at the block's timestamp, one local block each.
async fn replay_block(number: u64, hardfork: TempoHardfork) -> ReplayedBlock {
    let rpc_url = next_tempo_mainnet_rpc_endpoint();
    let mainnet = http_provider(&rpc_url);

    let block = mainnet
        .get_block_by_number(BlockNumberOrTag::Number(number))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("tempo mainnet block {number} not found"));
    let receipts = mainnet
        .get_block_receipts(BlockId::number(number))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("tempo mainnet block {number} has no receipts"));
    assert!(!receipts.is_empty(), "tempo mainnet block {number} is empty");
    assert_eq!(receipts.len(), block.transactions.len(), "block {number} receipts");

    let (api, _handle) = spawn(
        NodeConfig::test_tempo()
            .with_eth_rpc_url(Some(rpc_url))
            .with_fork_block_number(Some(number - 1))
            .with_hardfork(Some(hardfork.into())),
    )
    .await;
    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.hard_fork, hardfork.to_string(), "forced hardfork was not applied");
    api.anvil_set_auto_mine(false).await.unwrap();

    let mut transactions = Vec::with_capacity(receipts.len());
    for receipt in receipts {
        let hash = receipt.transaction_hash();
        let raw = mainnet.debug_get_raw_transaction(hash).await.unwrap();

        // Tempo blocks are sub-second apart, so the parent may share the timestamp; anvil accepts
        // an equal one. Setting it before submitting also validates time bounds against it.
        api.evm_set_next_block_timestamp(block.header.timestamp).unwrap();
        let sent = api.send_raw_transaction(raw).await.unwrap_or_else(|err| {
            panic!("block {number} under {hardfork}: {hash} rejected: {err}")
        });
        assert_eq!(sent, hash, "block {number}: raw transaction decoded to a different hash");
        api.mine_one().await.unwrap();

        let local = api
            .transaction_receipt(hash)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("block {number} under {hardfork}: {hash} was not mined"));
        transactions.push(ReplayedTransaction {
            hash,
            mainnet_success: receipt.status(),
            mainnet_gas_used: receipt.gas_used(),
            success: local.status(),
            gas_used: local.gas_used(),
        });
    }

    ReplayedBlock { number, hardfork, transactions }
}
