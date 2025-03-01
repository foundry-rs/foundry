#![warn(
    missing_docs,
    missing_copy_implementations,
    missing_debug_implementations,
    unreachable_pub,
    unused_crate_dependencies,
    clippy::missing_const_for_fn
)]

//! The bip39 crate is heavily inspired by and reuses code from
//! [Wagyu](https://github.com/AleoHQ/wagyu) under the [MIT](http://opensource.org/licenses/MIT)
//! license. The difference being, the underlying extended private keys are generated using the
//! [bip32](https://github.com/summa-tx/bitcoins-rs/tree/main/bip32) crate, that depends on
//! [k256](https://docs.rs/k256/0.10.0/k256/index.html) instead of
//! [libsecp256k1](https://docs.rs/libsecp256k1/0.3.5/secp256k1/).

/// Mnemonic phrases
pub mod mnemonic;
pub use self::mnemonic::*;

/// Wordlists
pub mod wordlist;
pub use self::wordlist::*;
