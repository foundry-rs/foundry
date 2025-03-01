# annotate-snippets

`annotate-snippets` is a Rust library for annotation of programming code slices.

[![crates.io](https://img.shields.io/crates/v/annotate-snippets.svg)](https://crates.io/crates/annotate-snippets)
[![documentation](https://img.shields.io/badge/docs-master-blue.svg)][Documentation]
![build status](https://github.com/rust-lang/annotate-snippets-rs/actions/workflows/ci.yml/badge.svg)

The library helps visualize meta information annotating source code slices.
It takes a data structure called `Snippet` on the input and produces a `String`
which may look like this:

![Screenshot](./examples/expected_type.svg)

Local Development
-----------------

    cargo build
    cargo test

When submitting a PR please use  [`cargo fmt`][] (nightly).

[`cargo fmt`]: https://github.com/rust-lang/rustfmt

[Documentation]: https://docs.rs/annotate-snippets/
