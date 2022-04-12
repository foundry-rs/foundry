//! Mines transactions

use crate::eth::pool::{transactions::PoolTransaction, Pool};
use ethers::prelude::TxHash;
use futures::{
    channel::mpsc::Receiver,
    stream::{Fuse, Stream, StreamExt},
};
use std::{
    collections::HashSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Interval;

#[derive(Debug)]
pub enum MiningMode {
    /// A miner that listens for new transactions that are ready.
    ///
    /// Either one transaction will be mined per block, or any number of transactions will be
    /// allowed
    Instant(ReadyTransactionMiner),
    /// A miner that constructs a new block every `interval` tick
    FixedBlockTime(FixedBlockTimeMiner),
}

impl MiningMode {
    pub fn instant(max_transactions: usize, listener: Receiver<TxHash>) -> Self {
        MiningMode::Instant(ReadyTransactionMiner {
            max_transactions,
            ready: Default::default(),
            rx: listener.fuse(),
        })
    }

    pub fn interval(duration: Duration) -> Self {
        MiningMode::FixedBlockTime(FixedBlockTimeMiner::new(duration))
    }

    /// polls the [Pool] and returns those transactions that should be put in a block, if any.
    pub fn poll(
        &mut self,
        pool: &Arc<Pool>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction>>> {
        match self {
            MiningMode::Instant(miner) => miner.poll(pool, cx),
            MiningMode::FixedBlockTime(miner) => miner.poll(pool, cx),
        }
    }
}

/// A miner that's supposed to create a new block every `interval`, mining all transactions that are
/// ready at that time.
///
/// The default blocktime is set to 6 seconds
#[derive(Debug)]
pub struct FixedBlockTimeMiner {
    interval: Interval,
}

impl FixedBlockTimeMiner {
    pub fn new(duration: Duration) -> Self {
        Self { interval: tokio::time::interval(duration) }
    }

    fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<Arc<PoolTransaction>>> {
        if self.interval.poll_tick(cx).is_ready() {
            // drain the pool
            return Poll::Ready(pool.ready_transactions().collect())
        }
        Poll::Pending
    }
}

impl Default for FixedBlockTimeMiner {
    fn default() -> Self {
        Self::new(Duration::from_secs(6))
    }
}

/// A miner that Listens for new ready transactions
#[derive(Debug)]
pub struct ReadyTransactionMiner {
    /// how many transactions to mine per block
    max_transactions: usize,
    /// transactions received
    ready: HashSet<TxHash>,
    /// receives hashes of transactions that are ready
    rx: Fuse<Receiver<TxHash>>,
}

impl ReadyTransactionMiner {
    fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<Arc<PoolTransaction>>> {
        while let Poll::Ready(Some(hash)) = Pin::new(&mut self.rx).poll_next(cx) {
            self.ready.insert(hash);
        }

        if self.ready.is_empty() {
            return Poll::Pending
        }

        let transactions =
            pool.ready_transactions().take(self.max_transactions).collect::<Vec<_>>();

        for tx in transactions.iter() {
            self.ready.remove(tx.hash());
        }

        Poll::Ready(transactions)
    }
}
