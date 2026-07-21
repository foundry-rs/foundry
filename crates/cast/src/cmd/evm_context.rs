use alloy_consensus::BlockHeader;
use alloy_evm::FromRecoveredTx;
use alloy_network::{BlockResponse, TransactionResponse};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions};
use eyre::{Result, WrapErr};
use foundry_evm::core::evm::{
    BlockResponseFor, ContextAuxFor, FoundryEvmFactory, FoundryEvmNetwork, TxEnvFor,
};

/// Transaction metadata for an exact block and its two ancestors.
pub(super) struct BlockContext<FEN: FoundryEvmNetwork> {
    grandparent: Vec<TxEnvFor<FEN>>,
    parent: Vec<TxEnvFor<FEN>>,
    current: Vec<TxEnvFor<FEN>>,
}

impl<FEN: FoundryEvmNetwork> BlockContext<FEN> {
    /// Fetches all transaction bodies needed to replay transactions in `block` exactly.
    pub(super) async fn fetch<P: Provider<FEN::Network>>(
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

        Ok(Self {
            grandparent: grandparent
                .as_ref()
                .map(transaction_envs::<FEN>)
                .transpose()?
                .unwrap_or_default(),
            parent: parent.as_ref().map(transaction_envs::<FEN>).transpose()?.unwrap_or_default(),
            current,
        })
    }

    /// Builds context for the transaction at `index` in the current block.
    pub(super) fn transaction(&self, index: usize) -> ContextAuxFor<FEN> {
        FEN::EvmFactory::default().context_for_block(
            &self.grandparent,
            &self.parent,
            &self.current,
            index,
        )
    }
}

/// Builds context for a synthetic transaction executed on top of `block_number`.
pub(super) async fn context_for_child_transaction<FEN, P>(
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
    let parent_transactions = transaction_envs::<FEN>(&block)?;
    let grandparent_transactions =
        parent.as_ref().map(transaction_envs::<FEN>).transpose()?.unwrap_or_default();

    Ok(factory.context_for_block(
        &grandparent_transactions,
        &parent_transactions,
        std::slice::from_ref(tx),
        0,
    ))
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
        eyre::bail!("block {} does not contain full transactions", block.header().number())
    };
    Ok(transactions
        .iter()
        .map(|tx| TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from()))
        .collect())
}
