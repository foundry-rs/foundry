# async-priority-channel

[![Build](https://github.com/rmcgibbo/async-priority-channel/workflows/Build%20and%20test/badge.svg)](
https://github.com/rmcgibbo/async-priority-channel/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](
https://github.com/rmcgibbo/async-priority-channel/)
[![Cargo](https://img.shields.io/crates/v/async-priority-channel.svg)](
https://crates.io/crates/async-priority-channel)
[![Documentation](https://docs.rs/async-priority-channel/badge.svg)](
https://docs.rs/async-priority-channel)

An async channel where pending messages are delivered in order of priority.

There are two kinds of channels:

1. Bounded channel with limited capacity.
2. Unbounded channel with unlimited capacity.

A channel has the `Sender` and `Receiver` side. Both sides are cloneable and can be shared
among multiple threads. When sending, you pass in a message and its priority. When receiving,
you'll get back the pending message with the highest priotiy.

When all `Sender`s or all `Receiver`s are dropped, the channel becomes closed. When a
channel is closed, no more messages can be sent, but remaining messages can still be received.

The channel can also be closed manually by calling `Sender::close()` or
`Receiver::close()`. The API and much of the documentation is based on  [async_channel](https://docs.rs/async-channel/1.6.1/async_channel/).

## Examples

```rust
let (s, r) = async_priority_channel::unbounded();

assert_eq!(s.send("Foo", 0).await, Ok(()));
assert_eq!(s.send("Bar", 2).await, Ok(()));
assert_eq!(s.send("Baz", 1).await, Ok(()));
assert_eq!(r.recv().await, Ok(("Bar", 2)));
```

License: Apache-2.0 OR MIT
