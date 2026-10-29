use crate::{RpcHandler, error::RequestError, handler::handle_request};
use anvil_rpc::{
    error::RpcError,
    request::{Id, Request},
    response::{Response, ResponseResult},
};

use futures::{FutureExt, Sink, SinkExt, Stream, StreamExt};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use std::{
    collections::VecDeque,
    fmt,
    hash::Hash,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// The general purpose trait for handling RPC requests and subscriptions
pub trait PubSubRpcHandler: Clone + Send + Sync + Unpin + 'static {
    /// The request type to expect
    type Request: DeserializeOwned + Send + Sync + fmt::Debug;
    /// The identifier to use for subscriptions
    type SubscriptionId: Hash + PartialEq + Eq + Send + Sync + fmt::Debug;
    /// The subscription type this handle may create
    type Subscription: Stream<Item = serde_json::Value> + Send + Sync + Unpin;

    /// Invoked when the request was received
    fn on_request(
        &self,
        request: Self::Request,
        cx: PubSubContext<Self>,
    ) -> impl Future<Output = ResponseResult> + Send;

    /// Invoked when a deserialized request returns an error from [`Self::on_request`].
    /// Parameters retain their original shape with fork URL credentials redacted.
    /// Deserialization errors are excluded because their messages can contain request parameters.
    fn on_request_error(
        &self,
        _method: &str,
        _id: &Id,
        _params: &serde_json::Value,
        _error: &RpcError,
    ) {
    }
}

type Subscriptions<SubscriptionId, Subscription> =
    Arc<Mutex<VecDeque<(SubscriptionId, Subscription)>>>;

/// Contains additional context and tracks subscriptions
pub struct PubSubContext<Handler: PubSubRpcHandler> {
    /// all active subscriptions `id -> Stream`
    subscriptions: Subscriptions<Handler::SubscriptionId, Handler::Subscription>,
}

impl<Handler: PubSubRpcHandler> PubSubContext<Handler> {
    /// Adds new active subscription
    ///
    /// Returns the previous subscription, if any
    pub fn add_subscription(
        &self,
        id: Handler::SubscriptionId,
        subscription: Handler::Subscription,
    ) -> Option<Handler::Subscription> {
        let mut subscriptions = self.subscriptions.lock();
        let mut removed = None;
        if let Some(idx) = subscriptions.iter().position(|(i, _)| id == *i) {
            trace!(target: "rpc", ?id,  "removed subscription");
            removed = Some(subscriptions.swap_remove_back(idx).unwrap().1);
        }
        trace!(target: "rpc", ?id,  "added subscription");
        subscriptions.push_back((id, subscription));
        removed
    }

    /// Removes an existing subscription
    pub fn remove_subscription(
        &self,
        id: &Handler::SubscriptionId,
    ) -> Option<Handler::Subscription> {
        let mut subscriptions = self.subscriptions.lock();
        if let Some(idx) = subscriptions.iter().position(|(i, _)| id == i) {
            trace!(target: "rpc", ?id,  "removed subscription");
            return Some(subscriptions.swap_remove_back(idx).unwrap().1);
        }
        None
    }
}

impl<Handler: PubSubRpcHandler> Clone for PubSubContext<Handler> {
    fn clone(&self) -> Self {
        Self { subscriptions: Arc::clone(&self.subscriptions) }
    }
}

impl<Handler: PubSubRpcHandler> Default for PubSubContext<Handler> {
    fn default() -> Self {
        Self { subscriptions: Arc::new(Mutex::new(VecDeque::new())) }
    }
}

/// A compatibility helper type to use common `RpcHandler` functions
struct ContextAwareHandler<Handler: PubSubRpcHandler> {
    handler: Handler,
    context: PubSubContext<Handler>,
}

impl<Handler: PubSubRpcHandler> Clone for ContextAwareHandler<Handler> {
    fn clone(&self) -> Self {
        Self { handler: self.handler.clone(), context: self.context.clone() }
    }
}

impl<Handler: PubSubRpcHandler> RpcHandler for ContextAwareHandler<Handler> {
    type Request = Handler::Request;

    fn on_request(&self, request: Self::Request) -> impl Future<Output = ResponseResult> + Send {
        self.handler.on_request(request, self.context.clone())
    }

    fn on_request_error(
        &self,
        method: &str,
        id: &Id,
        params: &serde_json::Value,
        error: &RpcError,
    ) {
        self.handler.on_request_error(method, id, params, error);
    }
}

/// Represents a connection to a client via websocket
///
/// Contains the state for the entire connection
pub struct PubSubConnection<Handler: PubSubRpcHandler, Connection> {
    /// the handler for the websocket connection
    handler: Handler,
    /// contains all the subscription related context
    context: PubSubContext<Handler>,
    /// The established connection
    connection: Connection,
    /// currently in progress requests
    processing: Vec<Pin<Box<dyn Future<Output = Option<Response>> + Send>>>,
    /// pending messages to send
    pending: VecDeque<String>,
}

impl<Handler: PubSubRpcHandler, Connection> PubSubConnection<Handler, Connection> {
    pub fn new(connection: Connection, handler: Handler) -> Self {
        Self {
            connection,
            handler,
            context: Default::default(),
            pending: Default::default(),
            processing: Default::default(),
        }
    }

    fn process_request(&mut self, req: serde_json::Result<Request>) {
        let handler =
            ContextAwareHandler { handler: self.handler.clone(), context: self.context.clone() };
        self.processing.push(Box::pin(async move {
            match req {
                Ok(req) => handle_request(req, handler).await,
                Err(err) => {
                    error!(target: "rpc", ?err, "invalid request");
                    let err = if err.is_syntax() || err.is_eof() {
                        RpcError::parse_error()
                    } else {
                        RpcError::invalid_request()
                    };
                    Some(Response::error(err))
                }
            }
        }));
    }
}

impl<Handler, Connection> Future for PubSubConnection<Handler, Connection>
where
    Handler: PubSubRpcHandler,
    Connection: Sink<String> + Stream<Item = Result<Option<Request>, RequestError>> + Unpin,
    <Connection as Sink<String>>::Error: fmt::Debug,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        loop {
            // drive the websocket
            loop {
                match pin.connection.poll_ready_unpin(cx) {
                    Poll::Ready(Ok(())) => {}
                    Poll::Ready(Err(err)) => {
                        trace!(target: "rpc", ?err, "connection err");
                        return Poll::Ready(());
                    }
                    Poll::Pending => break,
                }
                // only start sending if socket is ready
                if let Some(msg) = pin.pending.pop_front() {
                    if let Err(err) = pin.connection.start_send_unpin(msg) {
                        error!(target: "rpc", ?err, "Failed to send message");
                        return Poll::Ready(());
                    }
                } else {
                    break;
                }
            }

            // Ensure any pending messages are flushed
            // this needs to be called manually for tungsenite websocket: <https://github.com/foundry-rs/foundry/issues/6345>
            if let Poll::Ready(Err(err)) = pin.connection.poll_flush_unpin(cx) {
                trace!(target: "rpc", ?err, "websocket err");
                // close the connection
                return Poll::Ready(());
            }

            loop {
                match pin.connection.poll_next_unpin(cx) {
                    Poll::Ready(Some(req)) => match req {
                        Ok(Some(req)) => {
                            pin.process_request(Ok(req));
                        }
                        Err(err) => match err {
                            RequestError::Axum(err) => {
                                trace!(target: "rpc", ?err, "client disconnected");
                                return Poll::Ready(());
                            }
                            RequestError::Io(err) => {
                                trace!(target: "rpc", ?err, "client disconnected");
                                return Poll::Ready(());
                            }
                            RequestError::Serde(err) => {
                                pin.process_request(Err(err));
                            }
                            RequestError::Disconnect => {
                                trace!(target: "rpc", "client disconnected");
                                return Poll::Ready(());
                            }
                        },
                        _ => {}
                    },
                    Poll::Ready(None) => {
                        trace!(target: "rpc", "socket connection finished");
                        return Poll::Ready(());
                    }
                    Poll::Pending => break,
                }
            }

            let mut progress = false;
            for n in (0..pin.processing.len()).rev() {
                let mut req = pin.processing.swap_remove(n);
                #[allow(clippy::collapsible_match)]
                match req.poll_unpin(cx) {
                    Poll::Ready(Some(resp)) => {
                        if let Ok(text) = serde_json::to_string(&resp) {
                            pin.pending.push_back(text);
                            progress = true;
                        }
                    }
                    Poll::Ready(None) => {}
                    Poll::Pending => pin.processing.push(req),
                }
            }

            if pin.pending.is_empty() {
                // Keep notifications in their streams while the connection is backpressured.
                let mut subscriptions = pin.context.subscriptions.lock();
                'outer: for _ in 0..subscriptions.len() {
                    let (id, mut sub) = subscriptions.pop_back().unwrap();
                    'inner: loop {
                        match pin.connection.poll_ready_unpin(cx) {
                            Poll::Ready(Ok(())) => {}
                            Poll::Ready(Err(err)) => {
                                trace!(target: "rpc", ?err, "connection err");
                                return Poll::Ready(());
                            }
                            Poll::Pending => {
                                // Resume with the next subscription when the shared sink wakes us.
                                subscriptions.push_front((id, sub));
                                return Poll::Pending;
                            }
                        }
                        #[allow(clippy::collapsible_match)]
                        match sub.poll_next_unpin(cx) {
                            Poll::Ready(Some(res)) => {
                                if let Ok(text) = serde_json::to_string(&res) {
                                    if let Err(err) = pin.connection.start_send_unpin(text) {
                                        error!(target: "rpc", ?err, "Failed to send message");
                                        return Poll::Ready(());
                                    }
                                    progress = true;
                                }
                            }
                            Poll::Ready(None) => continue 'outer,
                            Poll::Pending => break 'inner,
                        }
                    }

                    subscriptions.push_front((id, sub));
                }
            }

            if !progress {
                return Poll::Pending;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_rpc::{
        request::{RequestParams, RpcCall, RpcNotification, Version},
        response::RpcResponse,
    };
    use std::{
        pin::pin,
        sync::atomic::{AtomicUsize, Ordering},
        task::{Wake, Waker},
    };

    #[derive(Clone, Default)]
    struct TestHandler {
        requests: Arc<AtomicUsize>,
        remove_on_request: Option<u64>,
    }

    impl PubSubRpcHandler for TestHandler {
        type Request = serde_json::Value;
        type SubscriptionId = u64;
        type Subscription = Pin<Box<dyn Stream<Item = serde_json::Value> + Send + Sync>>;

        async fn on_request(
            &self,
            _request: Self::Request,
            cx: PubSubContext<Self>,
        ) -> ResponseResult {
            self.requests.fetch_add(1, Ordering::Relaxed);
            if let Some(id) = self.remove_on_request {
                cx.remove_subscription(&id);
            }
            ResponseResult::success(serde_json::Value::Null)
        }
    }

    fn notification() -> RpcCall {
        RpcCall::Notification(RpcNotification {
            jsonrpc: Some(Version::V2),
            method: "eth_subscribe".to_owned(),
            params: RequestParams::None,
        })
    }

    fn run_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut future = pin!(future);
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future unexpectedly pending"),
        }
    }

    #[test]
    fn process_request_keeps_empty_batch_invalid() {
        let mut connection = PubSubConnection::new((), TestHandler::default());
        connection.process_request(Ok(Request::Batch(vec![])));

        let response = run_ready(connection.processing.pop().unwrap());
        assert_eq!(
            response,
            Some(Response::Single(RpcResponse::from(RpcError::invalid_request())))
        );
    }

    #[test]
    fn process_request_returns_parse_error_for_malformed_json() {
        let mut connection = PubSubConnection::new((), TestHandler::default());
        connection.process_request(serde_json::from_str("{"));

        let response = run_ready(connection.processing.pop().unwrap());
        assert_eq!(response, Some(Response::error(RpcError::parse_error())));
    }

    #[test]
    fn process_request_executes_notification_without_response() {
        let handler = TestHandler::default();
        let mut connection = PubSubConnection::new((), handler.clone());
        connection.process_request(Ok(Request::Batch(vec![notification()])));

        let response = run_ready(connection.processing.pop().unwrap());
        assert_eq!(response, None);
        assert_eq!(handler.requests.load(Ordering::Relaxed), 1);
    }

    #[derive(Default)]
    struct TestConnection {
        ready: bool,
        fail_next_send: bool,
        fail_after: Option<usize>,
        ready_polls: usize,
        sent: Vec<String>,
        requests: VecDeque<Request>,
        send_budget: Option<usize>,
        ready_waker: Option<Waker>,
        disconnected: bool,
    }

    impl Stream for TestConnection {
        type Item = Result<Option<Request>, RequestError>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.disconnected {
                return Poll::Ready(None);
            }
            match self.requests.pop_front() {
                Some(request) => Poll::Ready(Some(Ok(Some(request)))),
                None => Poll::Pending,
            }
        }
    }

    impl Sink<String> for TestConnection {
        type Error = &'static str;

        fn poll_ready(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            let polls = self.ready_polls;
            self.ready_polls += 1;
            if self.fail_after.is_some_and(|after| polls >= after) {
                return Poll::Ready(Err("sink failed"));
            }
            if self.ready {
                Poll::Ready(Ok(()))
            } else {
                self.ready_waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }

        fn start_send(mut self: Pin<&mut Self>, item: String) -> Result<(), Self::Error> {
            assert!(self.ready);
            if self.fail_next_send {
                self.fail_next_send = false;
                return Err("sink failed");
            }
            self.sent.push(item);
            if let Some(budget) = &mut self.send_budget {
                *budget -= 1;
                self.ready = *budget != 0;
            }
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn subscription_backpressure_preserves_events_and_request_progress() {
        let handler = TestHandler::default();
        let mut connection = PubSubConnection::new(TestConnection::default(), handler.clone());
        let events = (0..4).map(serde_json::Value::from).collect::<Vec<_>>();
        connection.context.add_subscription(1, Box::pin(futures::stream::iter(events.clone())));
        let mut cx = Context::from_waker(Waker::noop());

        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert!(connection.pending.is_empty(), "blocked subscriptions must retain their events");

        connection.connection.requests.push_back(Request::Single(notification()));
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), 1);
        assert!(connection.pending.is_empty());

        connection.connection.ready = true;
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert_eq!(
            connection.connection.sent,
            events.iter().map(|event| serde_json::to_string(event).unwrap()).collect::<Vec<_>>()
        );
        assert!(connection.pending.is_empty());
    }

    #[test]
    fn sink_readiness_error_terminates_connection() {
        for fail_after in [0, 1] {
            let transport =
                TestConnection { ready: true, fail_after: Some(fail_after), ..Default::default() };
            let mut connection = PubSubConnection::new(transport, TestHandler::default());
            connection.context.add_subscription(
                1,
                Box::pin(futures::stream::iter(vec![serde_json::Value::Null])),
            );
            let mut cx = Context::from_waker(Waker::noop());

            assert!(Pin::new(&mut connection).poll(&mut cx).is_ready());
            assert!(connection.connection.sent.is_empty());
        }
    }

    #[test]
    fn response_start_send_error_terminates_connection() {
        let transport = TestConnection { ready: true, fail_next_send: true, ..Default::default() };
        let mut connection = PubSubConnection::new(transport, TestHandler::default());
        connection.connection.requests.push_back(Request::Batch(vec![]));
        let mut cx = Context::from_waker(Waker::noop());

        assert!(Pin::new(&mut connection).poll(&mut cx).is_ready());
        assert!(connection.connection.sent.is_empty());
        assert!(connection.connection.poll_ready_unpin(&mut cx).is_ready());
    }

    #[test]
    fn subscription_backpressure_stops_readiness_scan() {
        for count in [1, 8, 64] {
            let mut connection =
                PubSubConnection::new(TestConnection::default(), TestHandler::default());
            for id in 0..count {
                connection.context.add_subscription(
                    id,
                    Box::pin(futures::stream::iter(vec![serde_json::Value::from(id)])),
                );
            }
            let mut cx = Context::from_waker(Waker::noop());

            assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
            // One check for pending responses and one for the shared subscription sink.
            assert_eq!(connection.connection.ready_polls, 2);
            assert!(connection.connection.sent.is_empty());
            assert!(connection.pending.is_empty());
            assert_eq!(connection.context.subscriptions.lock().len(), count as usize);

            connection.connection.ready = true;
            assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
            let mut sent = connection
                .connection
                .sent
                .iter()
                .map(|event| serde_json::from_str::<u64>(event).unwrap())
                .collect::<Vec<_>>();
            sent.sort_unstable();
            assert_eq!(sent, (0..count).collect::<Vec<_>>());
            assert!(connection.context.subscriptions.lock().is_empty());
        }
    }

    #[derive(Default)]
    struct TestWake(AtomicUsize);

    impl Wake for TestWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn subscription_backpressure_wakes_and_rotates_fairly() {
        let count = 3;
        let mut connection =
            PubSubConnection::new(TestConnection::default(), TestHandler::default());
        for id in 1..=count {
            let events = (0..if id == count { 12 } else { 3 })
                .map(|event| serde_json::json!([id, event]))
                .collect::<Vec<_>>();
            connection.context.add_subscription(id, Box::pin(futures::stream::iter(events)));
        }
        let wake = Arc::new(TestWake::default());
        let waker = Waker::from(wake.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());

        for window in 0..count * 3 {
            connection.connection.ready = true;
            connection.connection.send_budget = Some(1);
            connection.connection.ready_waker.take().unwrap().wake();
            assert_eq!(wake.0.swap(0, Ordering::Relaxed), 1);
            assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
            assert_eq!(connection.connection.sent.len(), window as usize + 1);
        }
        let sent = connection
            .connection
            .sent
            .iter()
            .map(|event| serde_json::from_str::<[u64; 2]>(event).unwrap())
            .collect::<Vec<_>>();
        let expected = (0..3)
            .flat_map(|event| (1..count).rev().chain([count]).map(move |id| [id, event]))
            .collect::<Vec<_>>();
        assert_eq!(sent, expected);

        connection.connection.ready = true;
        connection.connection.send_budget = None;
        connection.connection.ready_waker.take().unwrap().wake();
        assert_eq!(wake.0.swap(0, Ordering::Relaxed), 1);
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        let sent = connection
            .connection
            .sent
            .iter()
            .map(|event| serde_json::from_str::<[u64; 2]>(event).unwrap())
            .collect::<Vec<_>>();
        let expected =
            expected.into_iter().chain((3..12).map(|event| [count, event])).collect::<Vec<_>>();
        assert_eq!(sent, expected);
        assert!(connection.context.subscriptions.lock().is_empty());
    }

    #[test]
    fn subscription_rotation_skips_pending_and_finished_streams() {
        let transport = TestConnection { ready: true, send_budget: Some(1), ..Default::default() };
        let mut connection = PubSubConnection::new(transport, TestHandler::default());
        for id in 1..=2 {
            connection.context.add_subscription(
                id,
                Box::pin(futures::stream::iter(vec![serde_json::json!(id); 2])),
            );
        }
        connection.context.add_subscription(3, Box::pin(futures::stream::pending()));
        connection.context.add_subscription(4, Box::pin(futures::stream::empty()));
        let mut cx = Context::from_waker(Waker::noop());

        for _ in 0..4 {
            connection.connection.ready = true;
            connection.connection.send_budget = Some(1);
            assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        }
        assert_eq!(connection.connection.sent, ["2", "1", "2", "1"]);
        connection.connection.ready = true;
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert_eq!(connection.context.subscriptions.lock().len(), 1);
        assert!(connection.context.remove_subscription(&3).is_some());
    }

    #[test]
    fn subscription_backpressure_allows_unsubscribe_and_replacement() {
        let handler = TestHandler { remove_on_request: Some(1), ..Default::default() };
        let mut connection = PubSubConnection::new(TestConnection::default(), handler.clone());
        for id in 1..=3 {
            connection
                .context
                .add_subscription(id, Box::pin(futures::stream::iter(vec![serde_json::json!(id)])));
        }
        let mut cx = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());

        let mut replaced = connection
            .context
            .add_subscription(2, Box::pin(futures::stream::iter(vec![serde_json::json!(20)])))
            .unwrap();
        assert_eq!(run_ready(replaced.next()), Some(serde_json::json!(2)));
        connection.connection.requests.push_back(Request::Single(notification()));
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), 1);
        assert!(connection.context.remove_subscription(&1).is_none());
        assert!(connection.context.remove_subscription(&99).is_none());
        assert!(connection.connection.sent.is_empty());

        connection.connection.ready = true;
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert_eq!(connection.connection.sent, ["3", "20"]);
        assert!(connection.context.subscriptions.lock().is_empty());
    }

    #[test]
    fn subscription_backpressure_allows_disconnect() {
        let mut connection =
            PubSubConnection::new(TestConnection::default(), TestHandler::default());
        connection
            .context
            .add_subscription(1, Box::pin(futures::stream::iter(vec![serde_json::Value::Null])));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());

        connection.connection.disconnected = true;
        assert!(Pin::new(&mut connection).poll(&mut cx).is_ready());
        assert!(connection.connection.sent.is_empty());
    }
}
