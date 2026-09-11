//! Canary tests replaying Tempo mainnet transactions under the newest hardfork anvil knows.
//!
//! Every test forks Tempo mainnet at the parent of a pinned transaction's block, forces the
//! hardfork to [`TempoHardfork::latest`], and re-submits the transaction's original signed bytes
//! at the block's timestamp. The local receipt must reproduce mainnet: the same status, the same
//! logs, and, unless a case documents a known change, the same gas.
//!
//! The pinned transactions come from the services that keep Tempo busy: Relay's solver and
//! router, Tempo AA payout senders, an ERC-4337 bundler, an ERC-7821 relayer for EIP-7702
//! accounts, and end users sending through a frontend that appends an ERC-8021 attribution
//! suffix. T11 activated strict ABI decoding for precompile calls and broke the first and last of
//! those on mainnet, because both append bytes to TIP20 `transfer` and `approve` calldata.
//! Replaying the same transactions under T11 before it activated would have shown that, which is
//! what these tests do for every hardfork the pinned `tempo` revision adds; see
//! <https://github.com/tempoxyz/tempo/pull/7598> for the fix that ships in T12.
//!
//! Each pinned transaction is alone in its block, which is asserted, so replaying it on a fork of
//! the parent reproduces the state it executed against. Gas is compared exactly, so the pinned
//! blocks must have been executed under the hardfork that is active on mainnet: a hardfork may
//! change gas accounting, and T11 did for precompile calldata. When the newest hardfork changes
//! it again, record the exact change for the affected case with [`GasCheck::Offset`] and the
//! reason next to it, then re-pin the transaction past the activation once it is live and restore
//! the exact check.
//!
//! The upstream defaults to the public endpoint and honours `TEMPO_MAINNET_RPC_URL`, see
//! [`next_tempo_mainnet_rpc_endpoint`]. The canaries run in a single-threaded nextest group
//! because the public endpoint rate limits concurrent replays.

use crate::utils::http_provider;
use alloy_consensus::Transaction;
use alloy_network::{ReceiptResponse, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, b256};
use alloy_provider::{Provider, ext::DebugApi};
use alloy_rpc_types::{BlockId, BlockNumberOrTag, Log};
use anvil::{NodeConfig, spawn};
use foundry_test_utils::rpc::next_tempo_mainnet_rpc_endpoint;
use std::fmt;
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::TIP_FEE_MANAGER_ADDRESS;

/// Base URL of the Tempo mainnet explorer, used to link replayed blocks and transactions.
const EXPLORER: &str = "https://explore.tempo.xyz";

/// Topic of the TIP20 `Transfer(address,address,uint256)` event.
const TRANSFER_TOPIC: B256 =
    b256!("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");

/// Topic of the TIP20 `Approval(address,address,uint256)` event.
const APPROVAL_TOPIC: B256 =
    b256!("0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925");

/// Relay's solver settling a fill with a plain USDC.e `transfer`, sent after Relay dropped the
/// request id suffix that T11 rejected.
///
/// Block <https://explore.tempo.xyz/block/38917407>, transaction
/// <https://explore.tempo.xyz/receipt/0x061690e0b4378e2415de0ed6c8106edf8ef91b59ffe0ebe03b493ba02bff9645>.
const RELAY_USDCE_TRANSFER: PinnedTransaction = PinnedTransaction::new(
    38_917_407,
    b256!("0x061690e0b4378e2415de0ed6c8106edf8ef91b59ffe0ebe03b493ba02bff9645"),
);

/// A Relay fill executed under T10, before the incident: a USDC.e `transfer` with the 32-byte
/// request id appended to the calldata, which T10 accepted.
///
/// Block <https://explore.tempo.xyz/block/38890775>, transaction
/// <https://explore.tempo.xyz/receipt/0x70f6e31600875e6202c59862f1c9e47f8471fdbb347a9034e0b92a7019bec657>.
const RELAY_T10_TRANSFER_WITH_REQUEST_ID: PinnedTransaction = PinnedTransaction::new(
    38_890_775,
    b256!("0x70f6e31600875e6202c59862f1c9e47f8471fdbb347a9034e0b92a7019bec657"),
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

/// The T10-era fill, see [`RELAY_T10_TRANSFER_WITH_REQUEST_ID`], must reproduce mainnet exactly
/// under T10 and keep succeeding under the latest hardfork.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_canary_fork_relay_t10_transfer_with_request_id() {
    replay(RELAY_T10_TRANSFER_WITH_REQUEST_ID, TempoHardfork::T10)
        .await
        .assert_matches_mainnet(GasCheck::Exact);

    // T11 raised the gas of precompile calldata, which charges this call 96 gas more than mainnet
    // did under T10. The change is permanent for a T10-era transaction, so it is expected here.
    replay(RELAY_T10_TRANSFER_WITH_REQUEST_ID, TempoHardfork::latest())
        .await
        .assert_matches_mainnet(GasCheck::Offset(96));
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

/// Replays a TIP20 `transfer` or `approve` with trailing calldata bytes that failed on mainnet
/// under T11: T10 still accepts the bytes and applies the call, T11 reproduces the mainnet
/// failure, and the latest hardfork applies the call again.
async fn assert_trailing_bytes_regression(pinned: PinnedTransaction) {
    replay(pinned, TempoHardfork::T10).await.assert_applies_tip20_call();

    let regression = replay(pinned, TempoHardfork::T11).await;
    regression.assert_matches_mainnet(GasCheck::Exact);
    assert!(!regression.mainnet_success, "{regression}: expected the mainnet call to have failed");

    replay(pinned, TempoHardfork::latest()).await.assert_applies_tip20_call();
}

/// A mainnet transaction a canary is pinned to, alone in its block.
#[derive(Clone, Copy)]
struct PinnedTransaction {
    /// The block holding the transaction; the fork starts at its parent.
    block: u64,
    /// The transaction's hash, which must be the only one in the block.
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
    /// The replay must use exactly this much more gas than mainnet recorded, or less when
    /// negative; the known effect of a hardfork on the pinned transaction.
    Offset(i64),
}

/// The pinned transaction replayed on a fork of its parent block under a forced hardfork, next
/// to the outcome mainnet recorded for it.
struct Replayed {
    pinned: PinnedTransaction,
    hardfork: TempoHardfork,
    sender: Address,
    to: Option<Address>,
    input: Bytes,
    success: bool,
    gas_used: u64,
    logs: Vec<LogRecord>,
    mainnet_success: bool,
    mainnet_gas_used: u64,
    mainnet_logs: Vec<LogRecord>,
}

/// The parts of a log that execution determines.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LogRecord {
    address: Address,
    topics: Vec<B256>,
    data: Bytes,
}

impl LogRecord {
    fn new(log: &Log) -> Self {
        Self {
            address: log.address(),
            topics: log.topics().to_vec(),
            data: log.data().data.clone(),
        }
    }

    /// Returns whether this is the fee payment Tempo appends to every receipt: a TIP20 transfer
    /// from the fee payer to the fee manager.
    fn is_fee_payment(&self) -> bool {
        self.topics.first() == Some(&TRANSFER_TOPIC)
            && self.topics.get(2) == Some(&TIP_FEE_MANAGER_ADDRESS.into_word())
    }
}

impl Replayed {
    /// Asserts the replay reproduced the mainnet status, the mainnet logs, and, per `gas`, the
    /// mainnet gas usage.
    ///
    /// The fee payment log depends on the gas used, so it is only compared when the gas must
    /// match exactly.
    fn assert_matches_mainnet(&self, gas: GasCheck) {
        assert_eq!(
            self.success, self.mainnet_success,
            "{self}: status diverged from mainnet: success {} vs {} (gas {} vs {})",
            self.success, self.mainnet_success, self.gas_used, self.mainnet_gas_used
        );
        match gas {
            GasCheck::Exact => {
                assert_eq!(
                    self.gas_used, self.mainnet_gas_used,
                    "{self}: used different gas than on mainnet"
                );
                assert_eq!(self.logs, self.mainnet_logs, "{self}: logs diverged from mainnet");
            }
            GasCheck::Offset(offset) => {
                assert_eq!(
                    i128::from(self.gas_used) - i128::from(self.mainnet_gas_used),
                    i128::from(offset),
                    "{self}: used {} gas, mainnet used {}, expected offset {offset}",
                    self.gas_used,
                    self.mainnet_gas_used
                );
                assert_eq!(
                    self.effects(),
                    Self::effects_of(&self.mainnet_logs),
                    "{self}: logs diverged from mainnet"
                );
            }
        }
    }

    /// Asserts the replay succeeded and applied the pinned TIP20 `transfer` or `approve`: its
    /// only effect is the matching `Transfer` or `Approval` event, whatever mainnet recorded.
    fn assert_applies_tip20_call(&self) {
        assert!(self.success, "{self}: failed with gas {}", self.gas_used);
        let (selector, args) = self.input.split_at(4);
        let (topic, name) = match selector {
            [0xa9, 0x05, 0x9c, 0xbb] => (TRANSFER_TOPIC, "transfer"),
            [0x09, 0x5e, 0xa7, 0xb3] => (APPROVAL_TOPIC, "approve"),
            _ => panic!("{self}: not a TIP20 transfer or approve"),
        };
        let expected = LogRecord {
            address: self.to.unwrap_or_else(|| panic!("{self}: not a call")),
            topics: vec![topic, self.sender.into_word(), B256::from_slice(&args[..32])],
            data: Bytes::copy_from_slice(&args[32..64]),
        };
        assert_eq!(self.effects(), vec![expected], "{self}: {name} was not applied as encoded");
    }

    /// Returns the logs the replay emitted apart from the fee payment.
    fn effects(&self) -> Vec<LogRecord> {
        Self::effects_of(&self.logs)
    }

    fn effects_of(logs: &[LogRecord]) -> Vec<LogRecord> {
        logs.iter().filter(|log| !log.is_fee_payment()).cloned().collect()
    }
}

impl fmt::Display for Replayed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{EXPLORER}/receipt/{} in block {} ({EXPLORER}/block/{}) under {}",
            self.pinned.hash, self.pinned.block, self.pinned.block, self.hardfork
        )
    }
}

/// Forks Tempo mainnet at the parent of the pinned transaction's block, forces `hardfork`, and
/// replays the transaction's signed bytes at the block's timestamp.
async fn replay(pinned: PinnedTransaction, hardfork: TempoHardfork) -> Replayed {
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
    let [receipt] = receipts.as_slice() else {
        panic!(
            "block {number} holds {} transactions, expected only {EXPLORER}/receipt/{}",
            receipts.len(),
            pinned.hash
        );
    };
    assert_eq!(
        receipt.transaction_hash(),
        pinned.hash,
        "block {number} does not hold the pinned transaction {EXPLORER}/receipt/{}",
        pinned.hash
    );
    let tx = mainnet
        .get_transaction_by_hash(pinned.hash)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("tempo mainnet transaction {} not found", pinned.hash));
    let raw = mainnet.debug_get_raw_transaction(pinned.hash).await.unwrap();

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

    // Tempo blocks are sub-second apart, so the parent may share the timestamp; anvil accepts an
    // equal one. Setting it before submitting also validates time bounds against it.
    api.evm_set_next_block_timestamp(block.header.timestamp).unwrap();
    let sent = api.send_raw_transaction(raw).await.unwrap_or_else(|err| {
        panic!(
            "block {number} under {hardfork}: {EXPLORER}/receipt/{} rejected: {err}",
            pinned.hash
        )
    });
    assert_eq!(sent, pinned.hash, "block {number}: raw transaction decoded to a different hash");
    api.mine_one().await.unwrap();

    let local = api.transaction_receipt(pinned.hash).await.unwrap().unwrap_or_else(|| {
        panic!("block {number} under {hardfork}: {EXPLORER}/receipt/{} was not mined", pinned.hash)
    });
    Replayed {
        pinned,
        hardfork,
        sender: tx.from(),
        to: tx.to(),
        input: tx.input().clone(),
        success: local.status(),
        gas_used: local.gas_used(),
        logs: local.0.inner.inner.logs().iter().map(LogRecord::new).collect(),
        mainnet_success: receipt.status(),
        mainnet_gas_used: receipt.gas_used(),
        mainnet_logs: receipt.inner.inner.logs().iter().map(LogRecord::new).collect(),
    }
}
