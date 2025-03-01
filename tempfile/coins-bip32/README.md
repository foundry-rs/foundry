# coins-bip32

This is an implementation of
[BIP32](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki). It
uses [k256](https://docs.rs/k256/0.7.1/k256/index.html) and re-exports several
of its traits and types.

It can be used to build wallets and applications for Bitcoin and Ethereum.

## Building

```
$ cargo build
$ cargo build --target wasm32-unknown-unknown
```

Run tests (make sure to run with all feature combinations):
```
$ cargo test
```

Run bench marks
```
$ cargo bench
$ cargo bench --no-default-features
```