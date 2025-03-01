# alloy-networks

Ethereum blockchain RPC behavior abstraction.

This crate contains a simple abstraction of the RPC behavior of an
Ethereum-like blockchain. It is intended to be used by the Alloy client to
provide a consistent interface to the rest of the library, regardless of
changes the underlying blockchain makes to the RPC interface.

## Core Model

This crate handles abstracting RPC types. It does not handle the actual
networking. The core model is as follows:

- `Transaction` - A trait that defines an abstract interface for EVM-like
  transactions.
- `Network` - A trait that defines the RPC types for a given blockchain.
  Providers are parameterized by a `Network` type, and use the associated
  types to define the input and output types of the RPC methods.
- TODO: More!!!

## Usage

This crate is not intended to be used directly. It is used by the
[alloy-provider] library and reth to modify the input and output types of the
RPC methods.

This crate will primarily be used by blockchain maintainers to add bespoke RPC
types to the Alloy provider. This is done by implementing the `Network` trait,
and then parameterizing the `Provider` type with the new network type.

For example, to add a new network called `Foo`:

```rust,ignore
// Foo must be a ZST. It is a compile error to use a non-ZST type.
struct Foo;

impl Network for Foo {
    type Transaction = FooTransaction;
    type Block = FooBlock;
    type Header = FooHeader;
    type Receipt = FooReceipt;

    // etc.
}
```

The user may then instantiate a `Provider<Foo>` and use it as normal. This
allows the user to use the same API for all networks, regardless of the
underlying RPC types.

**Note:** If you need to also add custom _methods_ to your network, you should
make an extension trait for `Provider<N>` as follows:

```rust,ignore
#[async_trait]
trait FooProviderExt: Provider<Foo> {
    async fn custom_foo_method(&self) -> RpcResult<Something, TransportError>;

    async fn another_custom_method(&self) -> RpcResult<Something, TransportError>;
}
```

[alloy-provider]: ../provider
