# coins-bip39

This is an implementation of [BIP39](https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki). It is heavily inspired by and reuses code from [Wagyu](https://github.com/AleoHQ/wagyu) under the [MIT](http://opensource.org/licenses/MIT) license. It uses the [coins-bip32](https://github.com/summa-tx/bitcoins-rs/tree/main/bip32) to derive extended keys.

## Building

```
$ cargo build
$ cargo build --target wasm32-unknown-unknown
```

Run tests (make sure to run with all feature combinations):
```
$ cargo test
```
