mod test_utils;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::task::Poll;
use std::thread;
use std::time::Duration;

use futures_channel::{mpsc, oneshot};
use futures_util::future::{self, FutureExt, TryFutureExt};
use futures_util::stream::StreamExt;
use futures_util::{self, Stream};
use http_body_util::BodyExt;
use http_body_util::{Empty, Full, StreamBody};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use hyper::body::Bytes;
use hyper::body::Frame;
use hyper::Request;
use hyper_util::client::legacy::connect::{capture_connection, HttpConnector};
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};

use test_utils::{DebugConnector, DebugStream};

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("new rt")
}

fn s(buf: &[u8]) -> &str {
    std::str::from_utf8(buf).expect("from_utf8")
}

#[cfg(not(miri))]
#[test]
fn drop_body_before_eof_closes_connection() {
    // https://github.com/hyperium/hyper/issues/1353
    let _ = pretty_env_logger::try_init();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let rt = runtime();
    let (closes_tx, closes) = mpsc::channel::<()>(10);
    let client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(
        DebugConnector::with_http_and_closes(HttpConnector::new(), closes_tx),
    );
    let (tx1, rx1) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        let body = vec![b'x'; 1024 * 128];
        write!(
            sock,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .expect("write head");
        let _ = sock.write_all(&body);
        let _ = tx1.send(());
    });

    let req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req).map_ok(move |res| {
        assert_eq!(res.status(), hyper::StatusCode::OK);
    });
    let rx = rx1;
    rt.block_on(async move {
        let (res, _) = future::join(res, rx).await;
        res.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    rt.block_on(closes.into_future()).0.expect("closes");
}

#[cfg(not(miri))]
#[tokio::test]
async fn drop_client_closes_idle_connections() {
    let _ = pretty_env_logger::try_init();

    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let (closes_tx, mut closes) = mpsc::channel(10);

    let (tx1, rx1) = oneshot::channel();

    let t1 = tokio::spawn(async move {
        let mut sock = server.accept().await.unwrap().0;
        let mut buf = [0; 4096];
        sock.read(&mut buf).await.expect("read 1");
        let body = [b'x'; 64];
        let headers = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        sock.write_all(headers.as_bytes())
            .await
            .expect("write head");
        sock.write_all(&body).await.expect("write body");
        let _ = tx1.send(());

        // prevent this thread from closing until end of test, so the connection
        // stays open and idle until Client is dropped
        match sock.read(&mut buf).await {
            Ok(n) => assert_eq!(n, 0),
            Err(_) => (),
        }
    });

    let client = Client::builder(TokioExecutor::new()).build(DebugConnector::with_http_and_closes(
        HttpConnector::new(),
        closes_tx,
    ));

    let req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req).map_ok(move |res| {
        assert_eq!(res.status(), hyper::StatusCode::OK);
    });
    let rx = rx1;
    let (res, _) = future::join(res, rx).await;
    res.unwrap();

    // not closed yet, just idle
    future::poll_fn(|ctx| {
        assert!(Pin::new(&mut closes).poll_next(ctx).is_pending());
        Poll::Ready(())
    })
    .await;

    // drop to start the connections closing
    drop(client);

    // and wait a few ticks for the connections to close
    let t = tokio::time::sleep(Duration::from_millis(100)).map(|_| panic!("time out"));
    futures_util::pin_mut!(t);
    let close = closes.into_future().map(|(opt, _)| opt.expect("closes"));
    future::select(t, close).await;
    t1.await.unwrap();
}

#[cfg(not(miri))]
#[tokio::test]
async fn drop_response_future_closes_in_progress_connection() {
    let _ = pretty_env_logger::try_init();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let (closes_tx, closes) = mpsc::channel(10);

    let (tx1, rx1) = oneshot::channel();
    let (_client_drop_tx, client_drop_rx) = std::sync::mpsc::channel::<()>();

    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        // we never write a response head
        // simulates a slow server operation
        let _ = tx1.send(());

        // prevent this thread from closing until end of test, so the connection
        // stays open and idle until Client is dropped
        let _ = client_drop_rx.recv();
    });

    let res = {
        let client = Client::builder(TokioExecutor::new()).build(
            DebugConnector::with_http_and_closes(HttpConnector::new(), closes_tx),
        );

        let req = Request::builder()
            .uri(&*format!("http://{}/a", addr))
            .body(Empty::<Bytes>::new())
            .unwrap();
        client.request(req).map(|_| unreachable!())
    };

    future::select(res, rx1).await;

    // res now dropped
    let t = tokio::time::sleep(Duration::from_millis(100)).map(|_| panic!("time out"));
    futures_util::pin_mut!(t);
    let close = closes.into_future().map(|(opt, _)| opt.expect("closes"));
    future::select(t, close).await;
}

#[cfg(not(miri))]
#[tokio::test]
async fn drop_response_body_closes_in_progress_connection() {
    let _ = pretty_env_logger::try_init();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let (closes_tx, closes) = mpsc::channel(10);

    let (tx1, rx1) = oneshot::channel();
    let (_client_drop_tx, client_drop_rx) = std::sync::mpsc::channel::<()>();

    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        write!(
            sock,
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
        )
        .expect("write head");
        let _ = tx1.send(());

        // prevent this thread from closing until end of test, so the connection
        // stays open and idle until Client is dropped
        let _ = client_drop_rx.recv();
    });

    let rx = rx1;
    let res = {
        let client = Client::builder(TokioExecutor::new()).build(
            DebugConnector::with_http_and_closes(HttpConnector::new(), closes_tx),
        );

        let req = Request::builder()
            .uri(&*format!("http://{}/a", addr))
            .body(Empty::<Bytes>::new())
            .unwrap();
        // notably, haven't read body yet
        client.request(req)
    };

    let (res, _) = future::join(res, rx).await;
    // drop the body
    res.unwrap();

    // and wait a few ticks to see the connection drop
    let t = tokio::time::sleep(Duration::from_millis(100)).map(|_| panic!("time out"));
    futures_util::pin_mut!(t);
    let close = closes.into_future().map(|(opt, _)| opt.expect("closes"));
    future::select(t, close).await;
}

#[cfg(not(miri))]
#[tokio::test]
async fn no_keep_alive_closes_connection() {
    // https://github.com/hyperium/hyper/issues/1383
    let _ = pretty_env_logger::try_init();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let (closes_tx, closes) = mpsc::channel(10);

    let (tx1, rx1) = oneshot::channel();
    let (_tx2, rx2) = std::sync::mpsc::channel::<()>();

    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let _ = tx1.send(());

        // prevent this thread from closing until end of test, so the connection
        // stays open and idle until Client is dropped
        let _ = rx2.recv();
    });

    let client = Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(0)
        .build(DebugConnector::with_http_and_closes(
            HttpConnector::new(),
            closes_tx,
        ));

    let req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req).map_ok(move |res| {
        assert_eq!(res.status(), hyper::StatusCode::OK);
    });
    let rx = rx1;
    let (res, _) = future::join(res, rx).await;
    res.unwrap();

    let t = tokio::time::sleep(Duration::from_millis(100)).map(|_| panic!("time out"));
    futures_util::pin_mut!(t);
    let close = closes.into_future().map(|(opt, _)| opt.expect("closes"));
    future::select(close, t).await;
}

#[cfg(not(miri))]
#[tokio::test]
async fn socket_disconnect_closes_idle_conn() {
    // notably when keep-alive is enabled
    let _ = pretty_env_logger::try_init();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let (closes_tx, closes) = mpsc::channel(10);

    let (tx1, rx1) = oneshot::channel();

    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let _ = tx1.send(());
    });

    let client = Client::builder(TokioExecutor::new()).build(DebugConnector::with_http_and_closes(
        HttpConnector::new(),
        closes_tx,
    ));

    let req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req).map_ok(move |res| {
        assert_eq!(res.status(), hyper::StatusCode::OK);
    });
    let rx = rx1;

    let (res, _) = future::join(res, rx).await;
    res.unwrap();

    let t = tokio::time::sleep(Duration::from_millis(100)).map(|_| panic!("time out"));
    futures_util::pin_mut!(t);
    let close = closes.into_future().map(|(opt, _)| opt.expect("closes"));
    future::select(t, close).await;
}

#[cfg(not(miri))]
#[test]
fn connect_call_is_lazy() {
    // We especially don't want connects() triggered if there's
    // idle connections that the Checkout would have found
    let _ = pretty_env_logger::try_init();

    let _rt = runtime();
    let connector = DebugConnector::new();
    let connects = connector.connects.clone();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    assert_eq!(connects.load(Ordering::Relaxed), 0);
    let req = Request::builder()
        .uri("http://hyper.local/a")
        .body(Empty::<Bytes>::new())
        .unwrap();
    let _fut = client.request(req);
    // internal Connect::connect should have been lazy, and not
    // triggered an actual connect yet.
    assert_eq!(connects.load(Ordering::Relaxed), 0);
}

#[cfg(not(miri))]
#[test]
fn client_keep_alive_0() {
    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let rt = runtime();
    let connector = DebugConnector::new();
    let connects = connector.connects.clone();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        //drop(server);
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 1");
        let _ = tx1.send(());

        let n2 = sock.read(&mut buf).expect("read 2");
        assert_ne!(n2, 0);
        let second_get = "GET /b HTTP/1.1\r\n";
        assert_eq!(s(&buf[..second_get.len()]), second_get);
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 2");
        let _ = tx2.send(());
    });

    assert_eq!(connects.load(Ordering::SeqCst), 0);

    let rx = rx1;
    let req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req);
    rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();

    assert_eq!(connects.load(Ordering::SeqCst), 1);

    // sleep real quick to let the threadpool put connection in ready
    // state and back into client pool
    thread::sleep(Duration::from_millis(50));

    let rx = rx2;
    let req = Request::builder()
        .uri(&*format!("http://{}/b", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req);
    rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();

    assert_eq!(
        connects.load(Ordering::SeqCst),
        1,
        "second request should still only have 1 connect"
    );
    drop(client);
}

#[cfg(not(miri))]
#[test]
fn client_keep_alive_extra_body() {
    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let rt = runtime();

    let connector = DebugConnector::new();
    let connects = connector.connects.clone();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
            .expect("write 1");
        // the body "hello", while ignored because its a HEAD request, should mean the connection
        // cannot be put back in the pool
        let _ = tx1.send(());

        let mut sock2 = server.accept().unwrap().0;
        let n2 = sock2.read(&mut buf).expect("read 2");
        assert_ne!(n2, 0);
        let second_get = "GET /b HTTP/1.1\r\n";
        assert_eq!(s(&buf[..second_get.len()]), second_get);
        sock2
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 2");
        let _ = tx2.send(());
    });

    assert_eq!(connects.load(Ordering::Relaxed), 0);

    let rx = rx1;
    let req = Request::builder()
        .method("HEAD")
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req);
    rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();

    assert_eq!(connects.load(Ordering::Relaxed), 1);

    let rx = rx2;
    let req = Request::builder()
        .uri(&*format!("http://{}/b", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req);
    rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();

    assert_eq!(connects.load(Ordering::Relaxed), 2);
}

#[cfg(not(miri))]
#[tokio::test]
async fn client_keep_alive_when_response_before_request_body_ends() {
    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();

    let (closes_tx, mut closes) = mpsc::channel::<()>(10);
    let connector = DebugConnector::with_http_and_closes(HttpConnector::new(), closes_tx);
    let connects = connector.connects.clone();
    let client = Client::builder(TokioExecutor::new()).build(connector.clone());

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    let (_tx3, rx3) = std::sync::mpsc::channel::<()>();

    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 1");
        // after writing the response, THEN stream the body
        let _ = tx1.send(());

        sock.read(&mut buf).expect("read 2");
        let _ = tx2.send(());

        // prevent this thread from closing until end of test, so the connection
        // stays open and idle until Client is dropped
        let _ = rx3.recv();
    });

    assert_eq!(connects.load(Ordering::Relaxed), 0);

    let delayed_body = rx1
        .then(|_| Box::pin(tokio::time::sleep(Duration::from_millis(200))))
        .map(|_| Ok::<_, ()>(Frame::data(&b"hello a"[..])))
        .map_err(|_| -> hyper::Error { panic!("rx1") })
        .into_stream();

    let req = Request::builder()
        .method("POST")
        .uri(&*format!("http://{}/a", addr))
        .body(StreamBody::new(delayed_body))
        .unwrap();
    let res = client.request(req).map_ok(move |res| {
        assert_eq!(res.status(), hyper::StatusCode::OK);
    });

    future::join(res, rx2).await.0.unwrap();
    future::poll_fn(|ctx| {
        assert!(Pin::new(&mut closes).poll_next(ctx).is_pending());
        Poll::Ready(())
    })
    .await;

    assert_eq!(connects.load(Ordering::Relaxed), 1);

    drop(client);
    let t = tokio::time::sleep(Duration::from_millis(100)).map(|_| panic!("time out"));
    futures_util::pin_mut!(t);
    let close = closes.into_future().map(|(opt, _)| opt.expect("closes"));
    future::select(t, close).await;
}

#[cfg(not(miri))]
#[tokio::test]
async fn client_keep_alive_eager_when_chunked() {
    // If a response body has been read to completion, with completion
    // determined by some other factor, like decompression, and thus
    // it is in't polled a final time to clear the final 0-len chunk,
    // try to eagerly clear it so the connection can still be used.

    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let connector = DebugConnector::new();
    let connects = connector.connects.clone();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let (tx1, rx1) = oneshot::channel();
    let (tx2, rx2) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        //drop(server);
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(
            b"\
                HTTP/1.1 200 OK\r\n\
                transfer-encoding: chunked\r\n\
                \r\n\
                5\r\n\
                hello\r\n\
                0\r\n\r\n\
            ",
        )
        .expect("write 1");
        let _ = tx1.send(());

        let n2 = sock.read(&mut buf).expect("read 2");
        assert_ne!(n2, 0, "bytes of second request");
        let second_get = "GET /b HTTP/1.1\r\n";
        assert_eq!(s(&buf[..second_get.len()]), second_get);
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 2");
        let _ = tx2.send(());
    });

    assert_eq!(connects.load(Ordering::SeqCst), 0);

    let rx = rx1;
    let req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let fut = client.request(req);

    let resp = future::join(fut, rx).map(|r| r.0).await.unwrap();
    assert_eq!(connects.load(Ordering::SeqCst), 1);
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["transfer-encoding"], "chunked");

    // Read the "hello" chunk...
    let chunk = resp.collect().await.unwrap().to_bytes();
    assert_eq!(chunk, "hello");

    // sleep real quick to let the threadpool put connection in ready
    // state and back into client pool
    tokio::time::sleep(Duration::from_millis(50)).await;

    let rx = rx2;
    let req = Request::builder()
        .uri(&*format!("http://{}/b", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let fut = client.request(req);
    future::join(fut, rx).map(|r| r.0).await.unwrap();

    assert_eq!(
        connects.load(Ordering::SeqCst),
        1,
        "second request should still only have 1 connect"
    );
    drop(client);
}

#[cfg(not(miri))]
#[test]
fn connect_proxy_sends_absolute_uri() {
    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let rt = runtime();
    let connector = DebugConnector::new().proxy();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let (tx1, rx1) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        //drop(server);
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        let n = sock.read(&mut buf).expect("read 1");
        let expected = format!(
            "GET http://{addr}/foo/bar HTTP/1.1\r\nhost: {addr}\r\n\r\n",
            addr = addr
        );
        assert_eq!(s(&buf[..n]), expected);

        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 1");
        let _ = tx1.send(());
    });

    let rx = rx1;
    let req = Request::builder()
        .uri(&*format!("http://{}/foo/bar", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req);
    rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();
}

#[cfg(not(miri))]
#[test]
fn connect_proxy_http_connect_sends_authority_form() {
    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let rt = runtime();
    let connector = DebugConnector::new().proxy();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let (tx1, rx1) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        //drop(server);
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        let n = sock.read(&mut buf).expect("read 1");
        let expected = format!(
            "CONNECT {addr} HTTP/1.1\r\nhost: {addr}\r\n\r\n",
            addr = addr
        );
        assert_eq!(s(&buf[..n]), expected);

        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 1");
        let _ = tx1.send(());
    });

    let rx = rx1;
    let req = Request::builder()
        .method("CONNECT")
        .uri(&*format!("http://{}/useless/path", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let res = client.request(req);
    rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();
}

#[cfg(not(miri))]
#[test]
fn client_upgrade() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let _ = pretty_env_logger::try_init();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let rt = runtime();

    let connector = DebugConnector::new();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let (tx1, rx1) = oneshot::channel();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(
            b"\
                HTTP/1.1 101 Switching Protocols\r\n\
                Upgrade: foobar\r\n\
                \r\n\
                foobar=ready\
            ",
        )
        .unwrap();
        let _ = tx1.send(());

        let n = sock.read(&mut buf).expect("read 2");
        assert_eq!(&buf[..n], b"foo=bar");
        sock.write_all(b"bar=foo").expect("write 2");
    });

    let rx = rx1;

    let req = Request::builder()
        .method("GET")
        .uri(&*format!("http://{}/up", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();

    let res = client.request(req);
    let res = rt.block_on(future::join(res, rx).map(|r| r.0)).unwrap();

    assert_eq!(res.status(), 101);
    let upgraded = rt.block_on(hyper::upgrade::on(res)).expect("on_upgrade");

    let parts = upgraded.downcast::<DebugStream>().unwrap();
    assert_eq!(s(&parts.read_buf), "foobar=ready");

    let mut io = parts.io;
    rt.block_on(io.write_all(b"foo=bar")).unwrap();
    let mut vec = vec![];
    rt.block_on(io.read_to_end(&mut vec)).unwrap();
    assert_eq!(vec, b"bar=foo");
}

#[cfg(not(miri))]
#[test]
fn alpn_h2() {
    use http::Response;
    use hyper::service::service_fn;
    use tokio::net::TcpListener;

    let _ = pretty_env_logger::try_init();
    let rt = runtime();
    let listener = rt
        .block_on(TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let mut connector = DebugConnector::new();
    connector.alpn_h2 = true;
    let connects = connector.connects.clone();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    rt.spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let stream = TokioIo::new(stream);
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(
                stream,
                service_fn(|req| async move {
                    assert_eq!(req.headers().get("host"), None);
                    Ok::<_, hyper::Error>(Response::new(Full::<Bytes>::from("Hello, world")))
                }),
            )
            .await
            .expect("server");
    });

    assert_eq!(connects.load(Ordering::SeqCst), 0);

    let url = format!("http://{}/a", addr)
        .parse::<::hyper::Uri>()
        .unwrap();
    let res1 = client.get(url.clone());
    let res2 = client.get(url.clone());
    let res3 = client.get(url.clone());
    rt.block_on(future::try_join3(res1, res2, res3)).unwrap();

    // Since the client doesn't know it can ALPN at first, it will have
    // started 3 connections. But, the server above will only handle 1,
    // so the unwrapped responses futures show it still worked.
    assert_eq!(connects.load(Ordering::SeqCst), 3);

    let res4 = client.get(url.clone());
    rt.block_on(res4).unwrap();

    // HTTP/2 request allowed
    let res5 = client.request(
        Request::builder()
            .uri(url)
            .version(hyper::Version::HTTP_2)
            .body(Empty::<Bytes>::new())
            .unwrap(),
    );
    rt.block_on(res5).unwrap();

    assert_eq!(
        connects.load(Ordering::SeqCst),
        3,
        "after ALPN, no more connects"
    );
    drop(client);
}

#[cfg(not(miri))]
#[test]
fn capture_connection_on_client() {
    let _ = pretty_env_logger::try_init();

    let rt = runtime();
    let connector = DebugConnector::new();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    thread::spawn(move || {
        let mut sock = server.accept().unwrap().0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0; 4096];
        sock.read(&mut buf).expect("read 1");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("write 1");
    });
    let mut req = Request::builder()
        .uri(&*format!("http://{}/a", addr))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let captured_conn = capture_connection(&mut req);
    rt.block_on(client.request(req)).expect("200 OK");
    assert!(captured_conn.connection_metadata().is_some());
}

#[cfg(not(miri))]
#[test]
fn connection_poisoning() {
    use std::sync::atomic::AtomicUsize;

    let _ = pretty_env_logger::try_init();

    let rt = runtime();
    let connector = DebugConnector::new();

    let client = Client::builder(TokioExecutor::new()).build(connector);

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let num_conns: Arc<AtomicUsize> = Default::default();
    let num_requests: Arc<AtomicUsize> = Default::default();
    let num_requests_tracker = num_requests.clone();
    let num_conns_tracker = num_conns.clone();
    thread::spawn(move || loop {
        let mut sock = server.accept().unwrap().0;
        num_conns_tracker.fetch_add(1, Ordering::Relaxed);
        let num_requests_tracker = num_requests_tracker.clone();
        thread::spawn(move || {
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            sock.set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buf = [0; 4096];
            loop {
                if sock.read(&mut buf).expect("read 1") > 0 {
                    num_requests_tracker.fetch_add(1, Ordering::Relaxed);
                    sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                        .expect("write 1");
                }
            }
        });
    });
    let make_request = || {
        Request::builder()
            .uri(&*format!("http://{}/a", addr))
            .body(Empty::<Bytes>::new())
            .unwrap()
    };
    let mut req = make_request();
    let captured_conn = capture_connection(&mut req);
    rt.block_on(client.request(req)).expect("200 OK");
    assert_eq!(num_conns.load(Ordering::SeqCst), 1);
    assert_eq!(num_requests.load(Ordering::SeqCst), 1);

    rt.block_on(client.request(make_request())).expect("200 OK");
    rt.block_on(client.request(make_request())).expect("200 OK");
    // Before poisoning the connection is reused
    assert_eq!(num_conns.load(Ordering::SeqCst), 1);
    assert_eq!(num_requests.load(Ordering::SeqCst), 3);
    captured_conn
        .connection_metadata()
        .as_ref()
        .unwrap()
        .poison();

    rt.block_on(client.request(make_request())).expect("200 OK");

    // After poisoning, a new connection is established
    assert_eq!(num_conns.load(Ordering::SeqCst), 2);
    assert_eq!(num_requests.load(Ordering::SeqCst), 4);

    rt.block_on(client.request(make_request())).expect("200 OK");
    // another request can still reuse:
    assert_eq!(num_conns.load(Ordering::SeqCst), 2);
    assert_eq!(num_requests.load(Ordering::SeqCst), 5);
}
