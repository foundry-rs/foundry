# Figment &thinsp; [![ci.svg]][ci] [![crates.io]][crate] [![docs.rs]][docs]

[crates.io]: https://img.shields.io/crates/v/figment.svg
[crate]: https://crates.io/crates/figment
[docs.rs]: https://docs.rs/figment/badge.svg
[docs]: https://docs.rs/figment
[ci.svg]: https://github.com/SergioBenitez/Figment/workflows/CI/badge.svg
[ci]: https://github.com/SergioBenitez/Figment/actions

Figment is a semi-hierarchical configuration library for Rust so con-free, it's
_unreal_.

```rust
use serde::Deserialize;

use figment::{Figment, providers::{Format, Toml, Json, Env}};

#[derive(Deserialize)]
struct Package {
    name: String,
    authors: Vec<String>,
    publish: Option<bool>,
    // ... and so on ...
}

#[derive(Deserialize)]
struct Config {
    package: Package,
    rustc: Option<String>,
    // ... and so on ...
}

let config: Config = Figment::new()
    .merge(Toml::file("Cargo.toml"))
    .merge(Env::prefixed("CARGO_"))
    .merge(Env::raw().only(&["RUSTC", "RUSTDOC"]))
    .join(Json::file("Cargo.json"))
    .extract()?;
```

See the [documentation](https://docs.rs/figment) for a detailed usage guide and
information.

## Usage

Add the following to your `Cargo.toml`, enabling the desired built-in providers:

```toml
[dependencies]
figment = { version = "0.10", features = ["toml", "env"] }
```

#### Third-Party Providers

The following external libraries implement Figment providers:

 - [`figment_file_provider_adapter`](https://crates.io/crates/figment_file_provider_adapter)

   Wraps existing providers. For any key ending in `_FILE` (configurable), emits
   a key without the `_FILE` suffix with a value corresponding to the contents
   of the file whose path is the original key's value.

Please submit a pull request to add your library to this list.

## License

Figment is licensed under either of the following, at your option:

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
