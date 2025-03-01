# alloy-signer-local

Local signer implementations:

- [K256 private key](https://docs.rs/alloy-signer-local/latest/alloy_signer_local/struct.LocalSigner.html)
<!-- TODO: use the struct URL in these once it appears on docs.rs -->
- [Mnemonic phrase](https://docs.rs/alloy-signer-local/)
- [YubiHSM2](https://docs.rs/alloy-signer-local/)

## Features

- `keystore`: enables Ethereum keystore functionality on the `PrivateKeySigner` type.
- `mnemonic`: enables BIP-39 mnemonic functionality for building `PrivateKeySigner`s.
- `yubihsm`: enables `LocalSigner`s with [YubiHSM2] support.

[YubiHSM2]: https://www.yubico.com/products/hardware-security-module/
