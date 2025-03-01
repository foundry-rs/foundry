use crate::{client::RpcClientInner, ClientRef};
use alloy_json_rpc::{
    transform_response, try_deserialize_ok, Id, Request, RequestPacket, ResponsePacket, RpcRecv,
    RpcSend, SerializedRequest,
};
use alloy_primitives::map::HashMap;
use alloy_transport::{
    BoxTransport, TransportError, TransportErrorKind, TransportFut, TransportResult,
};
use futures::FutureExt;
use pin_project::pin_project;
use serde_json::value::RawValue;
use std::{
    borrow::Cow,
    future::{Future, IntoFuture},
    marker::PhantomData,
    pin::Pin,
    task::{
        self, ready,
        Poll::{self, Ready},
    },
};
use tokio::sync::oneshot;
use tower::Service;

pub(crate) type Channel = oneshot::Sender<TransportResult<Box<RawValue>>>;
pub(crate) type ChannelMap = HashMap<Id, Channel>;

/// A batch JSON-RPC request, used to bundle requests into a single transport
/// call.
#[derive(Debug)]
#[must_use = "A BatchRequest does nothing unless sent via `send_batch` and `.await`"]
pub struct BatchRequest<'a> {
    /// The transport via which the batch will be sent.
    transport: ClientRef<'a>,

    /// The requests to be sent.
    requests: RequestPacket,

    /// The channels to send the responses through.
    channels: ChannelMap,
}

/// Awaits a single response for a request that has been included in a batch.
#[must_use = "A Waiter does nothing unless the corresponding BatchRequest is sent via `send_batch` and `.await`, AND the Waiter is awaited."]
#[pin_project]
#[derive(Debug)]
pub struct Waiter<Resp, Output = Resp, Map = fn(Resp) -> Output> {
    #[pin]
    rx: oneshot::Receiver<TransportResult<Box<RawValue>>>,
    map: Option<Map>,
    _resp: PhantomData<fn() -> (Output, Resp)>,
}

impl<Resp, Output, Map> Waiter<Resp, Output, Map> {
    /// Map the response to a different type. This is usable for converting
    /// the response to a more usable type, e.g. changing `U64` to `u64`.
    ///
    /// ## Note
    ///
    /// Carefully review the rust documentation on [fn pointers] before passing
    /// them to this function. Unless the pointer is specifically coerced to a
    /// `fn(_) -> _`, the `NewMap` will be inferred as that function's unique
    /// type. This can lead to confusing error messages.
    ///
    /// [fn pointers]: https://doc.rust-lang.org/std/primitive.fn.html#creating-function-pointers
    pub fn map_resp<NewOutput, NewMap>(self, map: NewMap) -> Waiter<Resp, NewOutput, NewMap>
    where
        NewMap: FnOnce(Resp) -> NewOutput,
    {
        Waiter { rx: self.rx, map: Some(map), _resp: PhantomData }
    }
}

impl<Resp> From<oneshot::Receiver<TransportResult<Box<RawValue>>>> for Waiter<Resp> {
    fn from(rx: oneshot::Receiver<TransportResult<Box<RawValue>>>) -> Self {
        Self { rx, map: Some(std::convert::identity), _resp: PhantomData }
    }
}

impl<Resp, Output, Map> std::future::Future for Waiter<Resp, Output, Map>
where
    Resp: RpcRecv,
    Map: FnOnce(Resp) -> Output,
{
    type Output = TransportResult<Output>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match ready!(this.rx.poll_unpin(cx)) {
            Ok(resp) => {
                let resp: Result<Resp, _> = try_deserialize_ok(resp);
                Ready(resp.map(this.map.take().expect("polled after completion")))
            }
            Err(e) => Poll::Ready(Err(TransportErrorKind::custom(e))),
        }
    }
}

#[pin_project::pin_project(project = CallStateProj)]
#[allow(unnameable_types, missing_debug_implementations)]
pub enum BatchFuture {
    Prepared {
        transport: BoxTransport,
        requests: RequestPacket,
        channels: ChannelMap,
    },
    SerError(Option<TransportError>),
    AwaitingResponse {
        channels: ChannelMap,
        #[pin]
        fut: TransportFut<'static>,
    },
    Complete,
}

impl<'a> BatchRequest<'a> {
    /// Create a new batch request.
    pub fn new(transport: &'a RpcClientInner) -> Self {
        Self {
            transport,
            requests: RequestPacket::Batch(Vec::with_capacity(10)),
            channels: HashMap::with_capacity_and_hasher(10, Default::default()),
        }
    }

    fn push_raw(
        &mut self,
        request: SerializedRequest,
    ) -> oneshot::Receiver<TransportResult<Box<RawValue>>> {
        let (tx, rx) = oneshot::channel();
        self.channels.insert(request.id().clone(), tx);
        self.requests.push(request);
        rx
    }

    fn push<Params: RpcSend, Resp: RpcRecv>(
        &mut self,
        request: Request<Params>,
    ) -> TransportResult<Waiter<Resp>> {
        let ser = request.serialize().map_err(TransportError::ser_err)?;
        Ok(self.push_raw(ser).into())
    }

    /// Add a call to the batch.
    ///
    /// ### Errors
    ///
    /// If the request cannot be serialized, this will return an error.
    pub fn add_call<Params: RpcSend, Resp: RpcRecv>(
        &mut self,
        method: impl Into<Cow<'static, str>>,
        params: &Params,
    ) -> TransportResult<Waiter<Resp>> {
        let request = self.transport.make_request(method, Cow::Borrowed(params));
        self.push(request)
    }

    /// Send the batch future via its connection.
    pub fn send(self) -> BatchFuture {
        BatchFuture::Prepared {
            transport: self.transport.transport.clone(),
            requests: self.requests,
            channels: self.channels,
        }
    }
}

impl IntoFuture for BatchRequest<'_> {
    type Output = <BatchFuture as Future>::Output;
    type IntoFuture = BatchFuture;

    fn into_future(self) -> Self::IntoFuture {
        self.send()
    }
}

impl BatchFuture {
    fn poll_prepared(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<<Self as Future>::Output> {
        let CallStateProj::Prepared { transport, requests, channels } = self.as_mut().project()
        else {
            unreachable!("Called poll_prepared in incorrect state")
        };

        if let Err(e) = task::ready!(transport.poll_ready(cx)) {
            self.set(Self::Complete);
            return Poll::Ready(Err(e));
        }

        // We only have mut refs, and we want ownership, so we just replace with 0-capacity
        // collections.
        let channels = std::mem::take(channels);
        let req = std::mem::replace(requests, RequestPacket::Batch(Vec::new()));

        let fut = transport.call(req);
        self.set(Self::AwaitingResponse { channels, fut });
        cx.waker().wake_by_ref();
        Poll::Pending
    }

    fn poll_awaiting_response(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<<Self as Future>::Output> {
        let CallStateProj::AwaitingResponse { channels, fut } = self.as_mut().project() else {
            unreachable!("Called poll_awaiting_response in incorrect state")
        };

        // Has the service responded yet?
        let responses = match ready!(fut.poll(cx)) {
            Ok(responses) => responses,
            Err(e) => {
                self.set(Self::Complete);
                return Poll::Ready(Err(e));
            }
        };

        // Send all responses via channels
        match responses {
            ResponsePacket::Single(single) => {
                if let Some(tx) = channels.remove(&single.id) {
                    let _ = tx.send(transform_response(single));
                }
            }
            ResponsePacket::Batch(responses) => {
                for response in responses {
                    if let Some(tx) = channels.remove(&response.id) {
                        let _ = tx.send(transform_response(response));
                    }
                }
            }
        }

        // Any channels remaining in the map are missing responses.
        // To avoid hanging futures, we send an error.
        for (id, tx) in channels.drain() {
            let _ = tx.send(Err(TransportErrorKind::missing_batch_response(id)));
        }

        self.set(Self::Complete);
        Poll::Ready(Ok(()))
    }

    fn poll_ser_error(
        mut self: Pin<&mut Self>,
        _cx: &mut task::Context<'_>,
    ) -> Poll<<Self as Future>::Output> {
        let e = if let CallStateProj::SerError(e) = self.as_mut().project() {
            e.take().expect("no error")
        } else {
            unreachable!("Called poll_ser_error in incorrect state")
        };

        self.set(Self::Complete);
        Poll::Ready(Err(e))
    }
}

impl Future for BatchFuture {
    type Output = TransportResult<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        if matches!(*self.as_mut(), Self::Prepared { .. }) {
            return self.poll_prepared(cx);
        }

        if matches!(*self.as_mut(), Self::AwaitingResponse { .. }) {
            return self.poll_awaiting_response(cx);
        }

        if matches!(*self.as_mut(), Self::SerError(_)) {
            return self.poll_ser_error(cx);
        }

        panic!("Called poll on CallState in invalid state")
    }
}
