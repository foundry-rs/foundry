#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use alloy_consensus::SignableTransaction;
use alloy_network::{TxSigner, TxSignerSync};
use alloy_primitives::{Address, ChainId, PrimitiveSignature as Signature, B256};
use alloy_signer::{sign_transaction_with_chain_id, Result, Signer, SignerSync};
use async_trait::async_trait;
use k256::ecdsa::{self, signature::hazmat::PrehashSigner, RecoveryId};
use std::fmt;

mod error;
pub use error::LocalSignerError;

#[cfg(feature = "mnemonic")]
mod mnemonic;
#[cfg(feature = "mnemonic")]
pub use mnemonic::{MnemonicBuilder, MnemonicBuilderError};

mod private_key;

#[cfg(feature = "yubihsm")]
mod yubi;

#[cfg(feature = "yubihsm")]
pub use yubihsm;

#[cfg(feature = "mnemonic")]
pub use coins_bip39;

/// A signer instantiated with a locally stored private key.
pub type PrivateKeySigner = LocalSigner<k256::ecdsa::SigningKey>;

#[doc(hidden)]
#[deprecated(note = "use `PrivateKeySigner` instead")]
pub type LocalWallet = PrivateKeySigner;

/// A signer instantiated with a YubiHSM.
#[cfg(feature = "yubihsm")]
pub type YubiSigner = LocalSigner<yubihsm::ecdsa::Signer<k256::Secp256k1>>;

#[cfg(feature = "yubihsm")]
#[doc(hidden)]
#[deprecated(note = "use `YubiSigner` instead")]
pub type YubiWallet = YubiSigner;

/// An Ethereum private-public key pair which can be used for signing messages.
///
/// # Examples
///
/// ## Signing and Verifying a message
///
/// The signer can be used to produce ECDSA [`Signature`] objects, which can be
/// then verified. Note that this uses
/// [`eip191_hash_message`](alloy_primitives::eip191_hash_message) under the hood which will
/// prefix the message being hashed with the `Ethereum Signed Message` domain separator.
///
/// ```
/// use alloy_signer::{Signer, SignerSync};
/// use alloy_signer_local::PrivateKeySigner;
///
/// let signer = PrivateKeySigner::random();
///
/// // Optionally, the signer's chain id can be set, in order to use EIP-155
/// // replay protection with different chains
/// let signer = signer.with_chain_id(Some(1337));
///
/// // The signer can be used to sign messages
/// let message = b"hello";
/// let signature = signer.sign_message_sync(message)?;
/// assert_eq!(signature.recover_address_from_msg(&message[..]).unwrap(), signer.address());
///
/// // LocalSigner is cloneable:
/// let signer_clone = signer.clone();
/// let signature2 = signer_clone.sign_message_sync(message)?;
/// assert_eq!(signature, signature2);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct LocalSigner<C> {
    /// The signer's credential.
    pub(crate) credential: C,
    /// The signer's address.
    pub(crate) address: Address,
    /// The signer's chain ID (for EIP-155).
    pub(crate) chain_id: Option<ChainId>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<C: PrehashSigner<(ecdsa::Signature, RecoveryId)> + Send + Sync> Signer for LocalSigner<C> {
    #[inline]
    async fn sign_hash(&self, hash: &B256) -> Result<Signature> {
        self.sign_hash_sync(hash)
    }

    #[inline]
    fn address(&self) -> Address {
        self.address
    }

    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        self.chain_id
    }

    #[inline]
    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        self.chain_id = chain_id;
    }
}

impl<C: PrehashSigner<(ecdsa::Signature, RecoveryId)>> SignerSync for LocalSigner<C> {
    #[inline]
    fn sign_hash_sync(&self, hash: &B256) -> Result<Signature> {
        Ok(self.credential.sign_prehash(hash.as_ref())?.into())
    }

    #[inline]
    fn chain_id_sync(&self) -> Option<ChainId> {
        self.chain_id
    }
}

impl<C: PrehashSigner<(ecdsa::Signature, RecoveryId)>> LocalSigner<C> {
    /// Construct a new credential with an external [`PrehashSigner`].
    #[inline]
    pub const fn new_with_credential(
        credential: C,
        address: Address,
        chain_id: Option<ChainId>,
    ) -> Self {
        Self { credential, address, chain_id }
    }

    /// Returns this signer's credential.
    #[inline]
    pub const fn credential(&self) -> &C {
        &self.credential
    }

    /// Consumes this signer and returns its credential.
    #[inline]
    pub fn into_credential(self) -> C {
        self.credential
    }

    /// Returns this signer's address.
    #[inline]
    pub const fn address(&self) -> Address {
        self.address
    }

    /// Returns this signer's chain ID.
    #[inline]
    pub const fn chain_id(&self) -> Option<ChainId> {
        self.chain_id
    }
}

// do not log the signer
impl<C: PrehashSigner<(ecdsa::Signature, RecoveryId)>> fmt::Debug for LocalSigner<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalSigner")
            .field("address", &self.address)
            .field("chain_id", &self.chain_id)
            .finish()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<C> TxSigner<Signature> for LocalSigner<C>
where
    C: PrehashSigner<(ecdsa::Signature, RecoveryId)> + Send + Sync,
{
    fn address(&self) -> Address {
        self.address
    }

    #[doc(alias = "sign_tx")]
    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature> {
        sign_transaction_with_chain_id!(self, tx, self.sign_hash_sync(&tx.signature_hash()))
    }
}

impl<C> TxSignerSync<Signature> for LocalSigner<C>
where
    C: PrehashSigner<(ecdsa::Signature, RecoveryId)>,
{
    fn address(&self) -> Address {
        self.address
    }

    #[doc(alias = "sign_tx_sync")]
    fn sign_transaction_sync(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature> {
        sign_transaction_with_chain_id!(self, tx, self.sign_hash_sync(&tx.signature_hash()))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use alloy_consensus::TxLegacy;
    use alloy_primitives::{address, U256};

    #[tokio::test]
    async fn signs_tx() {
        async fn sign_tx_test(tx: &mut TxLegacy, chain_id: Option<ChainId>) -> Result<Signature> {
            let mut before = tx.clone();
            let sig = sign_dyn_tx_test(tx, chain_id).await?;
            if let Some(chain_id) = chain_id {
                assert_eq!(tx.chain_id, Some(chain_id), "chain ID was not set");
                before.chain_id = Some(chain_id);
            }
            assert_eq!(*tx, before);
            Ok(sig)
        }

        async fn sign_dyn_tx_test(
            tx: &mut dyn SignableTransaction<Signature>,
            chain_id: Option<ChainId>,
        ) -> Result<Signature> {
            let mut signer: PrivateKeySigner =
                "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".parse().unwrap();
            signer.set_chain_id(chain_id);

            let sig = signer.sign_transaction_sync(tx)?;
            let sighash = tx.signature_hash();
            assert_eq!(sig.recover_address_from_prehash(&sighash).unwrap(), signer.address());

            let sig_async = signer.sign_transaction(tx).await.unwrap();
            assert_eq!(sig_async, sig);

            Ok(sig)
        }

        // retrieved test vector from:
        // https://web3js.readthedocs.io/en/v1.2.0/web3-eth-accounts.html#eth-accounts-signtransaction
        let mut tx = TxLegacy {
            to: address!("F0109fC8DF283027b6285cc889F5aA624EaC1F55").into(),
            value: U256::from(1_000_000_000),
            gas_limit: 2_000_000,
            nonce: 0,
            gas_price: 21_000_000_000,
            input: Default::default(),
            chain_id: None,
        };
        let sig_none = sign_tx_test(&mut tx, None).await.unwrap();

        tx.chain_id = Some(1);
        let sig_1 = sign_tx_test(&mut tx, None).await.unwrap();
        let expected = "c9cf86333bcb065d140032ecaab5d9281bde80f21b9687b3e94161de42d51895727a108a0b8d101465414033c3f705a9c7b826e596766046ee1183dbc8aeaa6825".parse().unwrap();
        assert_eq!(sig_1, expected);
        assert_ne!(sig_1, sig_none);

        tx.chain_id = Some(2);
        let sig_2 = sign_tx_test(&mut tx, None).await.unwrap();
        assert_ne!(sig_2, sig_1);
        assert_ne!(sig_2, sig_none);

        // Sets chain ID.
        tx.chain_id = None;
        let sig_none_none = sign_tx_test(&mut tx, None).await.unwrap();
        assert_eq!(sig_none_none, sig_none);

        tx.chain_id = None;
        let sig_none_1 = sign_tx_test(&mut tx, Some(1)).await.unwrap();
        assert_eq!(sig_none_1, sig_1);

        tx.chain_id = None;
        let sig_none_2 = sign_tx_test(&mut tx, Some(2)).await.unwrap();
        assert_eq!(sig_none_2, sig_2);

        // Errors on mismatch.
        tx.chain_id = Some(2);
        let error = sign_tx_test(&mut tx, Some(1)).await.unwrap_err();
        let expected_error = alloy_signer::Error::TransactionChainIdMismatch { signer: 1, tx: 2 };
        assert_eq!(error.to_string(), expected_error.to_string());
    }

    // <https://github.com/alloy-rs/core/issues/705>
    #[test]
    fn test_parity() {
        let signer = PrivateKeySigner::random();
        let message = b"hello";
        let signature = signer.sign_message_sync(message).unwrap();
        let value = signature.as_bytes().to_vec();
        let recovered_signature: Signature = value.as_slice().try_into().unwrap();
        assert_eq!(signature, recovered_signature);
    }
}
