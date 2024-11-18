# foundry-cheatcodes

Foundry cheatcodes definitions and implementations.

## Structure

- [`assets/`](./assets/): JSON interface and specification
- [`spec/`](./spec/src/lib.rs): Defines common traits and structs
- [`src/`](./src/lib.rs): Rust implementations of the cheatcodes

## Overview

All cheatcodes are defined in a single [`sol!`] macro call in [`spec/src/vm.rs`].

This, combined with the use of an internal [`Cheatcode`](../../crates/cheatcodes/spec/src/cheatcode.rs) derive macro,
allows us to generate both the Rust definitions and the JSON specification of the cheatcodes.

Cheatcodes are manually implemented through the `Cheatcode` trait, which is called in the
`Cheatcodes` inspector implementation.

See the [cheatcodes dev documentation](../../docs/dev/cheatcodes.md#cheatcodes-implementation) for more details.

### JSON interface

The JSON interface is guaranteed to be stable, and can be used by third-party tools to interact with
the Foundry cheatcodes externally.

For example, here are some tools that make use of the JSON interface:
- Internally, this is used to generate [a simple Solidity interface](../../testdata/cheats/Vm.sol) for testing
- Used by [`forge-std`](https://github.com/foundry-rs/forge-std) to generate [user-friendly Solidity interfaces](https://github.com/foundry-rs/forge-std/blob/master/src/Vm.sol)
- (WIP) Used by [the Foundry book](https://github.com/foundry-rs/book) to generate [the cheatcodes reference](https://book.getfoundry.sh/cheatcodes)
- ...

If you are making use of the JSON interface, please don't hesitate to open a PR to add your project to this list!

### Adding a new cheatcode

Please see the [cheatcodes dev documentation](../../docs/dev/cheatcodes.md#adding-a-new-cheatcode) on how to add new cheatcodes.

[`sol!`]: https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html
[`spec/src/vm.rs`]: ./spec/src/vm.rs
