//! Network-native transactions.
//!
//! Transactions stay in their network's Alloy form until execution: tools keep the selected
//! [`TransactionRequest`](alloy_network::Network::TransactionRequest) or signed
//! [`TxEnvelope`](alloy_network::Network::TxEnvelope), mutate common fields through Alloy's
//! [`TransactionBuilder`], and convert once, right before execution, into the family's evm2
//! transaction with [`FoundryTxConversion`]. Signed, resumed and replayed transactions convert
//! from exactly the payload that was signed; nothing is rebuilt from Ethereum-shaped fields.

use super::{ChainFor, FoundryEvmExt, FoundryEvmNetwork, TxFor, TypesFor};
use alloy_chains::NamedChain;
use alloy_consensus::{
    BlockHeader, EthereumTxEnvelope, EthereumTypedTransaction, SignableTransaction, Signed,
    Transaction, TxEip1559, TxEip4844Variant, Typed2718,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_eips::Decodable2718;
use alloy_network::{
    AnyNetwork, AnyRpcBlock, AnyRpcTransaction, AnyTxEnvelope, Network, NetworkTransactionBuilder,
    TransactionBuilder, TransactionResponse,
};
use alloy_primitives::{Address, Signature};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockTransactions, TransactionRequest};
use evm2::{
    EvmTypesHost,
    ethereum::{LazyTxEip7702, TxEnvelope},
};
use eyre::WrapErr;
use foundry_evm_networks::celo::CELO_DYNAMIC_FEE_TX_TYPE;

/// Converts an Alloy network's native transactions into an evm2 execution family's transactions.
///
/// The family, rather than the transaction type alone, owns conversion: families can share a
/// transaction representation while retaining different conversion policies and diagnostics.
pub trait FoundryTxConversion<N: Network>: EvmTypesHost {
    /// Converts a network envelope sent by `sender`, once, right before execution.
    fn evm_tx(
        envelope: N::TxEnvelope,
        sender: Address,
        payload: TxPayload,
    ) -> eyre::Result<Recovered<Self::Tx>>;

    /// Converts a complete unsigned request into a simulated transaction.
    ///
    /// The request must name its sender and carry every field its transaction type needs; Foundry
    /// fills defaults through [`TransactionBuilder`] before converting, never afterwards.
    fn tx_from_request(request: N::TransactionRequest) -> eyre::Result<Recovered<Self::Tx>>
    where
        N::TransactionRequest: NetworkTransactionBuilder<N>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
    {
        let sender =
            request.from().ok_or_else(|| eyre::eyre!("transaction request has no sender"))?;
        let unsigned =
            request.build_unsigned().map_err(|err| eyre::eyre!(err.error.to_string()))?;
        let envelope = N::TxEnvelope::from(unsigned.into_signed(placeholder_signature()));
        Self::evm_tx(envelope, sender, TxPayload::Simulated)
    }

    /// Converts a signed envelope into a transaction sent by its recovered signer.
    fn tx_from_signed(envelope: N::TxEnvelope) -> eyre::Result<Recovered<Self::Tx>>
    where
        N::TxEnvelope: SignerRecoverable,
    {
        let sender = envelope.recover_signer()?;
        Self::evm_tx(envelope, sender, TxPayload::Signed)
    }

    /// Decodes exactly the EIP-2718 bytes that were signed and converts them for execution.
    fn decode_tx(mut bytes: &[u8]) -> eyre::Result<Recovered<Self::Tx>>
    where
        N::TxEnvelope: SignerRecoverable,
    {
        let envelope = N::TxEnvelope::decode_2718(&mut bytes)?;
        eyre::ensure!(bytes.is_empty(), "trailing bytes after the encoded transaction");
        Self::tx_from_signed(envelope)
    }

    /// Converts a transaction fetched over RPC into this network's envelope.
    ///
    /// Envelopes this family cannot represent fail rather than being projected onto another type.
    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<N::TxEnvelope>;

    /// Converts a transaction fetched over RPC, using its RPC sender as it executed on chain.
    fn tx_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<Recovered<Self::Tx>> {
        Self::evm_tx(Self::envelope_from_any_rpc(tx)?, tx.from(), TxPayload::Signed)
    }
}

/// Returns the error for an RPC transaction whose envelope `target` cannot represent.
pub(crate) fn unknown_transaction_type(tx: &AnyRpcTransaction, target: &str) -> eyre::Report {
    eyre::eyre!("cannot convert unknown transaction type {:#04x} to {target}", tx.inner.inner.ty())
}

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
    eth_unsigned_tx(match envelope {
        EthereumTxEnvelope::Legacy(tx) => tx.strip_signature().into(),
        EthereumTxEnvelope::Eip2930(tx) => tx.strip_signature().into(),
        EthereumTxEnvelope::Eip1559(tx) => tx.strip_signature().into(),
        EthereumTxEnvelope::Eip4844(tx) => tx.strip_signature().into(),
        EthereumTxEnvelope::Eip7702(tx) => tx.strip_signature().into(),
    })
}

/// Converts an unsigned Ethereum transaction into evm2's Ethereum transaction.
fn eth_unsigned_tx(tx: EthereumTypedTransaction<TxEip4844Variant>) -> TxEnvelope {
    match tx {
        EthereumTypedTransaction::Legacy(tx) => TxEnvelope::Legacy(tx),
        EthereumTypedTransaction::Eip2930(tx) => TxEnvelope::Eip2930(tx),
        EthereumTypedTransaction::Eip1559(tx) => TxEnvelope::Eip1559(tx),
        EthereumTypedTransaction::Eip4844(tx) => TxEnvelope::Eip4844(tx),
        EthereumTypedTransaction::Eip7702(tx) => {
            TxEnvelope::Eip7702(LazyTxEip7702::from_recovered_authorizations(tx))
        }
    }
}

/// Converts a complete unsigned Ethereum request into a simulated evm2 transaction.
///
/// evm2's Ethereum transaction has no signature, so this skips the placeholder-signed envelope of
/// [`FoundryTxConversion::tx_from_request`], which hashes the encoded transaction.
pub fn eth_tx_from_request(request: TransactionRequest) -> eyre::Result<Recovered<TxEnvelope>> {
    let sender = request.from.ok_or_else(|| eyre::eyre!("transaction request has no sender"))?;
    let unsigned = request.build_unsigned().map_err(|err| eyre::eyre!(err.error.to_string()))?;
    Ok(Recovered::new_unchecked(eth_unsigned_tx(unsigned), sender))
}

/// Returns the Ethereum envelope of a transaction fetched over RPC.
///
/// Celo CIP-64 transactions on Celo chains execute as EIP-1559 transactions with the same fields
/// and native fees: fee currencies are not supported by evm2.
pub fn eth_envelope_from_any_rpc(
    tx: &AnyRpcTransaction,
    target: &str,
) -> eyre::Result<alloy_consensus::TxEnvelope> {
    match &*tx.inner.inner {
        AnyTxEnvelope::Ethereum(envelope) => Ok(envelope.clone()),
        AnyTxEnvelope::Unknown(unknown)
            if unknown.ty() == CELO_DYNAMIC_FEE_TX_TYPE
                && matches!(
                    unknown.chain_id().and_then(NamedChain::from_chain_id),
                    Some(NamedChain::Celo | NamedChain::CeloSepolia)
                ) =>
        {
            let mut fields = unknown.inner.fields.clone();
            fields.insert("hash".into(), serde_json::to_value(unknown.hash)?);
            let signed = fields
                .deserialize_into::<Signed<TxEip1559>>()
                .wrap_err("invalid Celo CIP-64 transaction")?;
            Ok(signed.into())
        }
        _ => Err(unknown_transaction_type(tx, target)),
    }
}

/// Transactions of a block and its two ancestors, for network state that depends on the block
/// position of a transaction (see [`FoundryEvmExt::for_block`]).
#[derive(Clone, Debug)]
pub struct BlockContext<FEN: FoundryEvmNetwork> {
    grandparent: Vec<Recovered<TxFor<FEN>>>,
    parent: Vec<Recovered<TxFor<FEN>>>,
    current: Vec<Recovered<TxFor<FEN>>>,
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
    full_transactions(block)?.iter().map(TypesFor::<FEN>::tx_from_any_rpc).collect()
}

/// Returns the transactions of `block`, which must contain full transactions.
pub(crate) fn full_transactions(block: &AnyRpcBlock) -> eyre::Result<&[AnyRpcTransaction]> {
    let BlockTransactions::Full(transactions) = &block.transactions else {
        eyre::bail!("block {} does not contain full transactions", block.header.number());
    };
    Ok(transactions)
}
