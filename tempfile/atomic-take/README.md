# Atomic Take

![License](https://img.shields.io/badge/license-MIT-green.svg)
[![Cargo](https://img.shields.io/crates/v/atomic-take.svg)](https://crates.io/crates/atomic-take)
[![Documentation](https://docs.rs/atomic-take/badge.svg)](https://docs.rs/atomic-take)

This crate allows you to store a value that you can later take out atomically. As this
crate uses atomics, no locking is involved in taking the value out.

As an example, you could store the [`Sender`] of an oneshot channel in an
[`AtomicTake`], which would allow you to notify the first time a closure is called.

```rust
use atomic_take::AtomicTake;
use tokio::sync::oneshot;

let (send, mut recv) = oneshot::channel();

let take = AtomicTake::new(send);
let closure = move || {
    if let Some(send) = take.take() {
        // Notify the first time this closure is called.
        send.send(()).unwrap();
    }
};

closure();
assert_eq!(recv.try_recv().unwrap(), Some(()));

closure(); // This does nothing.
```

Additionally the closure above can be called concurrently from many threads. For
example, if you put the `AtomicTake` in an [`Arc`], you can share it between several
threads and receive a message from the first thread to run.

```rust
use std::thread;
use std::sync::Arc;
use atomic_take::AtomicTake;
use tokio::sync::oneshot;

let (send, mut recv) = oneshot::channel();

// Use an Arc to share the AtomicTake between several threads.
let take = Arc::new(AtomicTake::new(send));

// Spawn three threads and try to send a message from each.
let mut handles = Vec::new();
for i in 0..3 {
    let take_clone = Arc::clone(&take);
    let join_handle = thread::spawn(move || {

        // Check if this thread is first and send a message if so.
        if let Some(send) = take_clone.take() {
            // Send the index of the thread.
            send.send(i).unwrap();
        }

    });
    handles.push(join_handle);
}
// Wait for all three threads to finish.
for handle in handles {
    handle.join().unwrap();
}

// After all the threads finished, try to send again.
if let Some(send) = take.take() {
    // This will definitely not happen.
    send.send(100).unwrap();
}

// Confirm that one of the first three threads got to send the message first.
assert!(recv.try_recv().unwrap().unwrap() < 3);
```

This crate does not require the standard library.

[`Sender`]: https://docs.rs/tokio/latest/tokio/sync/oneshot/struct.Sender.html
[`AtomicTake`]: https://docs.rs/atomic-take/latest/atomic_take/struct.AtomicTake.html
[`Arc`]: https://doc.rust-lang.org/std/sync/struct.Arc.html

# Supported Rust Versions

The current MSRV is 1.48.0. It may also work on earlier compiler versions, but
they are not tested in CI when changes are made.

# License

This project is licensed under the MIT license.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, shall be licensed as MIT, without any
additional terms or conditions.
