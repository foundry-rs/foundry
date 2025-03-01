use async_priority_channel::unbounded;

#[tokio::test(flavor = "multi_thread")]
async fn utest_send_recv_1() {
    let (tx, rx) = unbounded();
    tx.send(1, 1).await.unwrap();
    assert_eq!(rx.recv().await.unwrap(), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn utest_send_recv_2() {
    let (tx, rx) = unbounded();
    tx.send(1, 1).await.unwrap();
    tx.send(3, 3).await.unwrap();
    tx.send(2, 2).await.unwrap();
    assert_eq!(rx.recv().await.unwrap(), (3, 3));
    assert_eq!(rx.recv().await.unwrap(), (2, 2));
    assert_eq!(rx.recv().await.unwrap(), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn utest_send_recv_close_1() {
    let (tx, rx) = unbounded();
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
    let (tx, rx) = unbounded();
    rx.close();
    tx.send(4, 4).await.unwrap_err();
    rx.recv().await.unwrap_err();
}

#[tokio::test(flavor = "multi_thread")]
async fn uconcurrent_1() {
    let n: i32 = 1000;
    let (tx, rx) = unbounded();
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
async fn concurrent_2() {
    let n: i32 = 500;
    let m: i32 = 10;
    let (tx, rx) = unbounded();

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
async fn concurrent_3() {
    let n: i32 = 500;
    let m: i32 = 10;
    let (tx, rx) = unbounded();

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
async fn uclose_1() {
    let n = 1000;
    let (tx, rx) = unbounded::<(), i32>();
    let mut jh = Vec::new();

    for _ in 0..n {
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

#[tokio::test]
async fn uclose_2() {
    let (tx, rx) = unbounded::<(), i32>();
    let a = tokio::spawn(async move {
        rx.recv().await.unwrap_err();
    });
    let b = tokio::spawn(async move {
        tx.close();
    });

    a.await.unwrap();
    b.await.unwrap();
}
