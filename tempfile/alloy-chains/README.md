# alloy-chains

Canonical type definitions for EIP-155 chains.

## Supported Rust Versions

<!--
When updating this, also update:
- clippy.toml
- Cargo.toml
- .github/workflows/ci.yml
-->

Alloy will keep a rolling MSRV (minimum supported rust version) policy of **at
least** 6 months. When increasing the MSRV, the new Rust version must have been
released at least six months ago. The current MSRV is 1.81.0.

Note that the MSRV is not increased automatically, and only as part of a minor
release.

## Adding a new chain

Check `src/named.rs`'s comment for the guidelines on how to add a new chain.

## Note on `no_std`

All crates in this workspace should support `no_std` environments, with the
`alloc` crate. If you find a crate that does not support `no_std`, please
[open an issue].

[open an issue]: https://github.com/alloy-rs/chains/issues/new/choose

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in these crates by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
</sub>
