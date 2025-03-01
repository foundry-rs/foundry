use alloy_primitives::B256;
use futures::{ready, Stream, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use std::{pin::Pin, task};
use tokio::sync::broadcast;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

/// A Subscription is a feed of notifications from the server, identified by a
/// local ID.
///
/// This type is mostly a wrapper around [`broadcast::Receiver`], and exposes
/// the same methods.
#[derive(Debug)]
pub struct RawSubscription {
    /// The channel via which notifications are received.
    pub(crate) rx: broadcast::Receiver<Box<RawValue>>,
    /// The local ID of the subscription.
    pub(crate) local_id: B256,
}

impl RawSubscription {
    /// Get the local ID of the subscription.
    pub const fn local_id(&self) -> &B256 {
        &self.local_id
    }

    /// Wrapper for [`blocking_recv`]. Block the current thread until a message
    /// is available.
    ///
    /// [`blocking_recv`]: broadcast::Receiver::blocking_recv
    pub fn blocking_recv(&mut self) -> Result<Box<RawValue>, broadcast::error::RecvError> {
        self.rx.blocking_recv()
    }

    /// Returns `true` if the broadcast channel is empty (i.e. there are
    /// currently no notifications to receive).
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    /// Returns the number of messages in the broadcast channel that this
    /// receiver has yet to receive.
    pub fn len(&self) -> usize {
        self.rx.len()
    }

    /// Wrapper for [`recv`]. Await an item from the channel.
    ///
    /// [`recv`]: broadcast::Receiver::recv
    pub async fn recv(&mut self) -> Result<Box<RawValue>, broadcast::error::RecvError> {
        self.rx.recv().await
    }

    /// Wrapper for [`resubscribe`]. Create a new Subscription, starting from
    /// the current tail element.
    ///
    /// [`resubscribe`]: broadcast::Receiver::resubscribe
    pub fn resubscribe(&self) -> Self {
        Self { rx: self.rx.resubscribe(), local_id: self.local_id }
    }

    /// Wrapper for [`same_channel`]. Returns `true` if the two subscriptions
    /// share the same broadcast channel.
    ///
    /// [`same_channel`]: broadcast::Receiver::same_channel
    pub fn same_channel(&self, other: &Self) -> bool {
        self.rx.same_channel(&other.rx)
    }

    /// Wrapper for [`try_recv`]. Attempt to receive a message from the channel
    /// without awaiting.
    ///
    /// [`try_recv`]: broadcast::Receiver::try_recv
    pub fn try_recv(&mut self) -> Result<Box<RawValue>, broadcast::error::TryRecvError> {
        self.rx.try_recv()
    }

    /// Convert the subscription into a stream.
    pub fn into_stream(self) -> BroadcastStream<Box<RawValue>> {
        self.rx.into()
    }

    /// Convert into a typed subscription.
    pub fn into_typed<T>(self) -> Subscription<T> {
        self.into()
    }
}

/// An item in a typed [`Subscription`]. This is either the expected type, or
/// some serialized value of another type.
#[derive(Debug)]
pub enum SubscriptionItem<T> {
    /// The expected item.
    Item(T),
    /// Some other value.
    Other(Box<RawValue>),
}

impl<T: DeserializeOwned> From<Box<RawValue>> for SubscriptionItem<T> {
    fn from(value: Box<RawValue>) -> Self {
        serde_json::from_str(value.get()).map_or_else(
            |_| {
                trace!(value = value.get(), "Received unexpected value in subscription.");
                Self::Other(value)
            },
            |item| Self::Item(item),
        )
    }
}

/// A Subscription is a feed of notifications from the server of a specific
/// type `T`, identified by a local ID.
///
/// For flexibility, we expose three similar APIs:
/// - The [`Subscription::recv`] method and its variants will discard any notifications of
///   unexpected types.
/// - The [`Subscription::recv_any`] and its variants will yield unexpected types as
///   [`SubscriptionItem::Other`].
/// - The [`Subscription::recv_result`] and its variants will attempt to deserialize the
///   notifications and yield the `serde_json::Result` of the deserialization.
#[derive(Debug)]
#[must_use]
pub struct Subscription<T> {
    pub(crate) inner: RawSubscription,
    _pd: std::marker::PhantomData<T>,
}

impl<T> From<RawSubscription> for Subscription<T> {
    fn from(inner: RawSubscription) -> Self {
        Self { inner, _pd: std::marker::PhantomData }
    }
}

impl<T> Subscription<T> {
    /// Get the local ID of the subscription.
    pub const fn local_id(&self) -> &B256 {
        self.inner.local_id()
    }

    /// Convert the subscription into its inner [`RawSubscription`].
    pub fn into_raw(self) -> RawSubscription {
        self.inner
    }

    /// Get a reference to the inner subscription.
    pub const fn inner(&self) -> &RawSubscription {
        &self.inner
    }

    /// Get a mutable reference to the inner subscription.
    pub fn inner_mut(&mut self) -> &mut RawSubscription {
        &mut self.inner
    }

    /// Returns `true` if the broadcast channel is empty (i.e. there are
    /// currently no notifications to receive).
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the number of messages in the broadcast channel that this
    /// receiver has yet to receive.
    ///
    /// NB: This count may include messages of unexpected types that will be
    /// discarded upon receipt.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Wrapper for [`resubscribe`]. Create a new [`RawSubscription`], starting
    /// from the current tail element.
    ///
    /// [`resubscribe`]: broadcast::Receiver::resubscribe
    pub fn resubscribe_inner(&self) -> RawSubscription {
        self.inner.resubscribe()
    }

    /// Wrapper for [`resubscribe`]. Create a new `Subscription`, starting from
    /// the current tail element.
    ///
    /// [`resubscribe`]: broadcast::Receiver::resubscribe
    pub fn resubscribe(&self) -> Self {
        self.inner.resubscribe().into()
    }

    /// Wrapper for [`same_channel`]. Returns `true` if the two subscriptions
    /// share the same broadcast channel.
    ///
    /// [`same_channel`]: broadcast::Receiver::same_channel
    pub fn same_channel<U>(&self, other: &Subscription<U>) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

impl<T: DeserializeOwned> Subscription<T> {
    /// Wrapper for [`blocking_recv`], may produce unexpected values. Block the
    /// current thread until a message is available.
    ///
    /// [`blocking_recv`]: broadcast::Receiver::blocking_recv
    pub fn blocking_recv_any(
        &mut self,
    ) -> Result<SubscriptionItem<T>, broadcast::error::RecvError> {
        self.inner.blocking_recv().map(Into::into)
    }

    /// Wrapper for [`recv`], may produce unexpected values. Await an item from
    /// the channel.
    ///
    /// [`recv`]: broadcast::Receiver::recv
    pub async fn recv_any(&mut self) -> Result<SubscriptionItem<T>, broadcast::error::RecvError> {
        self.inner.recv().await.map(Into::into)
    }

    /// Wrapper for [`try_recv`]. Attempt to receive a message from the channel
    /// without awaiting.
    ///
    /// [`try_recv`]: broadcast::Receiver::try_recv
    pub fn try_recv_any(&mut self) -> Result<SubscriptionItem<T>, broadcast::error::TryRecvError> {
        self.inner.try_recv().map(Into::into)
    }

    /// Convert the subscription into a stream.
    ///
    /// Errors are logged and ignored.
    pub fn into_stream(self) -> SubscriptionStream<T> {
        SubscriptionStream {
            id: self.inner.local_id,
            inner: self.inner.into_stream(),
            _pd: std::marker::PhantomData,
        }
    }

    /// Convert the subscription into a stream that returns deserialization results.
    pub fn into_result_stream(self) -> SubResultStream<T> {
        SubResultStream {
            id: self.inner.local_id,
            inner: self.inner.into_stream(),
            _pd: std::marker::PhantomData,
        }
    }

    /// Convert the subscription into a stream that may yield unexpected types.
    pub fn into_any_stream(self) -> SubAnyStream<T> {
        SubAnyStream {
            id: self.inner.local_id,
            inner: self.inner.into_stream(),
            _pd: std::marker::PhantomData,
        }
    }

    /// Wrapper for [`blocking_recv`]. Block the current thread until a message
    /// of the expected type is available.
    ///
    /// [`blocking_recv`]: broadcast::Receiver::blocking_recv
    pub fn blocking_recv(&mut self) -> Result<T, broadcast::error::RecvError> {
        loop {
            match self.blocking_recv_any()? {
                SubscriptionItem::Item(item) => return Ok(item),
                SubscriptionItem::Other(_) => continue,
            }
        }
    }

    /// Wrapper for [`recv`]. Await an item of the expected type from the
    /// channel.
    ///
    /// [`recv`]: broadcast::Receiver::recv
    pub async fn recv(&mut self) -> Result<T, broadcast::error::RecvError> {
        loop {
            match self.recv_any().await? {
                SubscriptionItem::Item(item) => return Ok(item),
                SubscriptionItem::Other(_) => continue,
            }
        }
    }

    /// Wrapper for [`try_recv`]. Attempt to receive a message of the expected
    /// type from the channel without awaiting.
    ///
    /// [`try_recv`]: broadcast::Receiver::try_recv
    pub fn try_recv(&mut self) -> Result<T, broadcast::error::TryRecvError> {
        loop {
            match self.try_recv_any()? {
                SubscriptionItem::Item(item) => return Ok(item),
                SubscriptionItem::Other(_) => continue,
            }
        }
    }

    /// Wrapper for [`blocking_recv`]. Block the current thread until a message
    /// is available, deserializing the message and returning the result.
    ///
    /// [`blocking_recv`]: broadcast::Receiver::blocking_recv
    pub fn blocking_recv_result(
        &mut self,
    ) -> Result<Result<T, serde_json::Error>, broadcast::error::RecvError> {
        self.inner.blocking_recv().map(|value| serde_json::from_str(value.get()))
    }

    /// Wrapper for [`recv`]. Await an item from the channel, deserializing the
    /// message and returning the result.
    ///
    /// [`recv`]: broadcast::Receiver::recv
    pub async fn recv_result(
        &mut self,
    ) -> Result<Result<T, serde_json::Error>, broadcast::error::RecvError> {
        self.inner.recv().await.map(|value| serde_json::from_str(value.get()))
    }

    /// Wrapper for [`try_recv`]. Attempt to receive a message from the channel
    /// without awaiting, deserializing the message and returning the result.
    ///
    /// [`try_recv`]: broadcast::Receiver::try_recv
    pub fn try_recv_result(
        &mut self,
    ) -> Result<Result<T, serde_json::Error>, broadcast::error::TryRecvError> {
        self.inner.try_recv().map(|value| serde_json::from_str(value.get()))
    }
}

/// A stream of notifications from the server, identified by a local ID. This
/// stream may yield unexpected types.
#[derive(Debug)]
pub struct SubAnyStream<T> {
    id: B256,
    inner: BroadcastStream<Box<RawValue>>,
    _pd: std::marker::PhantomData<fn() -> T>,
}

impl<T> SubAnyStream<T> {
    /// Get the local ID of the subscription.
    pub const fn id(&self) -> &B256 {
        &self.id
    }
}

impl<T: DeserializeOwned> Stream for SubAnyStream<T> {
    type Item = SubscriptionItem<T>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Option<Self::Item>> {
        loop {
            match ready!(self.inner.poll_next_unpin(cx)) {
                Some(Ok(value)) => return task::Poll::Ready(Some(value.into())),
                Some(Err(err @ BroadcastStreamRecvError::Lagged(_))) => {
                    // This is OK.
                    debug!(%err, %self.id, "stream lagged");
                    continue;
                }
                None => return task::Poll::Ready(None),
            }
        }
    }
}

/// A stream of notifications from the server, identified by a local ID. This/
/// stream will yield only the expected type, discarding any notifications of
/// unexpected types.
#[derive(Debug)]
pub struct SubscriptionStream<T> {
    id: B256,
    inner: BroadcastStream<Box<RawValue>>,
    _pd: std::marker::PhantomData<fn() -> T>,
}

impl<T> SubscriptionStream<T> {
    /// Get the local ID of the subscription.
    pub const fn id(&self) -> &B256 {
        &self.id
    }
}

impl<T: DeserializeOwned> Stream for SubscriptionStream<T> {
    type Item = T;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Option<Self::Item>> {
        loop {
            match ready!(self.inner.poll_next_unpin(cx)) {
                Some(Ok(value)) => match serde_json::from_str(value.get()) {
                    Ok(item) => return task::Poll::Ready(Some(item)),
                    Err(err) => {
                        debug!(value = ?value.get(), %err, %self.id, "failed deserializing subscription item");
                        error!(%err, %self.id, "failed deserializing subscription item");
                        continue;
                    }
                },
                Some(Err(err @ BroadcastStreamRecvError::Lagged(_))) => {
                    // This is OK.
                    debug!(%err, %self.id, "stream lagged");
                    continue;
                }
                None => return task::Poll::Ready(None),
            }
        }
    }
}

/// A stream of notifications from the server, identified by a local ID.
///
/// This stream will attempt to deserialize the notifications and yield the [`serde_json::Result`]
/// of the deserialization.
#[derive(Debug)]
pub struct SubResultStream<T> {
    id: B256,
    inner: BroadcastStream<Box<RawValue>>,
    _pd: std::marker::PhantomData<fn() -> T>,
}

impl<T> SubResultStream<T> {
    /// Get the local ID of the subscription.
    pub const fn id(&self) -> &B256 {
        &self.id
    }
}

impl<T: DeserializeOwned> Stream for SubResultStream<T> {
    type Item = serde_json::Result<T>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Option<Self::Item>> {
        loop {
            match ready!(self.inner.poll_next_unpin(cx)) {
                Some(Ok(value)) => {
                    return task::Poll::Ready(Some(serde_json::from_str(value.get())))
                }
                Some(Err(err @ BroadcastStreamRecvError::Lagged(_))) => {
                    // This is OK.
                    debug!(%err, %self.id, "stream lagged");
                    continue;
                }
                None => return task::Poll::Ready(None),
            }
        }
    }
}
