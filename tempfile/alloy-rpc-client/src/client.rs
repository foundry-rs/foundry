use crate::{poller::PollerBuilder, BatchRequest, ClientBuilder, RpcCall};
use alloy_json_rpc::{Id, Request, RpcRecv, RpcSend};
use alloy_transport::{BoxTransport, IntoBoxTransport};
use std::{
    borrow::Cow,
    ops::Deref,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Weak,
    },
    time::Duration,
};
use tower::{layer::util::Identity, ServiceBuilder};

/// An [`RpcClient`] in a [`Weak`] reference.
pub type WeakClient = Weak<RpcClientInner>;

/// A borrowed [`RpcClient`].
pub type ClientRef<'a> = &'a RpcClientInner;

/// Parameter type of a JSON-RPC request with no parameters.
pub type NoParams = [(); 0];

#[cfg(feature = "pubsub")]
type MaybePubsub = Option<alloy_pubsub::PubSubFrontend>;

#[cfg(not(feature = "pubsub"))]
type MaybePubsub = Option<()>;

/// A JSON-RPC client.
///
/// [`RpcClient`] should never be instantiated directly. Instead, use
/// [`ClientBuilder`].
///
/// [`ClientBuilder`]: crate::ClientBuilder
#[derive(Debug)]
pub struct RpcClient(Arc<RpcClientInner>);

impl Clone for RpcClient {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl RpcClient {
    /// Create a new [`ClientBuilder`].
    pub const fn builder() -> ClientBuilder<Identity> {
        ClientBuilder { builder: ServiceBuilder::new() }
    }
}

impl RpcClient {
    /// Create a new [`RpcClient`] with an HTTP transport.
    #[cfg(feature = "reqwest")]
    pub fn new_http(url: reqwest::Url) -> Self {
        let http = alloy_transport_http::Http::new(url);
        let is_local = http.guess_local();
        Self::new(http, is_local)
    }

    /// Creates a new [`RpcClient`] with the given transport.
    pub fn new(t: impl IntoBoxTransport, is_local: bool) -> Self {
        Self::new_maybe_pubsub(t, is_local, None)
    }

    /// Creates a new [`RpcClient`] with the given transport and an optional [`MaybePubsub`].
    pub(crate) fn new_maybe_pubsub(
        t: impl IntoBoxTransport,
        is_local: bool,
        pubsub: MaybePubsub,
    ) -> Self {
        Self(Arc::new(RpcClientInner::new_maybe_pubsub(t, is_local, pubsub)))
    }

    /// Creates the [`RpcClient`] with the `main_transport` (ipc, ws, http) and a `layer` closure.
    ///
    /// The `layer` fn is intended to be [`tower::ServiceBuilder::service`] that layers the
    /// transport services. The `main_transport` is expected to the type that actually emits the
    /// request object: `PubSubFrontend`. This exists so that we can intercept the
    /// `PubSubFrontend` which we need for [`RpcClientInner::pubsub_frontend`].
    /// This workaround exists because due to how [`tower::ServiceBuilder::service`] collapses into
    /// a [`BoxTransport`] we wouldn't be obtain the [`MaybePubsub`] by downcasting the layered
    /// `transport`.
    pub(crate) fn new_layered<F, T, R>(is_local: bool, main_transport: T, layer: F) -> Self
    where
        F: FnOnce(T) -> R,
        T: IntoBoxTransport,
        R: IntoBoxTransport,
    {
        #[cfg(feature = "pubsub")]
        {
            let t = main_transport.clone().into_box_transport();
            let maybe_pubsub = t.as_any().downcast_ref::<alloy_pubsub::PubSubFrontend>().cloned();
            Self::new_maybe_pubsub(layer(main_transport), is_local, maybe_pubsub)
        }

        #[cfg(not(feature = "pubsub"))]
        Self::new(layer(main_transport), is_local)
    }

    /// Creates a new [`RpcClient`] with the given inner client.
    pub fn from_inner(inner: RpcClientInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Get a reference to the client.
    pub const fn inner(&self) -> &Arc<RpcClientInner> {
        &self.0
    }

    /// Convert the client into its inner type.
    pub fn into_inner(self) -> Arc<RpcClientInner> {
        self.0
    }

    /// Get a [`Weak`] reference to the client.
    pub fn get_weak(&self) -> WeakClient {
        Arc::downgrade(&self.0)
    }

    /// Borrow the client.
    pub fn get_ref(&self) -> ClientRef<'_> {
        &self.0
    }

    /// Sets the poll interval for the client in milliseconds.
    ///
    /// Note: This will only set the poll interval for the client if it is the only reference to the
    /// inner client. If the reference is held by many, then it will not update the poll interval.
    pub fn with_poll_interval(self, poll_interval: Duration) -> Self {
        self.inner().set_poll_interval(poll_interval);
        self
    }

    /// Build a poller that polls a method with the given parameters.
    ///
    /// See [`PollerBuilder`] for examples and more details.
    pub fn prepare_static_poller<Params, Resp>(
        &self,
        method: impl Into<Cow<'static, str>>,
        params: Params,
    ) -> PollerBuilder<Params, Resp>
    where
        Params: RpcSend + 'static,
        Resp: RpcRecv + Clone,
    {
        PollerBuilder::new(self.get_weak(), method, params)
    }

    /// Boxes the transport.
    #[deprecated(since = "0.9.0", note = "`RpcClient` is now always boxed")]
    #[allow(clippy::missing_const_for_fn)]
    pub fn boxed(self) -> Self {
        self
    }

    /// Create a new [`BatchRequest`] builder.
    #[inline]
    pub fn new_batch(&self) -> BatchRequest<'_> {
        BatchRequest::new(&self.0)
    }
}

impl Deref for RpcClient {
    type Target = RpcClientInner;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A JSON-RPC client.
///
/// This struct manages a [`BoxTransport`] and a request ID counter. It is used to
/// build [`RpcCall`] and [`BatchRequest`] objects. The client delegates
/// transport access to the calls.
///
/// ### Note
///
/// IDs are allocated sequentially, starting at 0. IDs are reserved via
/// [`RpcClientInner::next_id`]. Note that allocated IDs may not be used. There
/// is no guarantee that a prepared [`RpcCall`] will be sent, or that a sent
/// call will receive a response.
#[derive(Debug)]
pub struct RpcClientInner {
    /// The underlying transport.
    pub(crate) transport: BoxTransport,
    /// Stores a handle to the PubSub service if pubsub.
    ///
    /// We store this _transport_ because if built through the [`ClientBuilder`] with an additional
    /// layer the actual transport can be an arbitrary type and we would be unable to obtain the
    /// `PubSubFrontend` by downcasting the `transport`. For example
    /// `RetryTransport<PubSubFrontend>`.
    #[allow(unused)]
    pub(crate) pubsub: MaybePubsub,
    /// `true` if the transport is local.
    pub(crate) is_local: bool,
    /// The next request ID to use.
    pub(crate) id: AtomicU64,
    /// The poll interval for the client in milliseconds.
    pub(crate) poll_interval: AtomicU64,
}

impl RpcClientInner {
    /// Create a new [`RpcClient`] with the given transport.
    ///
    /// Note: Sets the poll interval to 250ms for local transports and 7s for remote transports by
    /// default.
    #[inline]
    pub fn new(t: impl IntoBoxTransport, is_local: bool) -> Self {
        Self {
            transport: t.into_box_transport(),
            pubsub: None,
            is_local,
            id: AtomicU64::new(0),
            poll_interval: if is_local { AtomicU64::new(250) } else { AtomicU64::new(7000) },
        }
    }

    /// Create a new [`RpcClient`] with the given transport and an optional handle to the
    /// `PubSubFrontend`.
    pub(crate) fn new_maybe_pubsub(
        t: impl IntoBoxTransport,
        is_local: bool,
        pubsub: MaybePubsub,
    ) -> Self {
        Self { pubsub, ..Self::new(t.into_box_transport(), is_local) }
    }

    /// Sets the starting ID for the client.
    #[inline]
    pub fn with_id(self, id: u64) -> Self {
        Self { id: AtomicU64::new(id), ..self }
    }

    /// Returns the default poll interval (milliseconds) for the client.
    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.poll_interval.load(Ordering::Relaxed))
    }

    /// Set the poll interval for the client in milliseconds. Default:
    /// 7s for remote and 250ms for local transports.
    pub fn set_poll_interval(&self, poll_interval: Duration) {
        self.poll_interval.store(poll_interval.as_millis() as u64, Ordering::Relaxed);
    }

    /// Returns a reference to the underlying transport.
    #[inline]
    pub const fn transport(&self) -> &BoxTransport {
        &self.transport
    }

    /// Returns a mutable reference to the underlying transport.
    #[inline]
    pub fn transport_mut(&mut self) -> &mut BoxTransport {
        &mut self.transport
    }

    /// Consumes the client and returns the underlying transport.
    #[inline]
    pub fn into_transport(self) -> BoxTransport {
        self.transport
    }

    /// Returns a reference to the pubsub frontend if the transport supports it.
    #[cfg(feature = "pubsub")]
    #[inline]
    #[track_caller]
    pub fn pubsub_frontend(&self) -> Option<&alloy_pubsub::PubSubFrontend> {
        if let Some(pubsub) = &self.pubsub {
            return Some(pubsub);
        }
        self.transport.as_any().downcast_ref::<alloy_pubsub::PubSubFrontend>()
    }

    /// Returns a reference to the pubsub frontend if the transport supports it.
    ///
    /// # Panics
    ///
    /// Panics if the transport does not support pubsub.
    #[cfg(feature = "pubsub")]
    #[inline]
    #[track_caller]
    pub fn expect_pubsub_frontend(&self) -> &alloy_pubsub::PubSubFrontend {
        self.pubsub_frontend().expect("called pubsub_frontend on a non-pubsub transport")
    }

    /// Build a `JsonRpcRequest` with the given method and params.
    ///
    /// This function reserves an ID for the request, however the request is not sent.
    ///
    /// To send a request, use [`RpcClientInner::request`] and await the returned [`RpcCall`].
    #[inline]
    pub fn make_request<Params: RpcSend>(
        &self,
        method: impl Into<Cow<'static, str>>,
        params: Params,
    ) -> Request<Params> {
        Request::new(method, self.next_id(), params)
    }

    /// `true` if the client believes the transport is local.
    ///
    /// This can be used to optimize remote API usage, or to change program
    /// behavior on local endpoints. When the client is instantiated by parsing
    /// a URL or other external input, this value is set on a best-efforts
    /// basis and may be incorrect.
    #[inline]
    pub const fn is_local(&self) -> bool {
        self.is_local
    }

    /// Set the `is_local` flag.
    #[inline]
    pub fn set_local(&mut self, is_local: bool) {
        self.is_local = is_local;
    }

    /// Reserve a request ID value. This is used to generate request IDs.
    #[inline]
    fn increment_id(&self) -> u64 {
        self.id.fetch_add(1, Ordering::Relaxed)
    }

    /// Reserve a request ID u64.
    #[inline]
    pub fn next_id(&self) -> Id {
        self.increment_id().into()
    }

    /// Prepares an [`RpcCall`].
    ///
    /// This function reserves an ID for the request, however the request is not sent.
    /// To send a request, await the returned [`RpcCall`].
    ///
    /// # Note
    ///
    /// Serialization is done lazily. It will not be performed until the call is awaited.
    /// This means that if a serializer error occurs, it will not be caught until the call is
    /// awaited.
    #[doc(alias = "prepare")]
    pub fn request<Params: RpcSend, Resp: RpcRecv>(
        &self,
        method: impl Into<Cow<'static, str>>,
        params: Params,
    ) -> RpcCall<Params, Resp> {
        let request = self.make_request(method, params);
        RpcCall::new(request, self.transport.clone())
    }

    /// Prepares an [`RpcCall`] with no parameters.
    ///
    /// See [`request`](Self::request) for more details.
    pub fn request_noparams<Resp: RpcRecv>(
        &self,
        method: impl Into<Cow<'static, str>>,
    ) -> RpcCall<NoParams, Resp> {
        self.request(method, [])
    }

    /// Type erase the service in the transport, allowing it to be used in a
    /// generic context.
    #[deprecated(since = "0.9.0", note = "`RpcClientInner` is now always boxed")]
    #[allow(clippy::missing_const_for_fn)]
    pub fn boxed(self) -> Self {
        self
    }
}

#[cfg(feature = "pubsub")]
mod pubsub_impl {
    use super::*;
    use alloy_pubsub::{PubSubConnect, RawSubscription, Subscription};
    use alloy_transport::TransportResult;

    impl RpcClientInner {
        /// Get a [`RawSubscription`] for the given subscription ID.
        ///
        /// # Panics
        ///
        /// Panics if the transport does not support pubsub.
        pub async fn get_raw_subscription(&self, id: alloy_primitives::B256) -> RawSubscription {
            self.expect_pubsub_frontend().get_subscription(id).await.unwrap()
        }

        /// Get a [`Subscription`] for the given subscription ID.
        ///
        /// # Panics
        ///
        /// Panics if the transport does not support pubsub.
        pub async fn get_subscription<T: serde::de::DeserializeOwned>(
            &self,
            id: alloy_primitives::B256,
        ) -> Subscription<T> {
            Subscription::from(self.get_raw_subscription(id).await)
        }
    }

    impl RpcClient {
        /// Connect to a transport via a [`PubSubConnect`] implementor.
        pub async fn connect_pubsub<C: PubSubConnect>(connect: C) -> TransportResult<Self> {
            ClientBuilder::default().pubsub(connect).await
        }

        /// Get the currently configured channel size. This is the number of items
        /// to buffer in new subscription channels. Defaults to 16. See
        /// [`tokio::sync::broadcast`] for a description of relevant
        /// behavior.
        ///
        /// [`tokio::sync::broadcast`]: https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html
        ///
        /// # Panics
        ///
        /// Panics if the transport does not support pubsub.
        #[track_caller]
        pub fn channel_size(&self) -> usize {
            self.expect_pubsub_frontend().channel_size()
        }

        /// Set the channel size.
        ///
        /// # Panics
        ///
        /// Panics if the transport does not support pubsub.
        #[track_caller]
        pub fn set_channel_size(&self, size: usize) {
            self.expect_pubsub_frontend().set_channel_size(size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    fn test_client_with_poll_interval() {
        let poll_interval = Duration::from_millis(5_000);
        let client = RpcClient::new_http(reqwest::Url::parse("http://localhost").unwrap())
            .with_poll_interval(poll_interval);
        assert_eq!(client.poll_interval(), poll_interval);
    }
}
