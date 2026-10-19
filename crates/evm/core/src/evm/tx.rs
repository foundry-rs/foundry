//! Network-native transactions.
//!
//! Transactions stay in their network's Alloy form until execution: tools keep the selected
//! [`TransactionRequest`](alloy_network::Network::TransactionRequest) or signed
//! [`TxEnvelope`](alloy_network::Network::TxEnvelope), mutate common fields through Alloy's
//! [`TransactionBuilder`](alloy_network::TransactionBuilder), and convert once, right before
//! execution, into the family's evm2 transaction with
//! [`FoundryEvmNetwork`]. Signed, resumed and replayed transactions
//! convert from exactly the payload that was signed; nothing is rebuilt from Ethereum-shaped
//! fields.

use super::{ChainFor, FoundryEvmExt, FoundryEvmNetwork, TxFor};
use alloy_consensus::{
    BlockHeader, EthereumTxEnvelope, TxEip4844Variant, Typed2718, transaction::Recovered,
};
use alloy_network::{AnyNetwork, AnyRpcBlock, AnyRpcTransaction};
use alloy_primitives::Signature;
use alloy_provider::Provider;
use alloy_rpc_types::BlockTransactions;
use evm2::ethereum::{LazyTxEip7702, TxEnvelope};
use eyre::WrapErr;
use foundry_evm_networks::celo::CELO_DYNAMIC_FEE_TX_TYPE;

/// What an execution-time transaction is priced and validated as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TxPayload {
    /// The transaction's own signed encoding, as broadcast or replayed.
    Signed,
    /// A simulation of an unsigned request.
    ///
    /// Networks that price fees over the encoded transaction, such as the OP-stack L1 data fee,
    /// charge nothing for it, and networks with simulation-only metadata, such as Tempo, mark the
    /// transaction as simulated.
    #[default]
    Simulated,
}

impl TxPayload {
    /// Returns whether this is a simulation of an unsigned request.
    pub const fn is_simulated(self) -> bool {
        matches!(self, Self::Simulated)
    }
}

/// Placeholder signature used to put an unsigned request into its network's envelope.
///
/// The sender is always supplied separately, so the signature is never recovered.
pub fn placeholder_signature() -> Signature {
    Signature::test_signature()
}

/// Converts a signed Ethereum envelope into evm2's Ethereum transaction.
///
/// Signed EIP-7702 authorizations are kept as signed, so their authorities are recovered during
/// execution exactly as on chain.
pub fn eth_tx(envelope: EthereumTxEnvelope<TxEip4844Variant>) -> TxEnvelope {
    match envelope {
        EthereumTxEnvelope::Legacy(tx) => TxEnvelope::Legacy(tx.strip_signature()),
        EthereumTxEnvelope::Eip2930(tx) => TxEnvelope::Eip2930(tx.strip_signature()),
        EthereumTxEnvelope::Eip1559(tx) => TxEnvelope::Eip1559(tx.strip_signature()),
        EthereumTxEnvelope::Eip4844(tx) => TxEnvelope::Eip4844(tx.strip_signature()),
        EthereumTxEnvelope::Eip7702(tx) => {
            TxEnvelope::Eip7702(LazyTxEip7702::from_recovered_authorizations(tx.strip_signature()))
        }
    }
}

/// Returns the Ethereum envelope of a transaction fetched over RPC.
///
/// Celo's CIP-64 fee-currency transactions have no evm2 representation and fail explicitly.
pub fn eth_envelope_from_any_rpc(
    tx: &AnyRpcTransaction,
    network: &str,
) -> eyre::Result<alloy_consensus::TxEnvelope> {
    if let Some(envelope) = tx.as_envelope() {
        return Ok(envelope.clone());
    }
    if tx.inner.inner.ty() == CELO_DYNAMIC_FEE_TX_TYPE {
        eyre::bail!(
            "cannot replay Celo CIP-64 transaction {}: evm2 has no fee-currency transaction type",
            alloy_network::TransactionResponse::tx_hash(tx)
        );
    }
    Err(super::unknown_transaction_type(tx, network))
}

/// Transactions of a block and its two ancestors, for network state that depends on the block
/// position of a transaction (see [`FoundryEvmExt::for_block`]).
#[derive(Debug)]
pub struct BlockContext<FEN: FoundryEvmNetwork> {
    grandparent: Vec<Recovered<TxFor<FEN>>>,
    parent: Vec<Recovered<TxFor<FEN>>>,
    current: Vec<Recovered<TxFor<FEN>>>,
}

impl<FEN: FoundryEvmNetwork> Clone for BlockContext<FEN> {
    fn clone(&self) -> Self {
        Self {
            grandparent: self.grandparent.clone(),
            parent: self.parent.clone(),
            current: self.current.clone(),
        }
    }
}

impl<FEN: FoundryEvmNetwork> BlockContext<FEN> {
    /// Creates block context from grandparent, parent, and current block transactions.
    pub const fn new(
        grandparent: Vec<Recovered<TxFor<FEN>>>,
        parent: Vec<Recovered<TxFor<FEN>>>,
        current: Vec<Recovered<TxFor<FEN>>>,
    ) -> Self {
        Self { grandparent, parent, current }
    }

    /// Fetches the ancestors of `block` and converts the transactions of all three blocks.
    pub async fn fetch<P: Provider<AnyNetwork>>(
        provider: &P,
        block: &AnyRpcBlock,
    ) -> eyre::Result<Self> {
        let parent = fetch_parent(provider, block).await?;
        let grandparent = match &parent {
            Some(parent) => fetch_parent(provider, parent).await?,
            None => None,
        };
        Ok(Self::new(
            grandparent.as_ref().map(block_txs::<FEN>).transpose()?.unwrap_or_default(),
            parent.as_ref().map(block_txs::<FEN>).transpose()?.unwrap_or_default(),
            block_txs::<FEN>(block)?,
        ))
    }

    /// Prepares `ext` for the transaction at `index` in the current block.
    pub fn prepare(&self, ext: &mut ChainFor<FEN>, index: usize) {
        ext.for_block(&self.grandparent, &self.parent, &self.current, index);
    }

    /// Prepares `ext` for `tx` appended after the current block's transactions so far.
    pub fn prepare_next(&self, ext: &mut ChainFor<FEN>, tx: &Recovered<TxFor<FEN>>) {
        let mut current = self.current.clone();
        let index = current.len();
        current.push(tx.clone());
        ext.for_block(&self.grandparent, &self.parent, &current, index);
    }

    /// Returns a cursor positioned immediately before `index` in the current block.
    pub fn before_transaction(mut self, index: usize) -> eyre::Result<Self> {
        eyre::ensure!(
            index <= self.current.len(),
            "transaction index {index} exceeds block transaction count {}",
            self.current.len()
        );
        self.current.truncate(index);
        Ok(self)
    }

    /// Records a committed transaction at the cursor's current block position.
    pub fn record_transaction(&mut self, tx: Recovered<TxFor<FEN>>) {
        self.current.push(tx);
    }

    /// Advances the cursor to the start of the next block.
    pub fn advance_block(&mut self) {
        self.grandparent = std::mem::take(&mut self.parent);
        self.parent = std::mem::take(&mut self.current);
    }
}

/// Fetches the parent of `block`, or `None` for genesis.
async fn fetch_parent<P: Provider<AnyNetwork>>(
    provider: &P,
    block: &AnyRpcBlock,
) -> eyre::Result<Option<AnyRpcBlock>> {
    let parent_hash = block.header.parent_hash();
    if parent_hash.is_zero() {
        return Ok(None);
    }
    let parent = provider
        .get_block_by_hash(parent_hash)
        .full()
        .await
        .wrap_err_with(|| format!("failed to fetch ancestor block {parent_hash}"))?
        .ok_or_else(|| eyre::eyre!("ancestor block {parent_hash} not found"))?;
    eyre::ensure!(
        parent.header.hash == parent_hash
            && Some(parent.header.number()) == block.header.number().checked_sub(1),
        "ancestor block {parent_hash} does not match its child"
    );
    Ok(Some(parent))
}

/// Converts every transaction of a full RPC block.
pub fn block_txs<FEN: FoundryEvmNetwork>(
    block: &AnyRpcBlock,
) -> eyre::Result<Vec<Recovered<TxFor<FEN>>>> {
    let BlockTransactions::Full(transactions) = &block.transactions else {
        eyre::bail!("block {} does not contain full transactions", block.header.number());
    };
    transactions.iter().map(FEN::tx_from_any_rpc).collect()
}
