# alloy-transport

<!-- TODO: More links and real doctests -->

Low-level Ethereum JSON-RPC transport abstraction.

This crate handles RPC connection and request management. It builds an
`RpcClient` on top of the [tower `Service`] abstraction, and provides
futures for simple and batch RPC requests as well as a unified `TransportError`
type.

Typically, this crate should not be used directly. Most EVM users will want to
use the [alloy-provider] crate, which provides a high-level API for interacting
with JSON-RPC servers that provide the standard Ethereum RPC endpoints, or the
[alloy-rpc-client] crate, which provides a low-level JSON-RPC API without the
specific Ethereum endpoints.

[alloy-provider]: https://docs.rs/alloy_provider/
[tower `Service`]: https://docs.rs/tower/latest/tower/trait.Service.html

### Transports

Alloy maintains the following transports:

- [alloy-transport-http]: JSON-RPC via HTTP.
- [alloy-transport-ws]: JSON-RPC via Websocket, supports pubsub via
    [alloy-pubsub].
- [alloy-transport-ipc]: JSON-RPC via IPC, supports pubsub via [alloy-pubsub].

[alloy-transport-http]: https://docs.rs/alloy_transport_http/
[alloy-transport-ws]: https://docs.rs/alloy_transport_ws/
[alloy-transport-ipc]: https://docs.rs/alloy_transport_ipc/
[alloy-pubsub]: https://docs.rs/alloy_pubsub/
