//! Mines transactions

use crate::eth::pool::{transactions::PoolTransaction, Pool};
use ethers::prelude::TxHash;
use futures::{channel::mpsc::Receiver, stream::Fuse, Stream};
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

pub enum MiningMode {
    /// A miner that listens for new transactions that are ready
    /// Either one transaction will be mined per block, or any number of transactions will be
    /// allowed
    Instant(ReadyTransactionMiner),
    // TODO add fixed time option
}

impl MiningMode {
    /// polls the [Pool] and returns those transactions that should be put in a block, if any.
    pub fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<PoolTransaction>> {
        match self {
            MiningMode::Instant(miner) => miner.poll(pool, cx),
        }
    }
}

/// Listens for new ready transactions
pub struct ReadyTransactionMiner {
    /// how many transactions to mine per block
    max_transactions: usize,
    /// transactions received
    ready: VecDeque<TxHash>,
    /// receives hashes of transactions that are ready
    rx: Fuse<Receiver<TxHash>>,
}

impl ReadyTransactionMiner {
    fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<PoolTransaction>> {
        while let Poll::Ready(Some(hash)) = Pin::new(&mut self.rx).poll_next(cx) {
            self.ready.push_back(hash);
        }

        if self.ready.is_empty() {
            return Poll::Pending
        }

        pool.ready_transactions();

        todo!()
    }
}
