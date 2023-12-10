//! Task management support

use crate::{shutdown::Shutdown, tasks::block_listener::BlockListener, EthApi};
use anvil_core::types::Forking;
use ethers::{
    prelude::Middleware,
    providers::{JsonRpcClient, PubsubClient},
    types::{Block, H256},
};
use std::{fmt, future::Future};
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

// === impl TaskManager ===

impl TaskManager {
    /// Creates a new instance of the task manager
    pub fn new(tokio_handle: Handle, on_shutdown: Shutdown) -> Self {
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

    /// Spawns the blocking task.
    pub fn spawn_blocking(&self, task: impl Future<Output = ()> + Send + 'static) {
        let handle = self.tokio_handle.clone();
        self.tokio_handle.spawn_blocking(move || {
            handle.block_on(task);
        });
    }

    /// Spawns a new task that listens for new blocks and resets the forked provider for every new
    /// block
    ///
    /// ```
    /// use anvil::{spawn, NodeConfig};
    /// use ethers::providers::Provider;
    /// use std::sync::Arc;
    /// # async fn t() {
    /// let endpoint = "http://....";
    /// let (api, handle) = spawn(NodeConfig::default().with_eth_rpc_url(Some(endpoint))).await;
    ///
    /// let provider = Arc::new(Provider::try_from(endpoint).unwrap());
    ///
    /// handle.task_manager().spawn_reset_on_new_polled_blocks(provider, api);
    /// # }
    /// ```
    pub fn spawn_reset_on_new_polled_blocks<P>(&self, provider: P, api: EthApi)
    where
        P: Middleware + Clone + Unpin + 'static + Send + Sync,
        <P as Middleware>::Provider: JsonRpcClient,
    {
        self.spawn_block_poll_listener(provider.clone(), move |hash| {
            let provider = provider.clone();
            let api = api.clone();
            async move {
                if let Ok(Some(block)) = provider.get_block(hash).await {
                    let _ = api
                        .anvil_reset(Some(Forking {
                            json_rpc_url: None,
                            block_number: block.number.map(|b| b.as_u64()),
                        }))
                        .await;
                }
            }
        })
    }

    /// Spawns a new [`BlockListener`] task that listens for new blocks (poll-based) See also
    /// [`Provider::watch_blocks`] and executes the future the `task_factory` returns for the new
    /// block hash
    pub fn spawn_block_poll_listener<P, F, Fut>(&self, provider: P, task_factory: F)
    where
        P: Middleware + Unpin + 'static,
        <P as Middleware>::Provider: JsonRpcClient,
        F: Fn(H256) -> Fut + Unpin + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let shutdown = self.on_shutdown.clone();
        self.spawn(async move {
            let blocks = provider.watch_blocks().await.unwrap();
            BlockListener::new(shutdown, blocks, task_factory).await;
        });
    }

    /// Spawns a new task that listens for new blocks and resets the forked provider for every new
    /// block
    ///
    /// ```
    /// use anvil::{spawn, NodeConfig};
    /// use ethers::providers::Provider;
    /// # async fn t() {
    /// let (api, handle) = spawn(NodeConfig::default().with_eth_rpc_url(Some("http://...."))).await;
    ///
    /// let provider = Provider::connect("ws://...").await.unwrap();
    ///
    /// handle.task_manager().spawn_reset_on_subscribed_blocks(provider, api);
    ///
    /// # }
    /// ```
    pub fn spawn_reset_on_subscribed_blocks<P>(&self, provider: P, api: EthApi)
    where
        P: Middleware + Unpin + 'static + Send + Sync,
        <P as Middleware>::Provider: PubsubClient,
    {
        self.spawn_block_subscription(provider, move |block| {
            let api = api.clone();
            async move {
                let _ = api
                    .anvil_reset(Some(Forking {
                        json_rpc_url: None,
                        block_number: block.number.map(|b| b.as_u64()),
                    }))
                    .await;
            }
        })
    }

    /// Spawns a new [`BlockListener`] task that listens for new blocks (via subscription) See also
    /// [`Provider::subscribe_blocks()`] and executes the future the `task_factory` returns for the
    /// new block hash
    pub fn spawn_block_subscription<P, F, Fut>(&self, provider: P, task_factory: F)
    where
        P: Middleware + Unpin + 'static,
        <P as Middleware>::Provider: PubsubClient,
        F: Fn(Block<H256>) -> Fut + Unpin + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let shutdown = self.on_shutdown.clone();
        self.spawn(async move {
            let blocks = provider.subscribe_blocks().await.unwrap();
            BlockListener::new(shutdown, blocks, task_factory).await;
        });
    }
}

impl fmt::Debug for TaskManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskManager").finish_non_exhaustive()
    }
}
