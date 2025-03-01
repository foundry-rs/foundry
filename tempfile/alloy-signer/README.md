# alloy-signer

Ethereum signer abstraction.

You can implement the [`Signer`][Signer] trait to extend functionality to other signers
such as Hardware Security Modules, KMS etc. See [its documentation][Signer] for more.

Signer implementations in Alloy:
- [K256 private key](https://docs.rs/alloy-signer-local)
- [YubiHSM2](https://docs.rs/alloy-signer-local)
- [Ledger](https://docs.rs/alloy-signer-ledger)
- [Trezor](https://docs.rs/alloy-signer-trezor)
- [AWS KMS](https://docs.rs/alloy-signer-aws)
- [GCP KMS](https://docs.rs/alloy-signer-gcp)

[Signer]: https://docs.rs/alloy-signer/latest/alloy_signer/trait.Signer.html

## Examples

Sign an Ethereum prefixed message ([EIP-712](https://eips.ethereum.org/EIPS/eip-712)):

```rust,ignore
use alloy_signer::{Signer, SignerSync};
use alloy_signer_local::PrivateKeySigner;

// Instantiate a signer.
let signer = PrivateKeySigner::random();

// Sign a message.
let message = "Some data";
let signature = signer.sign_message_sync(message.as_bytes())?;

// Recover the signer from the message.
let recovered = signature.recover_address_from_msg(message)?;
assert_eq!(recovered, signer.address());
# Ok::<_, Box<dyn std::error::Error>>(())
```

Sign a transaction:

```rust,ignore
use alloy_consensus::TxLegacy;
use alloy_primitives::{U256, address, bytes};
use alloy_signer::{Signer, SignerSync};
use alloy_signer_local::PrivateKeySigner;
use alloy_network::TxSignerSync;

// Instantiate a signer.
let signer = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
    .parse::<PrivateKeySigner>()?;

// Create a transaction.
let mut tx = TxLegacy {
    to: address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into(),
    value: U256::from(1_000_000_000),
    gas_limit: 2_000_000,
    nonce: 0,
    gas_price: 21_000_000_000,
    input: bytes!(),
    chain_id: Some(1),
};

// Sign it.
let signature = signer.sign_transaction_sync(&mut tx)?;
# Ok::<_, Box<dyn std::error::Error>>(())
```
