use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Context;
use std::usize;

use event_listener::{Event, EventListener};
use waker_fn::waker_fn;

#[cfg(target_family = "wasm")]
use wasm_bindgen_test::wasm_bindgen_test as test;

fn is_notified(listener: Pin<&mut EventListener>) -> bool {
    let waker = waker_fn(|| ());
    listener.poll(&mut Context::from_waker(&waker)).is_ready()
}

#[test]
fn notify() {
    let event = Event::new();

    let mut l1 = event.listen();
    let mut l2 = event.listen();
    let mut l3 = event.listen();

    assert!(!is_notified(l1.as_mut()));
    assert!(!is_notified(l2.as_mut()));
    assert!(!is_notified(l3.as_mut()));

    assert_eq!(event.notify(2), 2);
    assert_eq!(event.notify(1), 0);

    assert!(is_notified(l1.as_mut()));
    assert!(is_notified(l2.as_mut()));
    assert!(!is_notified(l3.as_mut()));
}

#[test]
fn notify_additional() {
    let event = Event::new();

    let mut l1 = event.listen();
    let mut l2 = event.listen();
    let mut l3 = event.listen();

    assert_eq!(event.notify_additional(1), 1);
    assert_eq!(event.notify(1), 0);
    assert_eq!(event.notify_additional(1), 1);

    assert!(is_notified(l1.as_mut()));
    assert!(is_notified(l2.as_mut()));
    assert!(!is_notified(l3.as_mut()));
}

#[test]
fn notify_one() {
    let event = Event::new();

    let mut l1 = event.listen();
    let mut l2 = event.listen();

    assert!(!is_notified(l1.as_mut()));
    assert!(!is_notified(l2.as_mut()));

    assert_eq!(event.notify(1), 1);
    assert!(is_notified(l1.as_mut()));
    assert!(!is_notified(l2.as_mut()));

    assert_eq!(event.notify(1), 1);
    assert!(is_notified(l2.as_mut()));
}

#[test]
fn notify_all() {
    let event = Event::new();

    let mut l1 = event.listen();
    let mut l2 = event.listen();

    assert!(!is_notified(l1.as_mut()));
    assert!(!is_notified(l2.as_mut()));

    assert_eq!(event.notify(usize::MAX), 2);
    assert!(is_notified(l1.as_mut()));
    assert!(is_notified(l2.as_mut()));
}

#[test]
fn drop_notified() {
    let event = Event::new();

    let l1 = event.listen();
    let mut l2 = event.listen();
    let mut l3 = event.listen();

    assert_eq!(event.notify(1), 1);
    drop(l1);
    assert!(is_notified(l2.as_mut()));
    assert!(!is_notified(l3.as_mut()));
}

#[test]
fn drop_notified2() {
    let event = Event::new();

    let l1 = event.listen();
    let mut l2 = event.listen();
    let mut l3 = event.listen();

    assert_eq!(event.notify(2), 2);
    drop(l1);
    assert!(is_notified(l2.as_mut()));
    assert!(!is_notified(l3.as_mut()));
}

#[test]
fn drop_notified_additional() {
    let event = Event::new();

    let l1 = event.listen();
    let mut l2 = event.listen();
    let mut l3 = event.listen();
    let mut l4 = event.listen();

    assert_eq!(event.notify_additional(1), 1);
    assert_eq!(event.notify(2), 1);
    drop(l1);
    assert!(is_notified(l2.as_mut()));
    assert!(is_notified(l3.as_mut()));
    assert!(!is_notified(l4.as_mut()));
}

#[test]
fn drop_non_notified() {
    let event = Event::new();

    let mut l1 = event.listen();
    let mut l2 = event.listen();
    let l3 = event.listen();

    assert_eq!(event.notify(1), 1);
    drop(l3);
    assert!(is_notified(l1.as_mut()));
    assert!(!is_notified(l2.as_mut()));
}

#[test]
fn notify_all_fair() {
    let event = Event::new();
    let v = Arc::new(Mutex::new(vec![]));

    let mut l1 = event.listen();
    let mut l2 = event.listen();
    let mut l3 = event.listen();

    let waker1 = {
        let v = v.clone();
        waker_fn(move || v.lock().unwrap().push(1))
    };
    let waker2 = {
        let v = v.clone();
        waker_fn(move || v.lock().unwrap().push(2))
    };
    let waker3 = {
        let v = v.clone();
        waker_fn(move || v.lock().unwrap().push(3))
    };

    assert!(Pin::new(&mut l1)
        .poll(&mut Context::from_waker(&waker1))
        .is_pending());
    assert!(Pin::new(&mut l2)
        .poll(&mut Context::from_waker(&waker2))
        .is_pending());
    assert!(Pin::new(&mut l3)
        .poll(&mut Context::from_waker(&waker3))
        .is_pending());

    assert_eq!(event.notify(usize::MAX), 3);
    assert_eq!(&*v.lock().unwrap(), &[1, 2, 3]);

    assert!(Pin::new(&mut l1)
        .poll(&mut Context::from_waker(&waker1))
        .is_ready());
    assert!(Pin::new(&mut l2)
        .poll(&mut Context::from_waker(&waker2))
        .is_ready());
    assert!(Pin::new(&mut l3)
        .poll(&mut Context::from_waker(&waker3))
        .is_ready());
}

#[test]
fn more_than_one_event() {
    let event = Event::new();
    let event2 = Event::new();

    let mut listener = Box::pin(EventListener::<()>::new());
    listener.as_mut().listen(&event);
    listener.as_mut().listen(&event2);

    drop(listener);
    event.notify(1);
}
