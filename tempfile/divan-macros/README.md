<div align="center">
    <h1>Divan</h1>
    <a href="https://docs.rs/divan">
        <img src="https://img.shields.io/crates/v/divan.svg?label=docs&color=blue&logo=rust" alt="docs.rs badge">
    </a>
    <a href="https://crates.io/crates/divan">
        <img src="https://img.shields.io/crates/d/divan.svg" alt="Downloads badge">
    </a>
    <a href="https://github.com/nvzqz/divan">
        <img src="https://img.shields.io/github/stars/nvzqz/divan.svg?style=flat&color=black" alt="GitHub stars badge">
    </a>
    <a href="https://github.com/nvzqz/divan/actions/workflows/ci.yml">
        <img src="https://github.com/nvzqz/divan/actions/workflows/ci.yml/badge.svg" alt="CI build status badge">
    </a>
    <p>
        <strong>Comfy bench</strong>marking for Rust projects, brought to you by
        <a href="https://nikolaivazquez.com">Nikolai Vazquez</a>.
    </p>
</div>

## Sponsor

If you or your company find Divan valuable, consider [sponsoring on
GitHub](https://github.com/sponsors/nvzqz) or [donating via
PayPal](https://paypal.me/nvzqz). Sponsorships help me progress on what's
possible with benchmarking in Rust.

## Guide

A guide is being worked on. In the meantime, see:
- [Announcement post](https://nikolaivazquez.com/blog/divan/)
- ["Proving Performance" FOSDEM talk](https://youtu.be/P87C4jNakGs)

## Getting Started

Divan `0.1.17` requires Rust `1.80.0` or later.

1. Add the following to your project's [`Cargo.toml`](https://doc.rust-lang.org/cargo/reference/manifest.html):

    ```toml
    [dev-dependencies]
    divan = "0.1.17"

    [[bench]]
    name = "example"
    harness = false
    ```

2. Create a benchmarks file at `benches/example.rs`[^1] with your benchmarking code:

    ```rust
    fn main() {
        // Run registered benchmarks.
        divan::main();
    }

    // Register a `fibonacci` function and benchmark it over multiple cases.
    #[divan::bench(args = [1, 2, 4, 8, 16, 32])]
    fn fibonacci(n: u64) -> u64 {
        if n <= 1 {
            1
        } else {
            fibonacci(n - 2) + fibonacci(n - 1)
        }
    }
    ```

3. Run your benchmarks with [`cargo bench`](https://doc.rust-lang.org/cargo/commands/cargo-bench.html):

    ```txt
    example       fastest  │ slowest  │ median   │ mean     │ samples │ iters
    ╰─ fibonacci           │          │          │          │         │
       ├─ 1       0.626 ns │ 1.735 ns │ 0.657 ns │ 0.672 ns │ 100     │ 819200
       ├─ 2       2.767 ns │ 3.154 ns │ 2.788 ns │ 2.851 ns │ 100     │ 204800
       ├─ 4       6.816 ns │ 7.671 ns │ 7.061 ns │ 7.167 ns │ 100     │ 102400
       ├─ 8       57.31 ns │ 62.51 ns │ 57.96 ns │ 58.55 ns │ 100     │ 12800
       ├─ 16      2.874 µs │ 3.812 µs │ 2.916 µs │ 3.006 µs │ 100     │ 200
       ╰─ 32      6.267 ms │ 6.954 ms │ 6.283 ms │ 6.344 ms │ 100     │ 100
    ```

See [`#[divan::bench]`][bench_attr] for info on benchmark function registration.

## Examples

Practical example benchmarks can be found in the [`examples/benches`](https://github.com/nvzqz/divan/tree/main/examples/benches)
directory. These can be benchmarked locally by running:

```sh
git clone https://github.com/nvzqz/divan.git
cd divan

cargo bench -q -p examples --all-features
```

More thorough usage examples can be found in the [`#[divan::bench]` documentation][bench_attr_examples].

## License

Like the Rust project, this library may be used under either the
[MIT License](https://github.com/nvzqz/divan/blob/main/LICENSE-MIT) or
[Apache License (Version 2.0)](https://github.com/nvzqz/divan/blob/main/LICENSE-APACHE).

[^1]: Within your crate directory, i.e. [`$CARGO_MANIFEST_DIR`](https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates)

[bench_attr]: https://docs.rs/divan/latest/divan/attr.bench.html
[bench_attr_examples]: https://docs.rs/divan/latest/divan/attr.bench.html#examples
