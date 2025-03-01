# foundry-block-explorers

Bindings for Etherscan.io and other block explorer APIs.

Originally part of [`ethers-rs`] as [`ethers-etherscan`](https://crates.io/crates/ethers-etherscan).

[`ethers-rs`]: https://github.com/gakonst/ethers-rs

[![Build Status][actions-badge]][actions-url]
[![Telegram chat][telegram-badge]][telegram-url]

[actions-badge]: https://img.shields.io/github/actions/workflow/status/foundry-rs/block-explorers/ci.yml?branch=main&style=for-the-badge
[actions-url]: https://github.com/foundry-rs/block-explorers/actions?query=branch%3Amain
[telegram-badge]: https://img.shields.io/endpoint?color=neon&style=for-the-badge&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_rs
[telegram-url]: https://t.me/foundry_rs

## Examples

```rust,no_run
use alloy_chains::Chain;
use foundry_block_explorers::Client;

async fn foo() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Chain::mainnet(), "<your_api_key>")?;
    // Or using environment variables
    let client = Client::new_from_env(Chain::mainnet())?;

    let address = "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse()?;
    let metadata = client.contract_source_code(address).await?;
    assert_eq!(metadata.items[0].contract_name, "DAO");
    Ok(())
}
```

## Supported Rust Versions

<!--
When updating this, also update:
- clippy.toml
- Cargo.toml
- .github/workflows/ci.yml
-->

Foundry will keep a rolling MSRV (minimum supported rust version) policy of **at
least** 6 months. When increasing the MSRV, the new Rust version must have been
released at least six months ago. The current MSRV is 1.65.0.

Note that the MSRV is not increased automatically, and only as part of a minor
release.

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
