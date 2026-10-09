//! The `txpool` namespace, as reth serves it, except that a transaction is keyed the way anvil
//! keys it: by its nonce, or by its nonce key and nonce on a Tempo nonce lane, so the
//! transactions of one sender on two lanes do not collide.

use alloy_consensus::Transaction;
use alloy_primitives::Address;
use alloy_rpc_types::txpool::{
    TxpoolContent, TxpoolContentFrom, TxpoolInspect, TxpoolInspectSummary, TxpoolStatus,
};
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
};
use reth_ethereum::{
    TransactionSigned,
    pool::{
        AllPoolTransactions, PoolConsensusTx, PoolTransaction, PoolTx, TransactionPool,
        ValidPoolTransaction,
    },
    primitives::NodePrimitives,
};
use reth_rpc_eth_api::{EthApiTypes, RpcConvert, RpcNodeCore, RpcTransaction};
use std::{collections::BTreeMap, marker::PhantomData, sync::Arc};

/// The key of a pooled transaction in the `txpool` maps.
pub trait TxPoolKey<Net = ()> {
    /// Returns the key: the nonce, or the nonce key and nonce of a transaction on a nonce lane.
    fn txpool_key(&self) -> String;
}

impl<Net> TxPoolKey<Net> for TransactionSigned {
    fn txpool_key(&self) -> String {
        self.nonce().to_string()
    }
}

/// The `txpool` namespace.
#[rpc(server, namespace = "txpool")]
pub trait AnvilTxPoolApi<Tx> {
    /// Returns the number of pending and queued transactions.
    #[method(name = "status")]
    async fn txpool_status(&self) -> RpcResult<TxpoolStatus>;

    /// Returns a summary of the pending and queued transactions.
    #[method(name = "inspect")]
    async fn txpool_inspect(&self) -> RpcResult<TxpoolInspect>;

    /// Returns the pending and queued transactions of a sender.
    #[method(name = "contentFrom")]
    async fn txpool_content_from(&self, from: Address) -> RpcResult<TxpoolContentFrom<Tx>>;

    /// Returns the pending and queued transactions.
    #[method(name = "content")]
    async fn txpool_content(&self) -> RpcResult<TxpoolContent<Tx>>;
}

/// The `txpool` namespace over the node's `eth` API, which holds the pool and the RPC converter.
#[derive(Clone, Debug)]
pub struct AnvilTxPool<Eth, Net = ()> {
    network: PhantomData<fn() -> Net>,
    eth: Eth,
}

impl<Eth, Net: Clone + Send + Sync + 'static> AnvilTxPool<Eth, Net> {
    /// Creates the namespace.
    pub const fn new(eth: Eth) -> Self {
        Self { eth, network: PhantomData }
    }
}

impl<Eth, Net: Clone + Send + Sync + 'static> AnvilTxPool<Eth, Net>
where
    Eth: RpcNodeCore<
            Pool: TransactionPool<
                Transaction: PoolTransaction<Consensus: Transaction + TxPoolKey<Net>>,
            >,
        > + EthApiTypes<
            RpcConvert: RpcConvert<
                Primitives: NodePrimitives<SignedTx = PoolConsensusTx<Eth::Pool>>,
            >,
        >,
{
    /// Converts the pooled transaction and inserts it under its key.
    fn insert(
        &self,
        tx: &PoolTx<Eth::Pool>,
        txs: &mut BTreeMap<String, RpcTransaction<RpcNetwork<Eth>>>,
    ) -> RpcResult<()> {
        let tx = tx.clone_into_consensus();
        let key = tx.txpool_key();
        let tx = self.eth.converter().fill_pending(tx).map_err(Into::into)?;
        txs.insert(key, tx);
        Ok(())
    }
}

/// The RPC network of an `eth` API.
type RpcNetwork<Eth> = <<Eth as EthApiTypes>::RpcConvert as RpcConvert>::Network;

#[async_trait]
impl<Eth, Net: Clone + Send + Sync + 'static> AnvilTxPoolApiServer<RpcTransaction<RpcNetwork<Eth>>>
    for AnvilTxPool<Eth, Net>
where
    Eth: RpcNodeCore<
            Pool: TransactionPool<
                Transaction: PoolTransaction<Consensus: Transaction + TxPoolKey<Net>>,
            >,
        > + EthApiTypes<
            RpcConvert: RpcConvert<
                Primitives: NodePrimitives<SignedTx = PoolConsensusTx<Eth::Pool>>,
            >,
        > + 'static,
{
    async fn txpool_status(&self) -> RpcResult<TxpoolStatus> {
        let (pending, queued) = self.eth.pool().pending_and_queued_txn_count();
        Ok(TxpoolStatus { pending: pending as u64, queued: queued as u64 })
    }

    async fn txpool_inspect(&self) -> RpcResult<TxpoolInspect> {
        let AllPoolTransactions { pending, queued } = self.eth.pool().all_transactions();
        Ok(TxpoolInspect {
            pending: summarize::<_, Net>(pending),
            queued: summarize::<_, Net>(queued),
        })
    }

    async fn txpool_content_from(
        &self,
        from: Address,
    ) -> RpcResult<TxpoolContentFrom<RpcTransaction<RpcNetwork<Eth>>>> {
        let mut content = TxpoolContentFrom::default();
        let AllPoolTransactions { pending, queued } =
            self.eth.pool().all_transactions_by_sender(from);
        for tx in pending {
            self.insert(&tx.transaction, &mut content.pending)?;
        }
        for tx in queued {
            self.insert(&tx.transaction, &mut content.queued)?;
        }
        Ok(content)
    }

    async fn txpool_content(&self) -> RpcResult<TxpoolContent<RpcTransaction<RpcNetwork<Eth>>>> {
        let AllPoolTransactions { pending, queued } = self.eth.pool().all_transactions();
        let mut content = TxpoolContent::default();
        for tx in pending {
            let sender = tx.transaction.sender();
            self.insert(&tx.transaction, content.pending.entry(sender).or_default())?;
        }
        for tx in queued {
            let sender = tx.transaction.sender();
            self.insert(&tx.transaction, content.queued.entry(sender).or_default())?;
        }
        Ok(content)
    }
}

/// Summarizes pooled transactions by sender and key, as `txpool_inspect` reports them.
fn summarize<T: PoolTransaction<Consensus: Transaction + TxPoolKey<Net>>, Net>(
    txs: Vec<Arc<ValidPoolTransaction<T>>>,
) -> BTreeMap<Address, BTreeMap<String, TxpoolInspectSummary>> {
    let mut summary = BTreeMap::<Address, BTreeMap<String, TxpoolInspectSummary>>::new();
    for tx in txs {
        let sender = tx.transaction.sender();
        let tx = tx.transaction.clone_into_consensus();
        let key = tx.txpool_key();
        summary.entry(sender).or_default().insert(key, tx.into_inner().into());
    }
    summary
}
