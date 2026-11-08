//! Task management support

#![allow(rustdoc::private_doc_tests)]

use crate::{
    EthApi, eth::error::BlockchainError, shutdown::Shutdown, tasks::block_listener::BlockListener,
};
use alloy_consensus::BlockHeader;
use alloy_network::{BlockResponse, Network};
use alloy_primitives::B256;
use alloy_provider::Provider;
use alloy_rpc_types::anvil::Forking;
use foundry_primitives::FoundryNetwork;
use futures::StreamExt;
use std::{fmt, time::Duration};
use tokio::{runtime::Handle, task::JoinHandle};

pub mod block_listener;

/// A helper struct for managing additional tokio tasks.
#[derive(Clone)]
pub struct TaskManager {
    /// Tokio runtime handle that's used to spawn futures, See [tokio::runtime::Handle].
    tokio_handle: Handle,
    /// A receiver for the shutdown signal
    on_shutdown: Shutdown,
}

impl TaskManager {
    /// Creates a new instance of the task manager
    pub const fn new(tokio_handle: Handle, on_shutdown: Shutdown) -> Self {
        Self { tokio_handle, on_shutdown }
    }

    /// Returns a receiver for the shutdown event
    pub fn on_shutdown(&self) -> Shutdown {
        self.on_shutdown.clone()
    }

    /// Spawns the given task.
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> JoinHandle<()> {
        self.tokio_handle.spawn(task)
    }

    /// Spawns the blocking task and returns a handle to it.
    ///
    /// Returning the `JoinHandle` allows callers to cancel the task or await its completion.
    pub fn spawn_blocking(
        &self,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> JoinHandle<()> {
        let handle = self.tokio_handle.clone();
        self.tokio_handle.spawn_blocking(move || {
            handle.block_on(task);
        })
    }

    /// Spawns a new task that listens for new blocks and resets the forked provider to the latest
    /// available block.
    ///
    /// ```
    /// use alloy_network::Ethereum;
    /// use alloy_provider::RootProvider;
    /// use anvil::{NodeConfig, spawn};
    ///
    /// # async fn t() {
    /// let endpoint = "http://....";
    /// let (api, handle) = spawn(NodeConfig::default().with_eth_rpc_url(Some(endpoint))).await;
    ///
    /// let provider = RootProvider::connect(endpoint).await.unwrap();
    ///
    /// handle.task_manager().spawn_reset_on_new_polled_blocks::<Ethereum, _>(provider, api);
    /// # }
    /// ```
    pub fn spawn_reset_on_new_polled_blocks<N, P>(&self, provider: P, api: EthApi<FoundryNetwork>)
    where
        N: Network,
        P: Provider<N> + Clone + Unpin + 'static,
    {
        self.spawn_block_poll_listener(provider.clone(), move |hash| {
            let provider = provider.clone();
            let api = api.clone();
            async move {
                if let Ok(Some(block)) = provider.get_block(hash.into()).await {
                    reset_on_new_block(&api, block.header().number()).await;
                }
            }
        })
    }

    /// Spawns a new [`BlockListener`] task that listens for new blocks (poll-based) See also
    /// [`Provider::watch_blocks`] and executes the future the `task_factory` returns for the latest
    /// available block hash.
    pub fn spawn_block_poll_listener<N, P, F, Fut>(&self, provider: P, task_factory: F)
    where
        N: Network,
        P: Provider<N> + 'static,
        F: Fn(B256) -> Fut + Unpin + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let shutdown = self.on_shutdown.clone();
        self.spawn(async move {
            let blocks = provider
                .watch_blocks()
                .await
                .unwrap()
                .into_stream()
                .flat_map(futures::stream::iter);
            BlockListener::new(shutdown, blocks, task_factory).await;
        });
    }

    /// Spawns a new task that listens for new blocks and resets the forked provider to the latest
    /// available block.
    ///
    /// ```
    /// use alloy_network::Ethereum;
    /// use alloy_provider::RootProvider;
    /// use anvil::{NodeConfig, spawn};
    ///
    /// # async fn t() {
    /// let (api, handle) = spawn(NodeConfig::default().with_eth_rpc_url(Some("http://...."))).await;
    ///
    /// let provider = RootProvider::connect("ws://...").await.unwrap();
    ///
    /// handle.task_manager().spawn_reset_on_subscribed_blocks::<Ethereum, _>(provider, api);
    ///
    /// # }
    /// ```
    pub fn spawn_reset_on_subscribed_blocks<N, P>(&self, provider: P, api: EthApi<FoundryNetwork>)
    where
        N: Network,
        P: Provider<N> + 'static,
    {
        self.spawn_block_subscription(provider, move |header: N::HeaderResponse| {
            let api = api.clone();
            async move {
                reset_on_new_block(&api, header.number()).await;
            }
        })
    }

    /// Spawns a new [`BlockListener`] task that listens for new blocks (via subscription) See also
    /// [`Provider::subscribe_blocks()`] and executes the future the `task_factory` returns for the
    /// latest available block.
    pub fn spawn_block_subscription<N, P, F, Fut>(&self, provider: P, task_factory: F)
    where
        N: Network,
        P: Provider<N> + 'static,
        F: Fn(N::HeaderResponse) -> Fut + Unpin + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let shutdown = self.on_shutdown.clone();
        self.spawn(async move {
            let blocks = provider.subscribe_blocks().await.unwrap().into_stream();
            BlockListener::new(shutdown, blocks, task_factory).await;
        });
    }
}

impl fmt::Debug for TaskManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskManager").finish_non_exhaustive()
    }
}

/// Resets to an observed block once active RPCs allow the lifecycle transition.
pub(crate) async fn reset_on_new_block(api: &EthApi<FoundryNetwork>, block_number: u64) {
    // Delay busy retries to avoid a tight loop while preserving the last observed block.
    // BlockListener retains this future until completion and cancels it on shutdown.
    const BUSY_RETRY_DELAY: Duration = Duration::from_millis(100);

    while matches!(
        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(block_number) }))
            .await,
        Err(BlockchainError::NodeBusy)
    ) {
        tokio::time::sleep(BUSY_RETRY_DELAY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeConfig, spawn};

    #[tokio::test(flavor = "multi_thread")]
    async fn reset_on_new_block_stops_on_non_busy_errors() {
        let (api, _handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
        let instance = api.instance_id();

        // A node without a fork source cannot follow a block; this error must not be retried.
        tokio::time::timeout(Duration::from_secs(5), reset_on_new_block(&api, 1)).await.unwrap();

        assert_eq!(api.instance_id(), instance);
        assert!(!api.is_fork());
        assert_eq!(api.backend.best_number(), 0);
    }
}
