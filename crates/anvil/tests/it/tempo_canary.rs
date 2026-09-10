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
use alloy_primitives::{B256, b256};
use alloy_provider::{Provider, ext::DebugApi};
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::next_tempo_mainnet_rpc_endpoint;
use std::fmt;
use tempo_hardfork::TempoHardfork;

/// Base URL of the Tempo mainnet explorer, used to link replayed blocks and transactions.
const EXPLORER: &str = "https://explore.tempo.xyz";

/// Relay's solver settling a fill with a plain USDC.e `transfer`, sent after Relay dropped the
/// request id suffix that T11 rejected.
///
/// Block <https://explore.tempo.xyz/block/38917407>, transaction
/// <https://explore.tempo.xyz/receipt/0x061690e0b4378e2415de0ed6c8106edf8ef91b59ffe0ebe03b493ba02bff9645>.
const RELAY_USDCE_TRANSFER: PinnedTransaction = PinnedTransaction::new(
    38_917_407,
    b256!("0x061690e0b4378e2415de0ed6c8106edf8ef91b59ffe0ebe03b493ba02bff9645"),
);

/// The Relay fill reported during the T11 incident: a USDC.e `transfer` with the 32-byte request
/// id appended to the calldata, which the strict decoding T11 introduced rejected.
///
/// Block <https://explore.tempo.xyz/block/38915456>, transaction
/// <https://explore.tempo.xyz/receipt/0x834ad50aaceced3724b3b85c871be75f8ad6946d2b9335a59690848ffad72173>.
const RELAY_TRANSFER_WITH_REQUEST_ID: PinnedTransaction = PinnedTransaction::new(
    38_915_456,
    b256!("0x834ad50aaceced3724b3b85c871be75f8ad6946d2b9335a59690848ffad72173"),
);

/// Relay's solver paying out in pathUSD, the fee token every account holds.
///
/// Block <https://explore.tempo.xyz/block/38928039>, transaction
/// <https://explore.tempo.xyz/receipt/0x716cb37db2f216eb004fe0df1e211eb33a8083caa9fff13526b866549f44f409>.
const RELAY_PATH_USD_TRANSFER: PinnedTransaction = PinnedTransaction::new(
    38_928_039,
    b256!("0x716cb37db2f216eb004fe0df1e211eb33a8083caa9fff13526b866549f44f409"),
);

/// Relay's ERC20 router pulling the user's funds through a Permit2 signature and forwarding them
/// in one `permit2TransferAndMulticall`, the entry point of a cross-chain swap into Tempo.
///
/// Block <https://explore.tempo.xyz/block/38892542>, transaction
/// <https://explore.tempo.xyz/receipt/0x35826414c38c20aee8f78b2e7a55a2631ac12843db9828c376ee533d7491c184>.
const RELAY_ROUTER_PERMIT2_MULTICALL: PinnedTransaction = PinnedTransaction::new(
    38_892_542,
    b256!("0x35826414c38c20aee8f78b2e7a55a2631ac12843db9828c376ee533d7491c184"),
);

/// An end user's USDC.e `approve` carrying the ERC-8021 attribution suffix a frontend appends to
/// its calldata, which T11 rejected like Relay's request id.
///
/// Block <https://explore.tempo.xyz/block/38917602>, transaction
/// <https://explore.tempo.xyz/receipt/0xc966c5e310ff349d338ee73796fdf5d75b98b3193c639d2524752db4ac465327>.
const ERC8021_ATTRIBUTED_APPROVE: PinnedTransaction = PinnedTransaction::new(
    38_917_602,
    b256!("0xc966c5e310ff349d338ee73796fdf5d75b98b3193c639d2524752db4ac465327"),
);

/// A payout sender's Tempo AA transaction whose single call is a TIP20 `transfer`, paid for in
/// USDC.e.
///
/// Block <https://explore.tempo.xyz/block/38891824>, transaction
/// <https://explore.tempo.xyz/receipt/0x58e5fd7b57d925dec145fcdc3a492d5805f2ea8defbbf76f47ea127ac3b59b8f>.
const AA_TIP20_TRANSFER: PinnedTransaction = PinnedTransaction::new(
    38_891_824,
    b256!("0x58e5fd7b57d925dec145fcdc3a492d5805f2ea8defbbf76f47ea127ac3b59b8f"),
);

/// A sponsored Tempo AA transaction carrying a fee payer signature and a validity window, whose
/// call is a TIP20 `transferWithMemo`.
///
/// Block <https://explore.tempo.xyz/block/38891826>, transaction
/// <https://explore.tempo.xyz/receipt/0x125a827d32ff5b9416e7c29d63c43d1ce5e8464d4c26025b2246e965344ed541>.
const AA_SPONSORED_TRANSFER_WITH_MEMO: PinnedTransaction = PinnedTransaction::new(
    38_891_826,
    b256!("0x125a827d32ff5b9416e7c29d63c43d1ce5e8464d4c26025b2246e965344ed541"),
);

/// A pull payment: a Tempo AA transaction whose single call is a USDC.e `transferFrom` spending
/// another account's allowance, the pattern operators use to collect approved funds.
///
/// Block <https://explore.tempo.xyz/block/38928092>, transaction
/// <https://explore.tempo.xyz/receipt/0x2f329fd76706d8fefb3a888aa09c4ea490fe4200eef26fba687de7cf98cd7ee8>.
const AA_TIP20_TRANSFER_FROM: PinnedTransaction = PinnedTransaction::new(
    38_928_092,
    b256!("0x2f329fd76706d8fefb3a888aa09c4ea490fe4200eef26fba687de7cf98cd7ee8"),
);

/// An ERC-4337 bundler submitting `handleOps` to the v0.7 entry point.
///
/// Block <https://explore.tempo.xyz/block/38891903>, transaction
/// <https://explore.tempo.xyz/receipt/0x82941adeb79bbf6bbdf20165e1fce55d88e4f34ea0dc476f604df82aaf043442>.
const ERC4337_HANDLE_OPS: PinnedTransaction = PinnedTransaction::new(
    38_891_903,
    b256!("0x82941adeb79bbf6bbdf20165e1fce55d88e4f34ea0dc476f604df82aaf043442"),
);

/// A relayer executing an ERC-7821 batch on an EIP-7702 delegated account.
///
/// Block <https://explore.tempo.xyz/block/38891835>, transaction
/// <https://explore.tempo.xyz/receipt/0x140b39136cb50902bd6ebeed2b6485a4aca20868542ae85be83d5b86a332d4d7>.
const ERC7821_EXECUTE: PinnedTransaction = PinnedTransaction::new(
    38_891_835,
    b256!("0x140b39136cb50902bd6ebeed2b6485a4aca20868542ae85be83d5b86a332d4d7"),
);

/// Relay's fills must keep replaying under the newest hardfork, see [`RELAY_USDCE_TRANSFER`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_usdce_transfer() {
    replay(RELAY_USDCE_TRANSFER, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// The T11 incident fill, see [`RELAY_TRANSFER_WITH_REQUEST_ID`], replayed under the hardforks
/// around the change: T10 accepts the trailing request id, T11 rejects it exactly as mainnet did,
/// and the latest hardfork accepts it again.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_transfer_trailing_bytes_across_hardforks() {
    assert_trailing_bytes_regression(RELAY_TRANSFER_WITH_REQUEST_ID).await;
}

/// Relay's pathUSD payouts must keep replaying, see [`RELAY_PATH_USD_TRANSFER`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_path_usd_transfer() {
    replay(RELAY_PATH_USD_TRANSFER, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// Relay's router swaps must keep replaying, see [`RELAY_ROUTER_PERMIT2_MULTICALL`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_router_permit2_multicall() {
    replay(RELAY_ROUTER_PERMIT2_MULTICALL, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// The ERC-8021 approval T11 rejected, see [`ERC8021_ATTRIBUTED_APPROVE`], replayed under the
/// hardforks around the change like the Relay fill.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_erc8021_attribution_suffix_across_hardforks() {
    assert_trailing_bytes_regression(ERC8021_ATTRIBUTED_APPROVE).await;
}

/// AA payouts must keep replaying, see [`AA_TIP20_TRANSFER`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_aa_tip20_transfer() {
    replay(AA_TIP20_TRANSFER, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// Sponsored AA transfers must keep replaying, see [`AA_SPONSORED_TRANSFER_WITH_MEMO`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_aa_sponsored_transfer_with_memo() {
    replay(AA_SPONSORED_TRANSFER_WITH_MEMO, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// Allowance pulls must keep replaying, see [`AA_TIP20_TRANSFER_FROM`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_aa_tip20_transfer_from() {
    replay(AA_TIP20_TRANSFER_FROM, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// ERC-4337 bundles must keep replaying, see [`ERC4337_HANDLE_OPS`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_erc4337_bundler_handle_ops() {
    replay(ERC4337_HANDLE_OPS, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Exact);
}

/// ERC-7821 batches on delegated accounts must keep replaying, see [`ERC7821_EXECUTE`].
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_erc7821_execute_on_delegated_account() {
    replay(ERC7821_EXECUTE, TempoHardfork::latest()).await.assert_matches_mainnet(GasCheck::Exact);
}

/// Replays a precompile call with trailing calldata bytes that failed on mainnet under T11: T10
/// still accepts the bytes, T11 reproduces the mainnet failure, and the latest hardfork accepts
/// them again.
async fn assert_trailing_bytes_regression(pinned: PinnedTransaction) {
    let before = replay(pinned, TempoHardfork::T10).await;
    before.assert_all_succeed();

    let regression = replay(pinned, TempoHardfork::T11).await;
    regression.assert_matches_mainnet(GasCheck::Exact);
    assert!(
        regression.transactions.iter().all(|tx| !tx.mainnet_success),
        "{regression}: expected the block to hold the failed call"
    );

    let fixed = replay(pinned, TempoHardfork::latest()).await;
    fixed.assert_all_succeed();
}

/// A mainnet transaction a canary is pinned to, replayed together with the rest of its block.
#[derive(Clone, Copy)]
struct PinnedTransaction {
    /// The block holding the transaction; the fork starts at its parent.
    block: u64,
    /// The transaction the canary is about, which must be part of the replayed block.
    hash: B256,
}

impl PinnedTransaction {
    const fn new(block: u64, hash: B256) -> Self {
        Self { block, hash }
    }
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
                "{self}: {tx} diverged from mainnet: success {} vs {} (gas {} vs {})",
                tx.success, tx.mainnet_success, tx.gas_used, tx.mainnet_gas_used
            );
            match gas {
                GasCheck::Exact => assert_eq!(
                    tx.gas_used, tx.mainnet_gas_used,
                    "{self}: {tx} used different gas than on mainnet"
                ),
                GasCheck::Within(tolerance) => assert!(
                    tx.gas_used.abs_diff(tx.mainnet_gas_used) <= tolerance,
                    "{self}: {tx} used {} gas, mainnet used {}, tolerance {tolerance}",
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
                "{self}: {tx} failed (gas {}, mainnet success {} with gas {})",
                tx.gas_used, tx.mainnet_success, tx.mainnet_gas_used
            );
        }
    }
}

impl fmt::Display for ReplayedBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "block {} ({EXPLORER}/block/{}) under {}",
            self.number, self.number, self.hardfork
        )
    }
}

impl fmt::Display for ReplayedTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{EXPLORER}/receipt/{}", self.hash)
    }
}

/// Forks Tempo mainnet at the parent of the pinned block, forces `hardfork`, and replays the
/// block's raw transactions in order at the block's timestamp, one local block each.
async fn replay(pinned: PinnedTransaction, hardfork: TempoHardfork) -> ReplayedBlock {
    let number = pinned.block;
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
    assert_eq!(receipts.len(), block.transactions.len(), "block {number} receipts");
    assert!(
        receipts.iter().any(|receipt| receipt.transaction_hash() == pinned.hash),
        "block {number} does not hold the pinned transaction {EXPLORER}/receipt/{}",
        pinned.hash
    );

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
            panic!("block {number} under {hardfork}: {EXPLORER}/receipt/{hash} rejected: {err}")
        });
        assert_eq!(sent, hash, "block {number}: raw transaction decoded to a different hash");
        api.mine_one().await.unwrap();

        let local = api.transaction_receipt(hash).await.unwrap().unwrap_or_else(|| {
            panic!("block {number} under {hardfork}: {EXPLORER}/receipt/{hash} was not mined")
        });
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
