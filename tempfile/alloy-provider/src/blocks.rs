use alloy_network::{Ethereum, Network};
use alloy_primitives::{BlockNumber, U64};
use alloy_rpc_client::{NoParams, PollerBuilder, WeakClient};
use alloy_transport::RpcError;
use async_stream::stream;
use futures::{Stream, StreamExt};
use lru::LruCache;
use std::{marker::PhantomData, num::NonZeroUsize};

#[cfg(feature = "pubsub")]
use futures::{future::Either, FutureExt};

/// The size of the block cache.
const BLOCK_CACHE_SIZE: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(10) };

/// Maximum number of retries for fetching a block.
const MAX_RETRIES: usize = 3;

/// Default block number for when we don't have a block yet.
const NO_BLOCK_NUMBER: BlockNumber = BlockNumber::MAX;

/// Streams new blocks from the client.
pub(crate) struct NewBlocks<N: Network = Ethereum> {
    client: WeakClient,
    /// The next block to yield.
    /// [`NO_BLOCK_NUMBER`] indicates that it will be updated on the first poll.
    /// Only used by the polling task.
    next_yield: BlockNumber,
    /// LRU cache of known blocks. Only used by the polling task.
    known_blocks: LruCache<BlockNumber, N::BlockResponse>,
    _phantom: PhantomData<N>,
}

impl<N: Network> NewBlocks<N> {
    pub(crate) fn new(client: WeakClient) -> Self {
        Self {
            client,
            next_yield: NO_BLOCK_NUMBER,
            known_blocks: LruCache::new(BLOCK_CACHE_SIZE),
            _phantom: PhantomData,
        }
    }

    #[cfg(test)]
    #[allow(unused)]
    const fn with_next_yield(mut self, next_yield: u64) -> Self {
        self.next_yield = next_yield;
        self
    }

    pub(crate) fn into_stream(self) -> impl Stream<Item = N::BlockResponse> + 'static {
        // Return a stream that lazily subscribes to `newHeads` on the first poll.
        #[cfg(feature = "pubsub")]
        if let Some(client) = self.client.upgrade() {
            if client.pubsub_frontend().is_some() {
                let subscriber = self.into_subscription_stream().map(futures::stream::iter);
                let subscriber = futures::stream::once(subscriber);
                return Either::Left(subscriber.flatten().flatten());
            }
        }

        // Returns a stream that lazily initializes an `eth_blockNumber` polling task on the first
        // poll, mapped with `eth_getBlockByNumber`.
        #[cfg(feature = "pubsub")]
        let right = Either::Right;
        #[cfg(not(feature = "pubsub"))]
        let right = std::convert::identity;
        right(self.into_poll_stream())
    }

    #[cfg(feature = "pubsub")]
    async fn into_subscription_stream(
        self,
    ) -> Option<impl Stream<Item = N::BlockResponse> + 'static> {
        let Some(client) = self.client.upgrade() else {
            debug!("client dropped");
            return None;
        };
        let Some(pubsub) = client.pubsub_frontend() else {
            error!("pubsub_frontend returned None after being Some");
            return None;
        };
        let id = match client.request("eth_subscribe", ("newHeads",)).await {
            Ok(id) => id,
            Err(err) => {
                error!(%err, "failed to subscribe to newHeads");
                return None;
            }
        };
        let sub = match pubsub.get_subscription(id).await {
            Ok(sub) => sub,
            Err(err) => {
                error!(%err, "failed to get subscription");
                return None;
            }
        };
        Some(sub.into_typed::<N::BlockResponse>().into_stream())
    }

    fn into_poll_stream(mut self) -> impl Stream<Item = N::BlockResponse> + 'static {
        stream! {
        // Spawned lazily on the first `poll`.
        let poll_task_builder: PollerBuilder<NoParams, U64> =
            PollerBuilder::new(self.client.clone(), "eth_blockNumber", []);
        let mut poll_task = poll_task_builder.spawn().into_stream_raw();
        'task: loop {
            // Clear any buffered blocks.
            while let Some(known_block) = self.known_blocks.pop(&self.next_yield) {
                debug!(number=self.next_yield, "yielding block");
                self.next_yield += 1;
                yield known_block;
            }

            // Get the tip.
            let block_number = match poll_task.next().await {
                Some(Ok(block_number)) => block_number,
                Some(Err(err)) => {
                    // This is fine.
                    debug!(%err, "polling stream lagged");
                    continue 'task;
                }
                None => {
                    debug!("polling stream ended");
                    break 'task;
                }
            };
            let block_number = block_number.to::<u64>();
            trace!(%block_number, "got block number");
            if self.next_yield == NO_BLOCK_NUMBER {
                assert!(block_number < NO_BLOCK_NUMBER, "too many blocks");
                self.next_yield = block_number;
            } else if block_number < self.next_yield {
                debug!(block_number, self.next_yield, "not advanced yet");
                continue 'task;
            }

            // Upgrade the provider.
            let Some(client) = self.client.upgrade() else {
                debug!("client dropped");
                break 'task;
            };

            // Then try to fill as many blocks as possible.
            // TODO: Maybe use `join_all`
            let mut retries = MAX_RETRIES;
            for number in self.next_yield..=block_number {
                debug!(number, "fetching block");
                let block = match client.request("eth_getBlockByNumber", (U64::from(number), false)).await {
                    Ok(Some(block)) => block,
                    Err(RpcError::Transport(err)) if retries > 0 && err.recoverable() => {
                        debug!(number, %err, "failed to fetch block, retrying");
                        retries -= 1;
                        continue;
                    }
                    Ok(None) if retries > 0 => {
                        debug!(number, "failed to fetch block (doesn't exist), retrying");
                        retries -= 1;
                        continue;
                    }
                    Err(err) => {
                        error!(number, %err, "failed to fetch block");
                        break 'task;
                    }
                    Ok(None) => {
                        error!(number, "failed to fetch block (doesn't exist)");
                        break 'task;
                    }
                };
                self.known_blocks.put(number, block);
                if self.known_blocks.len() == BLOCK_CACHE_SIZE.get() {
                    // Cache is full, should be consumed before filling more blocks.
                    debug!(number, "cache full");
                    break;
                }
            }
        }
        }
    }
}

#[cfg(all(test, feature = "anvil-api"))] // Tests rely heavily on ability to mine blocks on demand.
mod tests {
    use super::*;
    use crate::{ext::AnvilApi, Provider, ProviderBuilder};
    use alloy_node_bindings::Anvil;
    use std::{future::Future, time::Duration};

    async fn timeout<T: Future>(future: T) -> T::Output {
        try_timeout(future).await.expect("Timeout")
    }

    async fn try_timeout<T: Future>(future: T) -> Option<T::Output> {
        tokio::time::timeout(Duration::from_secs(2), future).await.ok()
    }

    #[tokio::test]
    async fn yield_block_http() {
        yield_block(false).await;
    }
    #[tokio::test]
    #[cfg(feature = "ws")]
    async fn yield_block_ws() {
        yield_block(true).await;
    }
    async fn yield_block(ws: bool) {
        let anvil = Anvil::new().spawn();

        let url = if ws { anvil.ws_endpoint() } else { anvil.endpoint() };
        let provider = ProviderBuilder::new().on_builtin(&url).await.unwrap();

        let new_blocks = NewBlocks::<Ethereum>::new(provider.weak_client()).with_next_yield(1);
        let mut stream = Box::pin(new_blocks.into_stream());
        if ws {
            let _ = try_timeout(stream.next()).await; // Subscribe to newHeads.
        }

        // We will also use provider to manipulate anvil instance via RPC.
        provider.anvil_mine(Some(1), None).await.unwrap();

        let block = timeout(stream.next()).await.expect("Block wasn't fetched");
        assert_eq!(block.header.number, 1);
    }

    #[tokio::test]
    async fn yield_many_blocks_http() {
        yield_many_blocks(false).await;
    }
    #[tokio::test]
    #[cfg(feature = "ws")]
    async fn yield_many_blocks_ws() {
        yield_many_blocks(true).await;
    }
    async fn yield_many_blocks(ws: bool) {
        // Make sure that we can process more blocks than fits in the cache.
        const BLOCKS_TO_MINE: usize = BLOCK_CACHE_SIZE.get() + 1;

        let anvil = Anvil::new().spawn();

        let url = if ws { anvil.ws_endpoint() } else { anvil.endpoint() };
        let provider = ProviderBuilder::new().on_builtin(&url).await.unwrap();

        let new_blocks = NewBlocks::<Ethereum>::new(provider.weak_client()).with_next_yield(1);
        let mut stream = Box::pin(new_blocks.into_stream());
        if ws {
            let _ = try_timeout(stream.next()).await; // Subscribe to newHeads.
        }

        // We will also use provider to manipulate anvil instance via RPC.
        provider.anvil_mine(Some(BLOCKS_TO_MINE as u64), None).await.unwrap();

        let blocks = timeout(stream.take(BLOCKS_TO_MINE).collect::<Vec<_>>()).await;
        assert_eq!(blocks.len(), BLOCKS_TO_MINE);
        let first = blocks[0].header.number;
        assert_eq!(first, 1);
        for (i, block) in blocks.iter().enumerate() {
            assert_eq!(block.header.number, first + i as u64);
        }
    }
}
