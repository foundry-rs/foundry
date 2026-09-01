//! Fork tests for popular chains that serve RPC responses differing from Ethereum mainnet.
//!
//! Every chain runs through [`assert_can_fork`], so chain specific response shapes such as Arbitrum
//! Orbit system transactions, OP-stack deposits, or vendor specific header fields are covered by
//! the same assertions.
//!
//! These tests fork from public endpoints and are therefore prefixed with `flaky_` so they only run
//! in the nightly flaky job. A request the endpoint fails to serve skips the test; a response anvil
//! cannot decode still fails it, because covering those response shapes is the point.

use alloy_chains::NamedChain;
use alloy_eips::Typed2718;
use alloy_network::{ReceiptResponse, TransactionBuilder, TransactionResponse};
use alloy_primitives::{Address, B256};
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest, state::EvmOverrides};
use alloy_transport::{RpcError, TransportError};
use anvil::{
    NodeConfig,
    eth::{EthApi, error::BlockchainError},
    try_spawn,
};
use foundry_primitives::FoundryNetwork;
use foundry_test_utils::rpc::next_rpc_endpoint;
use std::fmt::{Debug, Display};

/// Highest EIP-2718 transaction type every chain is expected to serve in the standard shape.
///
/// Chain specific transactions above this range, such as Arbitrum's `0x6a`, Celo's CIP-64 `0x7b`,
/// or OP-stack deposits, are not part of the surface these tests assert on.
const MAX_STANDARD_TX_TYPE: u8 = 4;

/// Number of blocks scanned back from the fork head when looking for a transaction to round-trip.
const TRANSACTION_LOOKBACK: u64 = 8;

/// Fragments public endpoints use to report that they refused to serve a request.
///
/// Every gateway spells this differently and there is no shared error code, so the rendered error
/// is matched instead.
const SERVICE_REFUSALS: &[&str] =
    &["rate limit", "too many requests", "usage limit", "limit exceeded", "quota", "capacity"];

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_arbitrum() {
    assert_can_fork(NamedChain::Arbitrum).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_base() {
    assert_can_fork(NamedChain::Base).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_celo() {
    assert_can_fork(NamedChain::Celo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_gnosis() {
    assert_can_fork(NamedChain::Gnosis).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_hyperliquid() {
    assert_can_fork(NamedChain::Hyperliquid).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_optimism() {
    assert_can_fork(NamedChain::Optimism).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_polygon() {
    assert_can_fork(NamedChain::Polygon).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_fork_robinhood() {
    assert_can_fork(NamedChain::Robinhood).await;
}

/// Forks `chain` at its latest block and exercises the RPC surface fork mode depends on.
async fn assert_can_fork(chain: NamedChain) {
    // A `None` result means the endpoint failed to serve a request; the reason is already reported.
    let _ = fork_and_assert(chain).await;
}

/// Runs the fork assertions for `chain`, returning `None` once the endpoint stopped serving.
async fn fork_and_assert(chain: NamedChain) -> Option<()> {
    // Both forks below must reach the same provider: the Arbitrum endpoints rotate and their heads
    // are a few blocks apart.
    let fork_rpc = next_rpc_endpoint(chain);
    let config = NodeConfig::test().with_eth_rpc_url(Some(fork_rpc.clone()));
    let genesis_balance = config.genesis_balance;
    let (api, handle) = fork_ok(chain, "fork setup", try_spawn(config).await)?;

    assert_eq!(api.chain_id(), chain as u64, "{chain:?} fork adopted the wrong chain id");

    fork_ok(
        chain,
        "eth_call",
        api.call(
            TransactionRequest::default().with_to(Address::ZERO).into(),
            None,
            EvmOverrides::default(),
        )
        .await,
    )?;

    let fork_block = api.block_number().unwrap().to::<u64>();
    if fork_block == 0 {
        // Public endpoints occasionally report block zero while they are unhealthy.
        return skip(chain, "fork head", "endpoint reported block zero");
    }

    for wallet in handle.dev_wallets() {
        let balance = api.balance(wallet.address(), None).await.unwrap();
        assert_eq!(balance, genesis_balance, "{chain:?} fork did not fund the dev accounts");
    }

    let (block_number, tx_hash) = standard_transaction(chain, &api, fork_block).await?;

    // Selecting the transaction cached the whole block on `api`, which would serve the lookups
    // below without ever calling the endpoint. Use a fork that has not read that block instead,
    // so both responses are decoded as the endpoint returns them.
    let lookup_config = NodeConfig::test()
        .with_eth_rpc_url(Some(fork_rpc))
        .with_fork_block_number(Some(fork_block));
    let (lookup_api, _) = fork_ok(chain, "transaction fork setup", try_spawn(lookup_config).await)?;

    let tx =
        fork_ok(chain, "eth_getTransactionByHash", lookup_api.transaction_by_hash(tx_hash).await)?
            .unwrap_or_else(|| panic!("{chain:?} fork lost transaction {tx_hash}"));
    assert_eq!(tx.tx_hash(), tx_hash);
    assert_eq!(tx.block_number, Some(block_number));

    let receipt =
        fork_ok(chain, "eth_getTransactionReceipt", lookup_api.transaction_receipt(tx_hash).await)?
            .unwrap_or_else(|| panic!("{chain:?} fork lost the receipt for {tx_hash}"));
    assert_eq!(receipt.transaction_hash(), tx_hash);
    assert_eq!(receipt.block_number(), Some(block_number));

    // Local mining continues from the forked head.
    api.mine_one().await.unwrap();
    assert_eq!(api.block_number().unwrap().to::<u64>(), fork_block + 1);

    Some(())
}

/// Returns the block number and hash of a transaction with a standard EIP-2718 type, searching
/// backwards from the fork head.
async fn standard_transaction(
    chain: NamedChain,
    api: &EthApi<FoundryNetwork>,
    head: u64,
) -> Option<(u64, B256)> {
    for number in (head.saturating_sub(TRANSACTION_LOOKBACK)..=head).rev() {
        let block = fork_ok(
            chain,
            "eth_getBlockByNumber",
            api.block_by_number_full(BlockNumberOrTag::Number(number)).await,
        )?;
        let Some(block) = block else { continue };
        assert_eq!(block.header.number, number, "{chain:?} fork returned the wrong block");

        if let Some(transactions) = block.transactions.as_transactions()
            && let Some(tx) = transactions.iter().find(|tx| tx.inner.ty() <= MAX_STANDARD_TX_TYPE)
        {
            return Some((number, tx.tx_hash()));
        }
    }

    skip(chain, "transaction lookup", "no standard transaction near the fork head")
}

/// Unwraps a fork response, returning `None` once the endpoint failed to serve the request.
fn fork_ok<T, E: ForkError + Debug>(
    chain: NamedChain,
    request: &str,
    result: Result<T, E>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(err) if is_endpoint_unavailable(&err) => skip(chain, request, err.render()),
        Err(err) => panic!("{chain:?} fork: {request} failed: {err:?}"),
    }
}

/// Reports why a chain's fork test stopped early, so skipped runs stay visible in CI logs.
///
/// The flaky nextest profile keeps the output of these tests on success so the report is not
/// swallowed. Chains are named by their variant rather than their canonical name, which does not
/// always match, so the line points at the test that produced it.
#[expect(clippy::disallowed_macros)]
fn skip<T>(chain: NamedChain, request: &str, reason: impl Display) -> Option<T> {
    eprintln!("skipping {chain:?} fork test: {request}: {reason}");
    None
}

/// An error raised while exercising a public fork endpoint.
trait ForkError {
    /// Returns the transport error the fork endpoint reported, if this error still carries one.
    fn transport_error(&self) -> Option<&TransportError>;

    /// Renders the error together with its causes.
    fn render(&self) -> String;
}

impl ForkError for eyre::Report {
    fn transport_error(&self) -> Option<&TransportError> {
        self.downcast_ref::<TransportError>()
    }

    fn render(&self) -> String {
        format!("{self:#}")
    }
}

impl ForkError for BlockchainError {
    fn transport_error(&self) -> Option<&TransportError> {
        match self {
            Self::AlloyForkProvider(err) => Some(err),
            _ => None,
        }
    }

    fn render(&self) -> String {
        self.to_string()
    }
}

/// Returns whether the fork endpoint failed to serve the request at all.
///
/// Connection failures and refusals mean the endpoint never answered, which public endpoints do
/// often enough that failing on them would turn these tests into noise. Everything else stays a
/// failure, in particular a response anvil cannot decode.
fn is_endpoint_unavailable(err: &impl ForkError) -> bool {
    match err.transport_error() {
        // The endpoint never answered.
        Some(RpcError::Transport(_)) => return true,
        // A response anvil could not decode is the shape mismatch these tests exist to catch.
        Some(RpcError::DeserError { .. }) => return false,
        // Fork state fetches render the endpoint's response into their message instead of keeping
        // the transport error, so refusals are matched on the rendered error.
        _ => {}
    }

    let rendered = err.render().to_lowercase();
    SERVICE_REFUSALS.iter().any(|refusal| rendered.contains(refusal))
}
