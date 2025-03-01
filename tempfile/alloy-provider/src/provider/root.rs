use crate::{
    blocks::NewBlocks,
    heart::{Heartbeat, HeartbeatHandle},
    Identity, ProviderBuilder,
};
use alloy_network::{Ethereum, Network};
use alloy_rpc_client::{BuiltInConnectionString, ClientBuilder, ClientRef, RpcClient, WeakClient};
use alloy_transport::{TransportConnect, TransportError};
use std::{
    fmt,
    marker::PhantomData,
    sync::{Arc, OnceLock},
};

#[cfg(feature = "pubsub")]
use alloy_pubsub::{PubSubFrontend, Subscription};

/// The root provider manages the RPC client and the heartbeat. It is at the
/// base of every provider stack.
pub struct RootProvider<N: Network = Ethereum> {
    /// The inner state of the root provider.
    pub(crate) inner: Arc<RootProviderInner<N>>,
}

impl<N: Network> Clone for RootProvider<N> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<N: Network> fmt::Debug for RootProvider<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RootProvider").field("client", &self.inner.client).finish_non_exhaustive()
    }
}

/// Helper function to directly access [`ProviderBuilder`] with minimal
/// generics.
pub fn builder<N: Network>() -> ProviderBuilder<Identity, Identity, N> {
    ProviderBuilder::default()
}

impl<N: Network> RootProvider<N> {
    /// Creates a new HTTP root provider from the given URL.
    #[cfg(feature = "reqwest")]
    pub fn new_http(url: url::Url) -> Self {
        Self::new(RpcClient::new_http(url))
    }

    /// Creates a new root provider from the given RPC client.
    pub fn new(client: RpcClient) -> Self {
        Self { inner: Arc::new(RootProviderInner::new(client)) }
    }

    /// Creates a new root provider from the provided string.
    ///
    /// See [`BuiltInConnectionString`] for more information.
    pub async fn connect(s: &str) -> Result<Self, TransportError> {
        Self::connect_with(s.parse::<BuiltInConnectionString>()?).await
    }

    /// Creates a new root provider from the provided connection details.
    #[deprecated(since = "0.9.0", note = "use `connect` instead")]
    pub async fn connect_builtin(s: &str) -> Result<Self, TransportError> {
        Self::connect(s).await
    }

    /// Connects to a transport with the given connector.
    pub async fn connect_with<C: TransportConnect>(conn: C) -> Result<Self, TransportError> {
        ClientBuilder::default().connect_with(conn).await.map(Self::new)
    }

    /// Connects to a boxed transport with the given connector.
    #[deprecated(
        since = "0.9.0",
        note = "`RootProvider` is now always boxed, use `connect_with` instead"
    )]
    pub async fn connect_boxed<C: TransportConnect>(conn: C) -> Result<Self, TransportError> {
        Self::connect_with(conn).await
    }
}

impl<N: Network> RootProvider<N> {
    /// Boxes the inner client.
    #[deprecated(since = "0.9.0", note = "`RootProvider` is now always boxed")]
    #[allow(clippy::missing_const_for_fn)]
    pub fn boxed(self) -> Self {
        self
    }

    /// Gets the subscription corresponding to the given RPC subscription ID.
    #[cfg(feature = "pubsub")]
    pub async fn get_subscription<R: alloy_json_rpc::RpcRecv>(
        &self,
        id: alloy_primitives::B256,
    ) -> alloy_transport::TransportResult<Subscription<R>> {
        self.pubsub_frontend()?.get_subscription(id).await.map(Subscription::from)
    }

    /// Unsubscribes from the subscription corresponding to the given RPC subscription ID.
    #[cfg(feature = "pubsub")]
    pub fn unsubscribe(&self, id: alloy_primitives::B256) -> alloy_transport::TransportResult<()> {
        self.pubsub_frontend()?.unsubscribe(id)
    }

    #[cfg(feature = "pubsub")]
    pub(crate) fn pubsub_frontend(&self) -> alloy_transport::TransportResult<&PubSubFrontend> {
        self.inner
            .client_ref()
            .pubsub_frontend()
            .ok_or_else(alloy_transport::TransportErrorKind::pubsub_unavailable)
    }

    #[inline]
    pub(crate) fn get_heart(&self) -> &HeartbeatHandle<N> {
        self.inner.heart.get_or_init(|| {
            let new_blocks = NewBlocks::<N>::new(self.inner.weak_client());
            let stream = new_blocks.into_stream();
            Heartbeat::new(Box::pin(stream)).spawn()
        })
    }
}

/// The root provider manages the RPC client and the heartbeat. It is at the
/// base of every provider stack.
pub(crate) struct RootProviderInner<N: Network = Ethereum> {
    client: RpcClient,
    heart: OnceLock<HeartbeatHandle<N>>,
    _network: PhantomData<N>,
}

impl<N: Network> Clone for RootProviderInner<N> {
    fn clone(&self) -> Self {
        Self { client: self.client.clone(), heart: self.heart.clone(), _network: PhantomData }
    }
}

impl<N: Network> RootProviderInner<N> {
    pub(crate) fn new(client: RpcClient) -> Self {
        Self { client, heart: Default::default(), _network: PhantomData }
    }

    pub(crate) fn weak_client(&self) -> WeakClient {
        self.client.get_weak()
    }

    pub(crate) fn client_ref(&self) -> ClientRef<'_> {
        self.client.get_ref()
    }
}
