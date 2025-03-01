# alloy-sol-macro-input

This crate contains inputs to the `sol!` macro. It sits in-between
the `sol-macro` and `syn-solidity` crates, and contains an intermediate
representation of Solidity items. These items are then expanded into
Rust code by the `alloy-sol-macro` crate.

This crate is not meant to be used directly, but rather is a tool for
writing macros that generate Rust code from Solidity code.
