//! background service

use crate::eth::{backend, miner::MiningMode, pool::Pool};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// The type that drives the blockchain's state
///
/// This service is basically an endless future that continuously polls the miner which returns
/// transactions for the next block, then those transactions are handed off to the
/// [backend](backend::mem::Backend) to construct a new block, if all transactions were successfully
/// included in a new block they get purged from the `Pool`.
pub struct NodeService {
    /// the pool that holds all transactions
    pool: Arc<Pool>,
    /// holds the blockchain's state
    backend: Arc<backend::mem::Backend>,
    /// the miner responsible to select transactions from the `pool´
    miner: MiningMode,
}

impl NodeService {
    pub fn new(pool: Arc<Pool>, backend: Arc<backend::mem::Backend>, miner: MiningMode) -> Self {
        Self { pool, backend, miner }
    }
}

impl Future for NodeService {
    // Note: this is out of convenience as this gets joined with the server
    type Output = hyper::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        while let Poll::Ready(transactions) = pin.miner.poll(&pin.pool, cx) {
            // miner returned a set of transaction to put into a new block
            let block_number = pin.backend.mine_block(transactions.clone());

            // prune all the markers the mined transactions provide
            pin.pool.prune_markers(
                block_number,
                transactions.into_iter().flat_map(|tx| tx.provides.clone()),
            );
        }

        Poll::Pending
    }
}
