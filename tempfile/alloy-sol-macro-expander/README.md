# alloy-sol-macro-expander

This crate contains the expansion logic for a Solidity `proc_macro2::TokenStream`.
It's used to expand and generate Rust bindings from Solidity.

Note: This is not the procedural macro crate, it is intended to be used as library crate.

This crate is used by [`sol!`] macro in the [`alloy-sol-macro`] crate.

> [!WARNING]
> This crate does not have a stable API, and all exposed functions are subject to change.
> We reserve the right to make any breaking changes to this crate without notice.

[`sol!`]: https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html
[`alloy-sol-macro`]: https://crates.io/alloy-sol-macro/
