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

    /// Whether an ended subscription requires closing the client connection.
    fn close_on_subscription_end(&self, _subscription: &Self::Subscription) -> bool {
        false
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
    pending: PendingMessages,
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
        // Cooperate with other connections even when subscription streams stay ready.
        let mut remaining = 256;
        loop {
            // drive the websocket
            let mut writable = true;
            loop {
                match pin.connection.poll_ready_unpin(cx) {
                    Poll::Ready(Ok(())) => {}
                    Poll::Ready(Err(err)) => {
                        trace!(target: "rpc", ?err, "connection err");
                        return Poll::Ready(());
                    }
                    Poll::Pending => {
                        writable = false;
                        break;
                    }
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

            // Pause reading while responses await the client, so a client that does not read is
            // throttled by transport backpressure instead of growing the backlog. Requests still
            // being processed have a much higher limit, since they may wait on a later request.
            while pin.pending.responses < MAX_PENDING_RESPONSES
                && pin.processing.len() < MAX_PROCESSING_REQUESTS
            {
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
                match req.poll_unpin(cx) {
                    Poll::Ready(resp) => {
                        // A finished request may reopen request reading, so always loop again.
                        progress = true;
                        if let Some(resp) = resp
                            && let Ok(text) = serde_json::to_string(&resp)
                            && !pin.pending.send(
                                &mut pin.connection,
                                cx,
                                text,
                                false,
                                &mut writable,
                            )
                        {
                            return Poll::Ready(());
                        }
                    }
                    Poll::Pending => pin.processing.push(req),
                }
            }

            {
                // Materialize events before upstream queues overflow or their backing data expires.
                let mut subscriptions = pin.context.subscriptions.lock();
                for _ in 0..subscriptions.len() {
                    let (id, mut sub) = subscriptions.pop_back().unwrap();
                    #[allow(clippy::collapsible_match)]
                    match sub.poll_next_unpin(cx) {
                        Poll::Ready(Some(res)) => {
                            if let Ok(text) = serde_json::to_string(&res) {
                                if !pin.pending.send(
                                    &mut pin.connection,
                                    cx,
                                    text,
                                    true,
                                    &mut writable,
                                ) {
                                    return Poll::Ready(());
                                }
                                progress = true;
                            }
                            remaining -= 1;
                        }
                        Poll::Ready(None) => {
                            if pin.handler.close_on_subscription_end(&sub) {
                                warn!(target: "rpc", ?id, "Closing connection after subscription ended");
                                return Poll::Ready(());
                            }
                            continue;
                        }
                        Poll::Pending => {}
                    }

                    subscriptions.push_front((id, sub));
                    if remaining == 0 {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                }
            }

            if !progress {
                return Poll::Pending;
            }
        }
    }
}

/// Serialized backlog shared by responses and subscription notifications.
///
/// Only notifications count toward the backlog limits, since they arrive without client requests.
/// Queued responses are bounded in count by pausing request reads instead.
#[derive(Default)]
struct PendingMessages {
    /// Queued messages and whether each is a notification.
    messages: VecDeque<(String, bool)>,
    /// Number of queued responses.
    responses: usize,
    /// Serialized bytes of queued notifications.
    bytes: usize,
}

impl PendingMessages {
    fn pop_front(&mut self) -> Option<String> {
        let (message, notification) = self.messages.pop_front()?;
        if notification {
            self.bytes -= message.len();
        } else {
            self.responses -= 1;
        }
        Some(message)
    }

    /// Sends directly while writable, otherwise queues the message, enforcing the backlog limits
    /// for notifications. Returns false when the connection must terminate.
    fn send<Connection>(
        &mut self,
        connection: &mut Connection,
        cx: &mut Context<'_>,
        message: String,
        notification: bool,
        writable: &mut bool,
    ) -> bool
    where
        Connection: Sink<String> + Unpin,
        Connection::Error: fmt::Debug,
    {
        if *writable {
            debug_assert!(self.messages.is_empty());
            match connection.poll_ready_unpin(cx) {
                Poll::Ready(Ok(())) => {
                    if let Err(err) = connection.start_send_unpin(message) {
                        error!(target: "rpc", ?err, "Failed to send message");
                        return false;
                    }
                    return true;
                }
                Poll::Ready(Err(err)) => {
                    trace!(target: "rpc", ?err, "connection err");
                    return false;
                }
                Poll::Pending => *writable = false,
            }
        }

        if !notification {
            self.responses += 1;
        } else if self.messages.len() - self.responses >= MAX_PENDING_MESSAGES
            || message.len() > MAX_PENDING_BYTES - self.bytes
        {
            warn!(target: "rpc", "Closing connection because its notification backlog is full");
            return false;
        } else {
            self.bytes += message.len();
        }
        self.messages.push_back((message, notification));
        true
    }
}

// Bound both serialized notification payloads and per-message allocation overhead for slow
// subscribers.
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
const MAX_PENDING_MESSAGES: usize = 4096;
// Bound the number of responses awaiting a slow client before request reads pause.
const MAX_PENDING_RESPONSES: usize = 256;
// Bound requests being processed, since their responses are queued without further checks.
const MAX_PROCESSING_REQUESTS: usize = 4096;

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_rpc::{
        request::{RequestParams, RpcCall, RpcMethodCall, RpcNotification, Version},
        response::RpcResponse,
    };
    use futures::channel::mpsc;
    use std::{
        pin::pin,
        sync::atomic::{AtomicUsize, Ordering},
        task::{Wake, Waker},
    };

    #[derive(Clone, Default)]
    struct TestHandler {
        requests: Arc<AtomicUsize>,
        remove_on_request: Option<u64>,
        close_on_end: bool,
        response: serde_json::Value,
        block: bool,
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
            if self.block {
                std::future::pending::<()>().await;
            }
            ResponseResult::success(self.response.clone())
        }

        fn close_on_subscription_end(&self, _subscription: &Self::Subscription) -> bool {
            self.close_on_end
        }
    }

    fn notification() -> RpcCall {
        RpcCall::Notification(RpcNotification {
            jsonrpc: Some(Version::V2),
            method: "eth_subscribe".to_owned(),
            params: RequestParams::None,
        })
    }

    fn method_call(id: i64) -> Request {
        Request::Single(RpcCall::MethodCall(RpcMethodCall {
            jsonrpc: Version::V2,
            method: "eth_chainId".to_owned(),
            params: RequestParams::None,
            id: Id::Number(id),
        }))
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
    fn sink_readiness_error_terminates_connection() {
        for fail_after in [0, 1] {
            let transport =
                TestConnection { ready: true, fail_after: Some(fail_after), ..Default::default() };
            let mut connection = PubSubConnection::new(transport, TestHandler::default());
            connection.context.add_subscription(
                1,
                Box::pin(futures::stream::iter(vec![serde_json::Value::Null])),
            );

            assert!(poll_once(&mut connection).is_ready());
            assert!(connection.connection.sent.is_empty());
        }
    }

    #[test]
    fn response_start_send_error_terminates_connection() {
        let transport = TestConnection { ready: true, fail_next_send: true, ..Default::default() };
        let mut connection = PubSubConnection::new(transport, TestHandler::default());
        connection.connection.requests.push_back(Request::Batch(vec![]));

        assert!(poll_once(&mut connection).is_ready());
        assert!(!connection.connection.fail_next_send);
    }

    #[test]
    fn subscription_backpressure_limits_readiness_scan_and_allows_disconnect() {
        for count in [1, 64] {
            let mut connection = connection();
            for id in 0..count {
                connection.context.add_subscription(
                    id,
                    Box::pin(futures::stream::iter(vec![serde_json::Value::from(id)])),
                );
            }

            assert!(poll_once(&mut connection).is_pending());
            // One readiness check per round, regardless of the number of subscriptions.
            assert_eq!(connection.connection.ready_polls, 2);
            assert!(connection.connection.sent.is_empty());
            assert_eq!(connection.pending.messages.len(), count as usize);
            assert!(connection.context.subscriptions.lock().is_empty());

            connection.connection.disconnected = true;
            assert!(poll_once(&mut connection).is_ready());
            assert!(connection.connection.sent.is_empty());
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
        let mut connection = connection();
        for id in 1..=count {
            let events = (0..if id == count { 12 } else { 3 })
                .map(|event| serde_json::json!([id, event]))
                .collect::<Vec<_>>();
            connection.context.add_subscription(id, Box::pin(futures::stream::iter(events)));
        }
        connection.context.add_subscription(count + 1, Box::pin(futures::stream::pending()));
        connection.context.add_subscription(count + 2, Box::pin(futures::stream::empty()));
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
        let mut sent = sent_events(&connection.connection);
        for (event, window) in sent.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            window.sort_unstable();
            assert_eq!(*window, [[1, event as u64], [2, event as u64], [3, event as u64]]);
        }

        connection.connection.ready = true;
        connection.connection.send_budget = None;
        connection.connection.ready_waker.take().unwrap().wake();
        assert_eq!(wake.0.swap(0, Ordering::Relaxed), 1);
        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        let sent = sent_events(&connection.connection);
        assert_eq!(sent.len(), 18);
        for id in 1..=count {
            let events = sent.iter().filter(|event| event[0] == id).map(|event| event[1]);
            assert_eq!(
                events.collect::<Vec<_>>(),
                (0..if id == count { 12 } else { 3 }).collect::<Vec<_>>()
            );
        }
        assert_eq!(sent[9..], (3..12).map(|event| [count, event]).collect::<Vec<_>>());
        assert_eq!(connection.context.subscriptions.lock().len(), 1);
        assert!(connection.context.remove_subscription(&(count + 1)).is_some());
    }

    #[test]
    fn subscription_end_closes_only_when_requested() {
        for close_on_end in [false, true] {
            let handler = TestHandler { close_on_end, ..Default::default() };
            let mut connection = PubSubConnection::new(TestConnection::default(), handler);
            connection.context.add_subscription(1, Box::pin(futures::stream::empty()));

            assert_eq!(poll_once(&mut connection).is_ready(), close_on_end);
            assert!(connection.connection.sent.is_empty());
        }
    }

    #[test]
    fn subscription_backpressure_allows_unsubscribe_and_replacement() {
        let handler = TestHandler { remove_on_request: Some(1), ..Default::default() };
        let mut connection = PubSubConnection::new(TestConnection::default(), handler.clone());
        let mut senders = Vec::new();
        for id in 1..=3 {
            let (tx, rx) = mpsc::unbounded();
            tx.unbounded_send(serde_json::json!(id)).unwrap();
            senders.push(tx);
            connection.context.add_subscription(id, Box::pin(rx));
        }
        let mut cx = Context::from_waker(Waker::noop());
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(connection.pending.messages.len(), 3);

        let mut replaced = connection
            .context
            .add_subscription(2, Box::pin(futures::stream::iter(vec![serde_json::json!(20)])))
            .unwrap();
        assert!(replaced.poll_next_unpin(&mut cx).is_pending());
        drop(replaced);
        connection.connection.requests.push_back(Request::Single(notification()));
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), 1);
        assert_eq!(connection.pending.messages.len(), 4);
        assert!(connection.context.remove_subscription(&1).is_none());
        assert!(connection.connection.sent.is_empty());
        assert!(senders[0].unbounded_send(serde_json::json!(10)).is_err());
        assert!(senders[1].unbounded_send(serde_json::json!(20)).is_err());
        senders[2].unbounded_send(serde_json::json!(30)).unwrap();

        connection.connection.ready = true;
        assert!(poll_once(&mut connection).is_pending());
        // Already materialized notifications precede later cancellation and replacement.
        assert_eq!(connection.connection.sent.len(), 5);
        let (initial, later) = connection.connection.sent.split_at_mut(3);
        initial.sort_unstable();
        assert_eq!(initial, ["1", "2", "3"]);
        assert_eq!(later, ["20", "30"]);
        assert!(connection.pending.messages.is_empty());
        assert_eq!(connection.context.subscriptions.lock().len(), 1);
    }

    #[test]
    fn subscription_backpressure_drains_bounded_upstream() {
        let (mut tx, rx) = mpsc::channel(2048);
        let mut connection = connection();
        connection.context.add_subscription(1, Box::pin(rx.map(serde_json::Value::from)));

        for event in 0..3000 {
            tx.try_send(event).expect("the blocked connection must keep draining its upstream");
            assert!(poll_once(&mut connection).is_pending());
        }

        connection.connection.ready = true;
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(
            connection.connection.sent,
            (0..3000).map(|event| event.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn subscription_backlog_overflow_terminates_connection() {
        let mut connection = connection();
        connection
            .context
            .add_subscription(1, Box::pin(futures::stream::repeat(serde_json::Value::Null)));

        assert!((0..=MAX_PENDING_MESSAGES).any(|_| poll_once(&mut connection).is_ready()));
        assert_eq!(connection.pending.messages.len(), MAX_PENDING_MESSAGES);
        assert!(connection.connection.sent.is_empty());
    }

    #[test]
    fn queued_responses_bypass_notification_limits() {
        // Two responses that together exceed the notification byte budget.
        let response = serde_json::Value::String("x".repeat(MAX_PENDING_BYTES / 2));
        let handler = TestHandler { response, ..Default::default() };
        let mut connection = PubSubConnection::new(TestConnection::default(), handler);
        let (tx, rx) = mpsc::unbounded();
        connection.context.add_subscription(1, Box::pin(rx));
        connection.connection.requests.extend([method_call(1), method_call(2)]);
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(connection.pending.responses, 2);

        // Queued responses leave both notification budgets available: fill them exactly.
        let notification =
            serde_json::Value::String("x".repeat(MAX_PENDING_BYTES / MAX_PENDING_MESSAGES - 2));
        for _ in 0..MAX_PENDING_MESSAGES {
            tx.unbounded_send(notification.clone()).unwrap();
            assert!(poll_once(&mut connection).is_pending());
        }
        assert_eq!(connection.pending.messages.len(), MAX_PENDING_MESSAGES + 2);
        assert_eq!(connection.pending.bytes, MAX_PENDING_BYTES);

        connection.connection.ready = true;
        assert!(poll_once(&mut connection).is_pending());
        let sent = &connection.connection.sent;
        assert_eq!(sent.len(), MAX_PENDING_MESSAGES + 2);
        let mut ids = sent[..2]
            .iter()
            .map(|response| {
                serde_json::from_str::<serde_json::Value>(response).unwrap()["id"].clone()
            })
            .collect::<Vec<_>>();
        ids.sort_unstable_by_key(|id| id.as_i64());
        assert_eq!(ids, [1, 2]);
        let notification = notification.to_string();
        assert!(sent[2..].iter().all(|sent| *sent == notification));
        assert!(connection.pending.messages.is_empty());
        assert_eq!((connection.pending.responses, connection.pending.bytes), (0, 0));
    }

    #[test]
    fn queued_responses_pause_request_reads() {
        let handler = TestHandler::default();
        let mut connection = PubSubConnection::new(TestConnection::default(), handler.clone());
        let requests = (0..MAX_PENDING_RESPONSES as i64).map(method_call);
        connection.connection.requests.extend(requests);
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(connection.pending.responses, MAX_PENDING_RESPONSES);

        connection.connection.requests.push_back(method_call(MAX_PENDING_RESPONSES as i64));
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), MAX_PENDING_RESPONSES);
        assert_eq!(connection.connection.requests.len(), 1);

        connection.connection.ready = true;
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), MAX_PENDING_RESPONSES + 1);
        assert_eq!(connection.connection.sent.len(), MAX_PENDING_RESPONSES + 1);
    }

    #[test]
    fn requests_in_flight_have_their_own_read_limit() {
        // In-flight requests may wait on a later request, so their limit is far above the
        // response limit.
        let handler = TestHandler { block: true, ..Default::default() };
        let mut connection = PubSubConnection::new(TestConnection::default(), handler.clone());
        let requests = (0..=MAX_PROCESSING_REQUESTS as i64).map(method_call);
        connection.connection.requests.extend(requests);

        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), MAX_PROCESSING_REQUESTS);
        assert_eq!(connection.connection.requests.len(), 1);
    }

    #[test]
    fn completed_requests_without_responses_resume_reads() {
        let handler = TestHandler::default();
        let mut connection = PubSubConnection::new(TestConnection::default(), handler.clone());
        let notifications = (0..=MAX_PROCESSING_REQUESTS).map(|_| Request::Single(notification()));
        connection.connection.requests.extend(notifications);

        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(handler.requests.load(Ordering::Relaxed), MAX_PROCESSING_REQUESTS + 1);
    }

    #[test]
    fn backlog_byte_limit_reclaims_drained_messages() {
        let (tx, rx) = mpsc::unbounded();
        let mut connection = connection();
        connection.context.add_subscription(1, Box::pin(rx));
        // Include the JSON string's two quotes in the byte budget.
        let message = serde_json::Value::String("x".repeat(MAX_PENDING_BYTES / 2 - 2));
        for _ in 0..2 {
            tx.unbounded_send(message.clone()).unwrap();
            assert!(poll_once(&mut connection).is_pending());
        }
        assert_eq!(connection.pending.bytes, MAX_PENDING_BYTES);

        connection.connection.ready = true;
        connection.connection.send_budget = Some(1);
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(connection.connection.sent.len(), 1);
        assert_eq!(connection.pending.bytes, MAX_PENDING_BYTES / 2);

        tx.unbounded_send(message).unwrap();
        assert!(poll_once(&mut connection).is_pending());
        assert_eq!(connection.pending.bytes, MAX_PENDING_BYTES);

        tx.unbounded_send(serde_json::Value::Null).unwrap();
        assert!(poll_once(&mut connection).is_ready());
        assert_eq!(connection.pending.messages.len(), 2);
        assert_eq!(connection.pending.bytes, MAX_PENDING_BYTES);
    }

    #[test]
    fn oversized_message_requires_writable_connection() {
        for ready in [true, false] {
            let transport = TestConnection { ready, ..Default::default() };
            let mut connection = PubSubConnection::new(transport, TestHandler::default());
            connection.context.add_subscription(
                1,
                Box::pin(futures::stream::iter([serde_json::Value::String(
                    "x".repeat(MAX_PENDING_BYTES),
                )])),
            );

            assert_eq!(poll_once(&mut connection).is_pending(), ready);
            if ready {
                assert_eq!(connection.connection.sent[0].len(), MAX_PENDING_BYTES + 2);
            } else {
                assert!(connection.connection.sent.is_empty());
            }
            assert_eq!(connection.pending.bytes, 0);
            assert!(connection.pending.messages.is_empty());
        }
    }

    #[test]
    fn writable_subscription_burst_yields_and_rotates_without_buffering() {
        let transport = TestConnection { ready: true, ..Default::default() };
        let mut connection = PubSubConnection::new(transport, TestHandler::default());
        connection
            .context
            .add_subscription(1, Box::pin(futures::stream::iter([serde_json::json!(2)])));
        connection.context.add_subscription(
            2,
            Box::pin(futures::stream::iter(vec![serde_json::json!(1); MAX_PENDING_MESSAGES + 1])),
        );
        let wake = Arc::new(TestWake::default());
        let waker = Waker::from(wake.clone());
        let mut cx = Context::from_waker(&waker);

        assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
        assert_eq!(wake.0.swap(0, Ordering::Relaxed), 1);
        let mut first = connection.connection.sent[..2].to_vec();
        first.sort_unstable();
        assert_eq!(first, ["1", "2"]);
        assert!(connection.connection.sent.len() < MAX_PENDING_MESSAGES);

        for _ in 0..MAX_PENDING_MESSAGES {
            assert!(Pin::new(&mut connection).poll(&mut cx).is_pending());
            if connection.context.subscriptions.lock().is_empty() {
                break;
            }
        }
        assert!(connection.context.subscriptions.lock().is_empty());
        assert_eq!(connection.connection.sent.len(), MAX_PENDING_MESSAGES + 2);
        assert_eq!(connection.connection.sent[2..], vec!["1"; MAX_PENDING_MESSAGES]);
        assert!(connection.pending.messages.is_empty());
        assert_eq!(connection.pending.bytes, 0);
    }

    fn connection() -> PubSubConnection<TestHandler, TestConnection> {
        PubSubConnection::new(TestConnection::default(), TestHandler::default())
    }

    fn poll_once(connection: &mut PubSubConnection<TestHandler, TestConnection>) -> Poll<()> {
        Pin::new(connection).poll(&mut Context::from_waker(Waker::noop()))
    }

    fn sent_events(connection: &TestConnection) -> Vec<[u64; 2]> {
        connection.sent.iter().map(|event| serde_json::from_str(event).unwrap()).collect()
    }
}
