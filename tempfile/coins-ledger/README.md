# coins-ledger

Communication library between Rust and Ledger Nano S/X devices

# Building

Windows is not yet supported.

### Native

- Install dependencies
  - OSX
    - TODO
    - please file an issue if you know. I don't have a macbook :)
- Build with native transport
  - `cargo build`

### WASM

- Install wasm-pack
  - [Link here](https://rustwasm.github.io/wasm-pack/installer/)
- MUST pass `--disable-default-features`
- MUST select feature AT MOST ONE of `browser` and `node`
- Build with node WASM bindings to `@ledgerhq/hw-transport-node-hid`
  - `wasm-pack build --scope summa-tx --target nodejs -- --features=node --no-default-features`
  - Runtime environment MUST be able to import `@ledgerhq/hw-transport-node-hid`
- Build with browser WASM bindings to `@ledgerhq/hw-transport-u2f`
  - `wasm-pack build --scope summa-tx --target bundler -- --features=broswer --no-default-features`
  - Runtime environment MUST be able to import `@ledgerhq/hw-transport-u2f`

# Features

The `node` and `browser` features are mutually exclusive. You must specify
exactly one, as well as the `--no-default-features` flag.

When building for non-wasm architectures, a native HID transport is compiled
in. When building wasm via `wasm-pack`, you must specify whether you want the
node or browser wasm transport.

# Testing

- run the unit tests
  - `$ cargo test -- --lib`
- run the integration tests
  - Plug in a Ledger Nano S or X device
  - Unlock the device
  - Open the Ethereum application on the device
    - If you don't have the application, [install Ledger Live](https://support.ledger.com/hc/en-us/articles/360006395553) and follow [these instructions](https://support.ledger.com/hc/en-us/articles/360006523674-Install-or-uninstall-apps)
  - `$ cargo test`

# License Notes

This repo was forked from [Zondax's repo](https://github.com/Zondax/ledger-rs)
at commit [`7d40af96`](https://github.com/Zondax/ledger-rs/commit/7d40af9653d04e2d40f8b0c031675b6ff82d7f2c).
Their code is reproduced here under the terms of the Apache 2 License. Files
containing elements from their code maintain their original Apache 2 license
notice at the bottom of the file.

Further work by Summa is available under the GNU LGPLv3 license.

These changes are as follows:

- Remove bip44 crates
- Significant refactoring to all other crates
- Crates have been moved to be modules of a single crate
- Refactor APDUErrorCodes
- Refactor APDUCommand to move towards no_std support. They hold &'a [u8]
  instead of vectors
- Refactor APDUAnswer to move towards no_std support and avoid unnecessary
  copies. It is now a thin wrapper around a &[u8]
- Change exchange functions to accept a mutable buffer. The caller must allocate
  space for the response packet
- `wasm_bindgen` bindings for JS ledger transports
- Conditional compilation based abstraction of transport type
  - Native HID if not wasm32
  - Feature flags for browser or node if wasm32
- Break out integration tests
- Strip print logs
