use crate::{BuiltInConnectionString, RpcClient};
use alloy_transport::{BoxTransport, IntoBoxTransport, TransportConnect, TransportResult};
use tower::{
    layer::util::{Identity, Stack},
    Layer, ServiceBuilder,
};

/// A builder for the transport  [`RpcClient`].
///
/// This is a wrapper around [`tower::ServiceBuilder`]. It allows you to
/// configure middleware layers that will be applied to the transport, and has
/// some shortcuts for common layers and transports.
///
/// A builder accumulates Layers, and then is finished via the
/// [`ClientBuilder::connect`] method, which produces an RPC client.
#[derive(Debug)]
pub struct ClientBuilder<L> {
    pub(crate) builder: ServiceBuilder<L>,
}

impl Default for ClientBuilder<Identity> {
    fn default() -> Self {
        Self { builder: ServiceBuilder::new() }
    }
}

impl<L> ClientBuilder<L> {
    /// Add a middleware layer to the stack.
    ///
    /// This is a wrapper around [`tower::ServiceBuilder::layer`]. Layers that
    /// are added first will be called with the request first.
    pub fn layer<M>(self, layer: M) -> ClientBuilder<Stack<M, L>> {
        ClientBuilder { builder: self.builder.layer(layer) }
    }

    /// Create a new [`RpcClient`] with the given transport and the configured
    /// layers.
    ///
    /// This collapses the [`tower::ServiceBuilder`] with the given transport via
    /// [`tower::ServiceBuilder::service`].
    pub fn transport<T>(self, transport: T, is_local: bool) -> RpcClient
    where
        L: Layer<T>,
        T: IntoBoxTransport,
        L::Service: IntoBoxTransport,
    {
        RpcClient::new_layered(is_local, transport, move |t| self.builder.service(t))
    }

    /// Convenience function to create a new [`RpcClient`] with a [`reqwest`]
    /// HTTP transport.
    #[cfg(feature = "reqwest")]
    pub fn http(self, url: url::Url) -> RpcClient
    where
        L: Layer<alloy_transport_http::Http<reqwest::Client>>,
        L::Service: IntoBoxTransport,
    {
        let transport = alloy_transport_http::Http::new(url);
        let is_local = transport.guess_local();

        self.transport(transport, is_local)
    }

    /// Convenience function to create a new [`RpcClient`] with a `hyper` HTTP transport.
    #[cfg(all(not(target_arch = "wasm32"), feature = "hyper"))]
    pub fn hyper_http(self, url: url::Url) -> RpcClient
    where
        L: Layer<alloy_transport_http::HyperTransport>,
        L::Service: IntoBoxTransport,
    {
        let transport = alloy_transport_http::HyperTransport::new_hyper(url);
        let is_local = transport.guess_local();

        self.transport(transport, is_local)
    }

    /// Connect a pubsub transport, producing an [`RpcClient`] with the provided
    /// connection.
    #[cfg(feature = "pubsub")]
    pub async fn pubsub<C>(self, pubsub_connect: C) -> TransportResult<RpcClient>
    where
        C: alloy_pubsub::PubSubConnect,
        L: Layer<alloy_pubsub::PubSubFrontend>,
        L::Service: IntoBoxTransport,
    {
        let is_local = pubsub_connect.is_local();
        let transport = pubsub_connect.into_service().await?;
        Ok(self.transport(transport, is_local))
    }

    /// Connect a WS transport, producing an [`RpcClient`] with the provided
    /// connection.
    #[cfg(feature = "ws")]
    pub async fn ws(self, ws_connect: alloy_transport_ws::WsConnect) -> TransportResult<RpcClient>
    where
        L: Layer<alloy_pubsub::PubSubFrontend>,
        L::Service: IntoBoxTransport,
    {
        self.pubsub(ws_connect).await
    }

    /// Connect an IPC transport, producing an [`RpcClient`] with the provided
    /// connection.
    #[cfg(feature = "ipc")]
    pub async fn ipc<T>(
        self,
        ipc_connect: alloy_transport_ipc::IpcConnect<T>,
    ) -> TransportResult<RpcClient>
    where
        alloy_transport_ipc::IpcConnect<T>: alloy_pubsub::PubSubConnect,
        L: Layer<alloy_pubsub::PubSubFrontend>,
        L::Service: IntoBoxTransport,
    {
        self.pubsub(ipc_connect).await
    }

    /// Connect a transport specified by the given string, producing an [`RpcClient`].
    ///
    /// See [`BuiltInConnectionString`] for more information.
    pub async fn connect(self, s: &str) -> TransportResult<RpcClient>
    where
        L: Layer<BoxTransport>,
        L::Service: IntoBoxTransport,
    {
        self.connect_with(s.parse::<BuiltInConnectionString>()?).await
    }

    /// Connect a transport, producing an [`RpcClient`].
    pub async fn connect_with<C>(self, connect: C) -> TransportResult<RpcClient>
    where
        C: TransportConnect,
        L: Layer<BoxTransport>,
        L::Service: IntoBoxTransport,
    {
        let transport = connect.get_transport().await?;
        Ok(self.transport(transport, connect.is_local()))
    }

    /// Connect a transport, producing an [`RpcClient`] with a [`BoxTransport`]
    /// connection.
    #[deprecated(
        since = "0.9.0",
        note = "RpcClient is now always boxed, use `connect_with` instead"
    )]
    pub async fn connect_boxed<C>(self, connect: C) -> TransportResult<RpcClient>
    where
        C: TransportConnect,
        L: Layer<BoxTransport>,
        L::Service: IntoBoxTransport,
    {
        self.connect_with(connect).await
    }
}
