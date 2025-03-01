use async_priority_channel::bounded;

#[tokio::test]
async fn capacity() {
    for i in 1..10 {
        let (s, r) = bounded::<(), i32>(i);
        assert_eq!(s.capacity(), Some(i));
        assert_eq!(r.capacity(), Some(i));
    }
}

#[tokio::test]
async fn len_empty_full() {
    let (s, r) = bounded(2);

    assert_eq!(s.len(), 0);
    assert_eq!(s.is_empty(), true);
    assert_eq!(s.is_full(), false);
    assert_eq!(r.len(), 0);
    assert_eq!(r.is_empty(), true);
    assert_eq!(r.is_full(), false);

    s.send((), 0).await.unwrap();

    assert_eq!(s.len(), 1);
    assert_eq!(s.is_empty(), false);
    assert_eq!(s.is_full(), false);
    assert_eq!(r.len(), 1);
    assert_eq!(r.is_empty(), false);
    assert_eq!(r.is_full(), false);

    s.send((), 0).await.unwrap();

    assert_eq!(s.len(), 2);
    assert_eq!(s.is_empty(), false);
    assert_eq!(s.is_full(), true);
    assert_eq!(r.len(), 2);
    assert_eq!(r.is_empty(), false);
    assert_eq!(r.is_full(), true);

    r.recv().await.unwrap();

    assert_eq!(s.len(), 1);
    assert_eq!(s.is_empty(), false);
    assert_eq!(s.is_full(), false);
    assert_eq!(r.len(), 1);
    assert_eq!(r.is_empty(), false);
    assert_eq!(r.is_full(), false);
}

#[test]
fn receiver_count() {
    let (s, r) = bounded::<(), i32>(5);
    let receiver_clones: Vec<_> = (0..20).map(|_| r.clone()).collect();

    assert_eq!(s.receiver_count(), 21);
    assert_eq!(r.receiver_count(), 21);

    drop(receiver_clones);

    assert_eq!(s.receiver_count(), 1);
    assert_eq!(r.receiver_count(), 1);
}

#[test]
fn sender_count() {
    let (s, r) = bounded::<(), i32>(5);
    let sender_clones: Vec<_> = (0..20).map(|_| s.clone()).collect();

    assert_eq!(s.sender_count(), 21);
    assert_eq!(r.sender_count(), 21);

    drop(sender_clones);

    assert_eq!(s.receiver_count(), 1);
    assert_eq!(r.receiver_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_recv_1() {
    let (tx, rx) = bounded(1);
    tx.send(1, 1).await.unwrap();
    assert_eq!(rx.recv().await.unwrap(), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_recv_2() {
    let (tx, rx) = bounded(3);
    tx.send(1, 1).await.unwrap();
    tx.send(3, 3).await.unwrap();
    tx.send(2, 2).await.unwrap();
    assert_eq!(rx.recv().await.unwrap(), (3, 3));
    assert_eq!(rx.recv().await.unwrap(), (2, 2));
    assert_eq!(rx.recv().await.unwrap(), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_recv_close_1() {
    let (tx, rx) = bounded(3);
    tx.send(1, 1).await.unwrap();
    tx.send(3, 3).await.unwrap();
    tx.send(2, 2).await.unwrap();
    tx.close();
    tx.send(4, 4).await.unwrap_err();
    assert_eq!(rx.recv().await.unwrap(), (3, 3));
    assert_eq!(rx.recv().await.unwrap(), (2, 2));
    assert_eq!(rx.recv().await.unwrap(), (1, 1));
    rx.recv().await.unwrap_err();
}
#[tokio::test(flavor = "multi_thread")]
async fn test_send_recv_close_2() {
    let (tx, rx) = bounded(2);
    rx.close();
    tx.send(4, 4).await.unwrap_err();
    rx.recv().await.unwrap_err();
}

#[tokio::test(flavor = "multi_thread")]
async fn bconcurrent_1() {
    let n: i32 = 1000;
    let (tx, rx) = bounded(10);
    tokio::spawn(async move {
        for i in 0..n {
            tx.send(i, i).await.unwrap();
        }
    });
    let mut v = Vec::new();
    for _ in 0..n {
        let r = rx.recv().await.unwrap();
        v.push(r.0);
    }
    v.sort();

    let expected: Vec<i32> = (0..n).collect();
    assert_eq!(v, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn bconcurrent_2() {
    let n: i32 = 500;
    let m: i32 = 10;
    let (tx, rx) = bounded(10);

    for j in 0..m {
        let tx = tx.clone();
        tokio::spawn(async move {
            for i in 0..n {
                let priority = j * n + i;
                tx.send((), priority).await.unwrap();
            }
        });
    }

    let mut v = Vec::new();
    for _ in 0..n * m {
        let r = rx.recv().await.unwrap();
        v.push(r.1);
    }
    v.sort();

    let expected: Vec<i32> = (0..n * m).collect();
    assert_eq!(v, expected);
}
#[tokio::test(flavor = "multi_thread")]
async fn bconcurrent_3() {
    let n: i32 = 500;
    let m: i32 = 10;
    let (tx, rx) = bounded(10);

    for j in 0..m {
        let tx = tx.clone();
        tokio::spawn(async move {
            for i in 0..n {
                tx.send((), j * n + i).await.unwrap();
            }
        });
    }

    let mut collected = Vec::new();

    for _ in 0..m {
        let tx = rx.clone();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let mut v = Vec::new();
        tokio::spawn(async move {
            for _ in 0..n {
                v.push(tx.recv().await.unwrap().1);
            }
            result_tx.send(v).unwrap();
        });
        collected.push(result_rx);
    }

    let mut v = Vec::new();
    for item in collected {
        v.extend(item.await.unwrap());
    }
    v.sort();

    let expected: Vec<i32> = (0..n * m).collect();
    assert_eq!(v, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn bclose_1() {
    let (tx, rx) = bounded::<(), i32>(10);
    let mut jh = Vec::new();
    for _ in 0..10 {
        let rx = rx.clone();
        let thread = tokio::spawn(async move {
            rx.recv().await.unwrap_err();
        });
        jh.push(thread);
    }
    jh.push(tokio::spawn(async move {
        tx.close();
    }));
    for thread in jh {
        thread.await.unwrap();
    }
}

#[test]
fn sendv_1() {
    let cap = 10;
    let (tx, rx) = bounded::<&str, i32>(cap);
    let v = vec![("a", 0), ("b", 1)];
    tx.try_sendv(v.into_iter().peekable()).unwrap();
    assert_eq!(rx.try_recv().unwrap(), ("b", 1));
    assert_eq!(rx.try_recv().unwrap(), ("a", 0));

    let n = 100;
    let v = vec![("a", 0); n as usize];
    let err = tx.try_sendv(v.into_iter().peekable()).unwrap_err();
    assert_eq!(err.into_inner().count() as u64, n - cap);
}

#[test]
fn sendv_2() {
    let cap = 10;
    let (tx, _rx) = bounded::<&str, i32>(cap);
    tx.close();
    assert!(tx.is_closed());
    let err = tx
        .try_sendv(vec![("a", 0), ("b", 1)].into_iter().peekable())
        .unwrap_err();
    assert_eq!(err.into_inner().count(), 2);
}

#[test]
fn sendv_3() {
    let (tx, _rx) = bounded::<(), i32>(2);
    tx.try_send((), 0).unwrap();
    tx.try_send((), 0).unwrap();
}
