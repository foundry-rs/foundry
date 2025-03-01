# alloy-provider

<!-- TODO: links, docs, examples, etc -->

Interface with an Ethereum blockchain.

This crate contains the `Provider` trait, which exposes Ethereum JSON-RPC
methods. Providers in alloy are similar to [`ethers.js`] providers. They manage
an `RpcClient` and allow other parts of the program to easily make RPC calls.

Unlike an [`ethers.js`] Provider, an alloy Provider is network-aware. It is
parameterized with a `Network` from [`alloy-networks`]. This allows the Provider
to expose a consistent interface to the rest of the program, while adjusting
request and response types to match the underlying blockchain.

Providers can be composed via stacking. For example, a `Provider` that tracks
the nonce for a given address can be stacked onto a `Provider` that signs
transactions to create a `Provider` that can send signed transactions with
correct nonces.

The `ProviderBuilder` struct can quickly create a stacked provider, similar to
[`tower::ServiceBuilder`].

[alloy-networks]: ../networks/
[`tower::ServiceBuilder`]: https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html
[`ethers.js`]: https://docs.ethers.org/v6/

## Feature flags

- `pubsub` - Enable support for subscription methods.
- `ws` - Enable WebSocket support. Implicitly enables `pubsub`.
- `ipc` - Enable IPC support. Implicitly enables `pubsub`.

## Usage

TODO :)
