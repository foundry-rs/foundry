use alloy_consensus::BlockHeader;
use alloy_evm::FromRecoveredTx;
use alloy_network::{BlockResponse, TransactionResponse};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions};
use eyre::{Result, WrapErr};

use super::{BlockResponseFor, ContextAuxFor, FoundryEvmFactory, FoundryEvmNetwork, TxEnvFor};

/// Transaction metadata for an exact block and its two ancestors.
#[derive(Clone, Debug)]
pub struct BlockContext<FEN: FoundryEvmNetwork> {
    grandparent: Vec<TxEnvFor<FEN>>,
    parent: Vec<TxEnvFor<FEN>>,
    current: Vec<TxEnvFor<FEN>>,
}

impl<FEN: FoundryEvmNetwork> BlockContext<FEN> {
    /// Creates block context from grandparent, parent, and current block transactions.
    pub const fn new(
        grandparent: Vec<TxEnvFor<FEN>>,
        parent: Vec<TxEnvFor<FEN>>,
        current: Vec<TxEnvFor<FEN>>,
    ) -> Self {
        Self { grandparent, parent, current }
    }

    /// Fetches all transaction bodies needed to replay transactions in `block` exactly.
    pub async fn fetch<P: Provider<FEN::Network>>(
        provider: &P,
        block: &BlockResponseFor<FEN>,
    ) -> Result<Self> {
        let current = transaction_envs::<FEN>(block)?;
        let parent = fetch_parent::<FEN, P>(provider, block).await?;
        let grandparent = if let Some(parent) = &parent {
            fetch_parent::<FEN, P>(provider, parent).await?
        } else {
            None
        };

        Ok(Self::new(
            grandparent.as_ref().map(transaction_envs::<FEN>).transpose()?.unwrap_or_default(),
            parent.as_ref().map(transaction_envs::<FEN>).transpose()?.unwrap_or_default(),
            current,
        ))
    }

    /// Builds context for the transaction at `index` in the current block.
    pub fn transaction(&self, index: usize) -> ContextAuxFor<FEN> {
        FEN::EvmFactory::default().context_for_block(
            &self.grandparent,
            &self.parent,
            &self.current,
            index,
        )
    }

    /// Returns a cursor positioned immediately before `index` in the current block.
    pub fn before_transaction(mut self, index: usize) -> Result<Self> {
        if index > self.current.len() {
            eyre::bail!(
                "transaction index {index} exceeds block transaction count {}",
                self.current.len()
            );
        }
        self.current.truncate(index);
        Ok(self)
    }

    /// Returns a cursor positioned at the start of a child block.
    pub fn into_child(mut self) -> Self {
        self.grandparent = std::mem::take(&mut self.parent);
        self.parent = std::mem::take(&mut self.current);
        self
    }

    /// Builds context for the next transaction at the cursor's current block position.
    pub fn next_transaction(&self, tx: &TxEnvFor<FEN>) -> ContextAuxFor<FEN> {
        let mut current = self.current.clone();
        let index = current.len();
        current.push(tx.clone());
        FEN::EvmFactory::default().context_for_block(
            &self.grandparent,
            &self.parent,
            &current,
            index,
        )
    }

    /// Records a committed transaction at the cursor's current block position.
    pub fn record_transaction(&mut self, tx: TxEnvFor<FEN>) {
        self.current.push(tx);
    }

    /// Advances the cursor to the start of the next block.
    pub fn advance_block(&mut self) {
        self.grandparent = std::mem::take(&mut self.parent);
        self.parent = std::mem::take(&mut self.current);
    }

    /// Builds context for a synthetic transaction in a child of the current block.
    pub fn child(&self, tx: &TxEnvFor<FEN>) -> ContextAuxFor<FEN> {
        self.clone().into_child().next_transaction(tx)
    }
}

/// Builds context for a synthetic transaction executed on top of `block_number`.
pub async fn context_for_child_transaction<FEN, P>(
    provider: &P,
    block_number: u64,
    tx: &TxEnvFor<FEN>,
) -> Result<ContextAuxFor<FEN>>
where
    FEN: FoundryEvmNetwork,
    P: Provider<FEN::Network>,
{
    let factory = FEN::EvmFactory::default();
    if !FEN::EvmFactory::NEEDS_BLOCK_CONTEXT {
        return Ok(factory.context_for_transaction(tx));
    }

    let block = provider
        .get_block(BlockNumberOrTag::Number(block_number).into())
        .full()
        .await?
        .ok_or_else(|| eyre::eyre!("block {block_number} not found while building EVM context"))?;
    let parent = fetch_parent::<FEN, P>(provider, &block).await?;
    let current = transaction_envs::<FEN>(&block)?;
    let parent = parent.as_ref().map(transaction_envs::<FEN>).transpose()?.unwrap_or_default();

    Ok(BlockContext::<FEN>::new(Vec::new(), parent, current).child(tx))
}

async fn fetch_parent<FEN, P>(
    provider: &P,
    block: &BlockResponseFor<FEN>,
) -> Result<Option<BlockResponseFor<FEN>>>
where
    FEN: FoundryEvmNetwork,
    P: Provider<FEN::Network>,
{
    let parent_hash = block.header().parent_hash();
    if parent_hash.is_zero() {
        return Ok(None);
    }

    provider
        .get_block_by_hash(parent_hash)
        .full()
        .await
        .wrap_err_with(|| format!("failed to fetch ancestor block {parent_hash}"))?
        .map(Some)
        .ok_or_else(|| eyre::eyre!("ancestor block {parent_hash} not found"))
}

fn transaction_envs<FEN: FoundryEvmNetwork>(
    block: &BlockResponseFor<FEN>,
) -> Result<Vec<TxEnvFor<FEN>>> {
    let BlockTransactions::Full(transactions) = block.transactions() else {
        eyre::bail!("block {} does not contain full transactions", block.header().number());
    };
    Ok(transactions
        .iter()
        .map(|tx| TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from()))
        .collect())
}
