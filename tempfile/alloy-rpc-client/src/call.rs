use alloy_json_rpc::{
    transform_response, try_deserialize_ok, Request, RequestPacket, ResponsePacket, RpcRecv,
    RpcResult, RpcSend,
};
use alloy_transport::{BoxTransport, IntoBoxTransport, RpcFut, TransportError, TransportResult};
use core::panic;
use futures::FutureExt;
use serde_json::value::RawValue;
use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{self, ready, Poll::Ready},
};
use tower::Service;

/// The states of the [`RpcCall`] future.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project(project = CallStateProj)]
enum CallState<Params>
where
    Params: RpcSend,
{
    Prepared {
        request: Option<Request<Params>>,
        connection: BoxTransport,
    },
    AwaitingResponse {
        #[pin]
        fut: <BoxTransport as Service<RequestPacket>>::Future,
    },
    Complete,
}

impl<Params> Clone for CallState<Params>
where
    Params: RpcSend,
{
    fn clone(&self) -> Self {
        match self {
            Self::Prepared { request, connection } => {
                Self::Prepared { request: request.clone(), connection: connection.clone() }
            }
            _ => panic!("cloned after dispatch"),
        }
    }
}

impl<Params> fmt::Debug for CallState<Params>
where
    Params: RpcSend,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Prepared { .. } => "Prepared",
            Self::AwaitingResponse { .. } => "AwaitingResponse",
            Self::Complete => "Complete",
        })
    }
}

impl<Params> Future for CallState<Params>
where
    Params: RpcSend,
{
    type Output = TransportResult<Box<RawValue>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        loop {
            match self.as_mut().project() {
                CallStateProj::Prepared { connection, request } => {
                    if let Err(e) =
                        task::ready!(Service::<RequestPacket>::poll_ready(connection, cx))
                    {
                        self.set(Self::Complete);
                        return Ready(RpcResult::Err(e));
                    }

                    let request = request.take().expect("no request");
                    debug!(method=%request.meta.method, id=%request.meta.id, "sending request");
                    trace!(params_ty=%std::any::type_name::<Params>(), ?request, "full request");
                    let request = request.serialize();
                    let fut = match request {
                        Ok(request) => {
                            trace!(request=%request.serialized(), "serialized request");
                            connection.call(request.into())
                        }
                        Err(err) => {
                            trace!(?err, "failed to serialize request");
                            self.set(Self::Complete);
                            return Ready(RpcResult::Err(TransportError::ser_err(err)));
                        }
                    };
                    self.set(Self::AwaitingResponse { fut });
                }
                CallStateProj::AwaitingResponse { fut } => {
                    let res = match task::ready!(fut.poll(cx)) {
                        Ok(ResponsePacket::Single(res)) => Ready(transform_response(res)),
                        Err(e) => Ready(RpcResult::Err(e)),
                        _ => panic!("received batch response from single request"),
                    };
                    self.set(Self::Complete);
                    return res;
                }
                CallStateProj::Complete => {
                    panic!("Polled after completion");
                }
            }
        }
    }
}

/// A prepared, but unsent, RPC call.
///
/// This is a future that will send the request when polled. It contains a
/// [`Request`], a [`BoxTransport`], and knowledge of its expected response
/// type. Upon awaiting, it will send the request and wait for the response. It
/// will then deserialize the response into the expected type.
///
/// Errors are captured in the [`RpcResult`] type. Rpc Calls will result in
/// either a successful response of the `Resp` type, an error response, or a
/// transport error.
///
/// ### Note
///
/// Serializing the request is done lazily. The request is not serialized until
/// the future is polled. This differs from the behavior of
/// [`crate::BatchRequest`], which serializes greedily. This is because the
/// batch request must immediately erase the `Param` type to allow batching of
/// requests with different `Param` types, while the `RpcCall` may do so lazily.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project]
#[derive(Clone)]
pub struct RpcCall<Params, Resp, Output = Resp, Map = fn(Resp) -> Output>
where
    Params: RpcSend,
    Map: FnOnce(Resp) -> Output,
{
    #[pin]
    state: CallState<Params>,
    map: Option<Map>,
    _pd: core::marker::PhantomData<fn() -> (Resp, Output)>,
}

impl<Params, Resp, Output, Map> core::fmt::Debug for RpcCall<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Map: FnOnce(Resp) -> Output,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RpcCall").field("state", &self.state).finish()
    }
}

impl<Params, Resp> RpcCall<Params, Resp>
where
    Params: RpcSend,
{
    #[doc(hidden)]
    pub fn new(req: Request<Params>, connection: impl IntoBoxTransport) -> Self {
        Self {
            state: CallState::Prepared {
                request: Some(req),
                connection: connection.into_box_transport(),
            },
            map: Some(std::convert::identity),
            _pd: PhantomData,
        }
    }
}

impl<Params, Resp, Output, Map> RpcCall<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Map: FnOnce(Resp) -> Output,
{
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
    pub fn map_resp<NewOutput, NewMap>(
        self,
        map: NewMap,
    ) -> RpcCall<Params, Resp, NewOutput, NewMap>
    where
        NewMap: FnOnce(Resp) -> NewOutput,
    {
        RpcCall { state: self.state, map: Some(map), _pd: PhantomData }
    }

    /// Returns `true` if the request is a subscription.
    ///
    /// # Panics
    ///
    /// Panics if called after the request has been sent.
    pub fn is_subscription(&self) -> bool {
        self.request().meta.is_subscription()
    }

    /// Set the request to be a non-standard subscription (i.e. not
    /// "eth_subscribe").
    ///
    /// # Panics
    ///
    /// Panics if called after the request has been sent.
    pub fn set_is_subscription(&mut self) {
        self.request_mut().meta.set_is_subscription();
    }

    /// Set the subscription status of the request.
    pub fn set_subscription_status(&mut self, status: bool) {
        self.request_mut().meta.set_subscription_status(status);
    }

    /// Get a mutable reference to the params of the request.
    ///
    /// This is useful for modifying the params after the request has been
    /// prepared.
    ///
    /// # Panics
    ///
    /// Panics if called after the request has been sent.
    pub fn params(&mut self) -> &mut Params {
        &mut self.request_mut().params
    }

    /// Returns a reference to the request.
    ///
    /// # Panics
    ///
    /// Panics if called after the request has been sent.
    pub fn request(&self) -> &Request<Params> {
        let CallState::Prepared { request, .. } = &self.state else {
            panic!("Cannot get request after request has been sent");
        };
        request.as_ref().expect("no request in prepared")
    }

    /// Returns a mutable reference to the request.
    ///
    /// # Panics
    ///
    /// Panics if called after the request has been sent.
    pub fn request_mut(&mut self) -> &mut Request<Params> {
        let CallState::Prepared { request, .. } = &mut self.state else {
            panic!("Cannot get request after request has been sent");
        };
        request.as_mut().expect("no request in prepared")
    }

    /// Map the params of the request into a new type.
    pub fn map_params<NewParams: RpcSend>(
        self,
        map: impl Fn(Params) -> NewParams,
    ) -> RpcCall<NewParams, Resp, Output, Map> {
        let CallState::Prepared { request, connection } = self.state else {
            panic!("Cannot get request after request has been sent");
        };
        let request = request.expect("no request in prepared").map_params(map);
        RpcCall {
            state: CallState::Prepared { request: Some(request), connection },
            map: self.map,
            _pd: PhantomData,
        }
    }
}

impl<Params, Resp, Output, Map> RpcCall<&Params, Resp, Output, Map>
where
    Params: RpcSend + ToOwned,
    Params::Owned: RpcSend,
    Map: FnOnce(Resp) -> Output,
{
    /// Convert this call into one with owned params, by cloning the params.
    ///
    /// # Panics
    ///
    /// Panics if called after the request has been polled.
    pub fn into_owned_params(self) -> RpcCall<Params::Owned, Resp, Output, Map> {
        let CallState::Prepared { request, connection } = self.state else {
            panic!("Cannot get params after request has been sent");
        };
        let request = request.expect("no request in prepared").into_owned_params();

        RpcCall {
            state: CallState::Prepared { request: Some(request), connection },
            map: self.map,
            _pd: PhantomData,
        }
    }
}

impl<'a, Params, Resp, Output, Map> RpcCall<Params, Resp, Output, Map>
where
    Params: RpcSend + 'a,
    Resp: RpcRecv,
    Output: 'static,
    Map: FnOnce(Resp) -> Output + Send + 'a,
{
    /// Convert this future into a boxed, pinned future, erasing its type.
    pub fn boxed(self) -> RpcFut<'a, Output> {
        Box::pin(self)
    }
}

impl<Params, Resp, Output, Map> Future for RpcCall<Params, Resp, Output, Map>
where
    Params: RpcSend,
    Resp: RpcRecv,
    Output: 'static,
    Map: FnOnce(Resp) -> Output,
{
    type Output = TransportResult<Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        trace!(?self.state, "polling RpcCall");

        let this = self.get_mut();
        let resp = try_deserialize_ok(ready!(this.state.poll_unpin(cx)));

        Ready(resp.map(this.map.take().expect("polled after completion")))
    }
}
