use crate::{ix::PubSubInstruction, managers::InFlight, RawSubscription};
use alloy_json_rpc::{RequestPacket, Response, ResponsePacket, SerializedRequest};
use alloy_primitives::B256;
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use futures::{future::try_join_all, FutureExt, TryFutureExt};
use std::{
    future::Future,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};
use tokio::sync::{mpsc, oneshot};

/// A `PubSubFrontend` is [`Transport`] composed of a channel to a running
/// PubSub service.
///
/// [`Transport`]: alloy_transport::Transport
#[derive(Debug)]
pub struct PubSubFrontend {
    tx: mpsc::UnboundedSender<PubSubInstruction>,
    /// The number of items to buffer in new subscription channels. Defaults to
    /// 16. See [`tokio::sync::broadcast::channel`] for a description.
    channel_size: AtomicUsize,
}

impl Clone for PubSubFrontend {
    fn clone(&self) -> Self {
        let channel_size = self.channel_size.load(Ordering::Relaxed);
        Self { tx: self.tx.clone(), channel_size: AtomicUsize::new(channel_size) }
    }
}

impl PubSubFrontend {
    /// Create a new frontend.
    pub(crate) const fn new(tx: mpsc::UnboundedSender<PubSubInstruction>) -> Self {
        Self { tx, channel_size: AtomicUsize::new(16) }
    }

    /// Get the subscription ID for a local ID.
    pub fn get_subscription(
        &self,
        id: B256,
    ) -> impl Future<Output = TransportResult<RawSubscription>> + Send + 'static {
        let backend_tx = self.tx.clone();
        async move {
            let (tx, rx) = oneshot::channel();
            backend_tx
                .send(PubSubInstruction::GetSub(id, tx))
                .map_err(|_| TransportErrorKind::backend_gone())?;
            rx.await.map_err(|_| TransportErrorKind::backend_gone())
        }
    }

    /// Unsubscribe from a subscription.
    pub fn unsubscribe(&self, id: B256) -> TransportResult<()> {
        self.tx
            .send(PubSubInstruction::Unsubscribe(id))
            .map_err(|_| TransportErrorKind::backend_gone())
    }

    /// Send a request.
    pub fn send(
        &self,
        req: SerializedRequest,
    ) -> impl Future<Output = TransportResult<Response>> + Send + 'static {
        let tx = self.tx.clone();
        let channel_size = self.channel_size.load(Ordering::Relaxed);

        async move {
            let (in_flight, rx) = InFlight::new(req, channel_size);
            tx.send(PubSubInstruction::Request(in_flight))
                .map_err(|_| TransportErrorKind::backend_gone())?;
            rx.await.map_err(|_| TransportErrorKind::backend_gone())?
        }
    }

    /// Send a packet of requests, by breaking it up into individual requests.
    ///
    /// Once all responses are received, we return a single response packet.
    pub fn send_packet(&self, req: RequestPacket) -> TransportFut<'static> {
        match req {
            RequestPacket::Single(req) => self.send(req).map_ok(ResponsePacket::Single).boxed(),
            RequestPacket::Batch(reqs) => try_join_all(reqs.into_iter().map(|req| self.send(req)))
                .map_ok(ResponsePacket::Batch)
                .boxed(),
        }
    }

    /// Get the currently configured channel size. This is the number of items
    /// to buffer in new subscription channels. Defaults to 16. See
    /// [`tokio::sync::broadcast`] for a description of relevant
    /// behavior.
    pub fn channel_size(&self) -> usize {
        self.channel_size.load(Ordering::Relaxed)
    }

    /// Set the channel size. This is the number of items to buffer in new
    /// subscription channels. Defaults to 16. See
    /// [`tokio::sync::broadcast`] for a description of relevant
    /// behavior.
    pub fn set_channel_size(&self, channel_size: usize) {
        debug_assert_ne!(channel_size, 0, "channel size must be non-zero");
        self.channel_size.store(channel_size, Ordering::Relaxed);
    }
}

impl tower::Service<RequestPacket> for PubSubFrontend {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let result =
            if self.tx.is_closed() { Err(TransportErrorKind::backend_gone()) } else { Ok(()) };
        Poll::Ready(result)
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.send_packet(req)
    }
}
