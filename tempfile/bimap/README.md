# `bimap-rs`

<!-- badges -->
[![version][version badge]][lib.rs]
[![documentation][documentation badge]][docs.rs]
[![license][license badge]](#license)


`bimap-rs` is a pure Rust library for dealing with bijective maps, aiming to
feel like an extension of the standard library's data structures whenever
possible. There are no external dependencies by default but [Serde] and
[`no_std`] compatibility are available through feature flags.

1. [Quick start](#quick-start)
1. [Feature flags](#feature-flags)
1. [Documentation](#documentation)
1. [Contributing](#contributing)
1. [Semantic versioning](#semantic-versioning)
1. [Minimum supported Rust version](#minimum-supported-rust-version)
1. [License](#license)

## Quick start

To use the latest version of `bimap-rs` with the default features, add this to
your project's `Cargo.toml` file:

```toml
[dependencies]
bimap = "0.6.3"
```

You can now run the `bimap-rs` Hello World!

```rust
fn main() {
    // A bijective map between letters of the English alphabet and their positions.
    let mut alphabet = bimap::BiMap::<char, u8>::new();

    alphabet.insert('A', 1);
    // ...
    alphabet.insert('Z', 26);

    println!("A is at position {}", alphabet.get_by_left(&'A').unwrap());
    println!("{} is at position 26", alphabet.get_by_right(&26).unwrap());
}
```

## Feature flags

| Flag name | Description                        | Enabled by default? |
| ---       | ---                                | ---                 |
| `std`     | Standard library usage (`HashMap`) | yes                 |
| `serde`   | (De)serialization using [Serde]    | no                  |

This `Cargo.toml` shows how these features can be enabled and disabled.

```toml
[dependencies]
# I just want to use `bimap-rs`.
bimap = "0.6.3"

# I want to use `bimap-rs` without the Rust standard library.
bimap = { version = "0.6.3", default-features = false }

# I want to use `bimap-rs` with Serde support.
bimap = { version = "0.6.3", features = ["serde"] }
```

## Documentation

Documentation for the latest version of `bimap-rs` is available on [docs.rs].

## Contributing

Thank you for your interest in improving `bimap-rs`! Please read the [code of
conduct] and the [contributing guidelines] before submitting an issue or
opening a pull request.

## Semantic versioning

`bimap-rs` adheres to the de-facto Rust variety of Semantic Versioning.

## Minimum supported Rust version

| `bimap` | MSRV   |
| ---     | ---    |
| v0.6.3  | 1.56.1 |
| v0.6.2  | 1.56.1 |
| v0.6.1  | 1.42.0 |
| v0.6.0  | 1.38.0 |
| v0.5.3  | 1.38.0 |
| v0.5.2  | 1.38.0 |
| v0.5.1  | 1.38.0 |
| v0.5.0  | 1.38.0 |
| v0.4.0  | 1.38.0 |

## License

`bimap-rs` is dual-licensed under the [Apache License] and the [MIT License].
As a library user, this means that you are free to choose either license when
using `bimap-rs`. As a library contributor, this means that any work you
contribute to `bimap-rs` will be similarly dual-licensed.

<!-- external links -->
[docs.rs]: https://docs.rs/bimap/
[lib.rs]: https://lib.rs/crates/bimap
[`no_std`]: https://rust-embedded.github.io/book/intro/no-std.html
[Serde]: https://serde.rs/

<!-- local files -->
[Apache License]: LICENSE_APACHE
[code of conduct]: CODE_OF_CONDUCT.md
[contributing guidelines]: CONTRIBUTING.md
[MIT License]: LICENSE_MIT

<!-- static badge images (all purple) -->
[documentation badge]: https://img.shields.io/static/v1?label=documentation&message=docs.rs&color=blueviolet
[license badge]: https://img.shields.io/static/v1?label=license&message=Apache-2.0/MIT&color=blueviolet
[version badge]: https://img.shields.io/static/v1?label=latest%20version&message=lib.rs&color=blueviolet
