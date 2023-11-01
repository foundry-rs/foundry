# foundry-cheatcodes

Foundry cheatcodes definitions and implementations.

## Structure

- [`assets/`](./assets/): JSON interface and specification
- [`src/defs`](./src/defs/mod.rs): Defines traits and structs
- [`src/impls`](./src/impls/mod.rs): Rust implementations of the cheatcodes. This is gated to the `impl` feature, since these are not needed when only using the definitions.

## Overview

All cheatcodes are defined in a single [`sol!`] macro call in [`src/defs/vm.rs`].

This, combined with the use of an internal [`Cheatcode`](../macros/impl/src/cheatcodes.rs) derive macro,
allows us to generate both the Rust definitions and the JSON specification of the cheatcodes.

Cheatcodes are manually implemented through the `Cheatcode` trait, which is called in the
`Cheatcodes` inspector implementation.

See the [cheatcodes dev documentation](../../docs/dev/cheatcodes.md#cheatcodes-implementation) for more details.

### JSON interface

The JSON interface is guaranteed to be stable, and can be used by third-party tools to interact with
the Foundry cheatcodes externally.

For example, here are some tools that make use of the JSON interface:
- Internally, this is used to generate [a simple Solidity interface](../../testdata/cheats/Vm.sol) for testing
- (WIP) Used by [`forge-std`](https://github.com/foundry-rs/forge-std) to generate [user-friendly Solidity interfaces](https://github.com/foundry-rs/forge-std/blob/master/src/Vm.sol)
- (WIP) Used by [the Foundry book](https://github.com/foundry-rs/book) to generate [the cheatcodes reference](https://book.getfoundry.sh/cheatcodes)
- ...

If you are making use of the JSON interface, please don't hesitate to open a PR to add your project to this list!

### Adding a new cheatcode

Please see the [cheatcodes dev documentation](../../docs/dev/cheatcodes.md#adding-a-new-cheatcode) on how to add new cheatcodes.

[`sol!`]: https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html
[`src/defs/vm.rs`]: ./src/defs/vm.rs
