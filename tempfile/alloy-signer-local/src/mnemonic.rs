//! Specific helper functions for creating/loading a mnemonic private key following [BIP-39]
//! specifications.
//!
//! [BIP-39]: https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki

use crate::{LocalSigner, LocalSignerError, PrivateKeySigner};
use alloy_signer::utils::secret_key_to_address;
use coins_bip32::path::DerivationPath;
use coins_bip39::{English, Mnemonic, Wordlist};
use k256::ecdsa::SigningKey;
use rand::Rng;
use std::{marker::PhantomData, path::PathBuf};
use thiserror::Error;

const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";
const DEFAULT_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";

/// Represents a structure that can resolve into a `PrivateKeySigner`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "builders do nothing unless `build` is called"]
pub struct MnemonicBuilder<W: Wordlist = English> {
    /// The mnemonic phrase can be supplied to the builder as a string. A builder that has a valid
    /// phrase should `build` the signer.
    phrase: Option<String>,
    /// The mnemonic builder can also be asked to generate a new random signer by providing the
    /// number of words in the phrase. By default this is set to 12.
    word_count: usize,
    /// The derivation path at which the extended private key child will be derived at. By default
    /// the mnemonic builder uses the path: "m/44'/60'/0'/0/0".
    derivation_path: DerivationPath,
    /// Optional password for the mnemonic phrase.
    password: Option<String>,
    /// Optional field that if enabled, writes the mnemonic phrase to disk storage at the provided
    /// path.
    write_to: Option<PathBuf>,
    /// PhantomData
    _wordlist: PhantomData<W>,
}

/// Error produced by the mnemonic signer module.
#[derive(Debug, Error)]
pub enum MnemonicBuilderError {
    /// Error suggests that a phrase (path or words) was expected but not found.
    #[error("expected phrase not found")]
    ExpectedPhraseNotFound,
    /// Error suggests that a phrase (path or words) was not expected but found.
    #[error("unexpected phrase found")]
    UnexpectedPhraseFound,
}

impl<W: Wordlist> Default for MnemonicBuilder<W> {
    fn default() -> Self {
        Self {
            phrase: None,
            word_count: 12usize,
            derivation_path: DEFAULT_DERIVATION_PATH.parse().unwrap(),
            password: None,
            write_to: None,
            _wordlist: PhantomData,
        }
    }
}

impl<W: Wordlist> MnemonicBuilder<W> {
    /// Sets the phrase in the mnemonic builder. The phrase can either be a string or a path to
    /// the file that contains the phrase. Once a phrase is provided, the key will be generated
    /// deterministically by calling the `build` method.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn foo() -> Result<(), Box<dyn std::error::Error>> {
    /// use alloy_signer_local::{MnemonicBuilder, coins_bip39::English};
    ///
    /// let signer = MnemonicBuilder::<English>::default()
    ///     .phrase("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about")
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn phrase<P: Into<String>>(mut self, phrase: P) -> Self {
        self.phrase = Some(phrase.into());
        self
    }

    /// Sets the word count of a mnemonic phrase to be generated at random. If the `phrase` field
    /// is set, then `word_count` will be ignored.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn foo() -> Result<(), Box<dyn std::error::Error>> {
    /// use alloy_signer_local::{coins_bip39::English, MnemonicBuilder};
    ///
    /// let signer = MnemonicBuilder::<English>::default().word_count(24).build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub const fn word_count(mut self, count: usize) -> Self {
        self.word_count = count;
        self
    }

    /// Sets the derivation path of the child key to be derived. The derivation path is calculated
    /// using the default derivation path prefix used in Ethereum, i.e. "m/44'/60'/0'/0/{index}".
    pub fn index(self, index: u32) -> Result<Self, LocalSignerError> {
        self.derivation_path(format!("{DEFAULT_DERIVATION_PATH_PREFIX}{index}"))
    }

    /// Sets the derivation path of the child key to be derived.
    pub fn derivation_path<T: AsRef<str>>(mut self, path: T) -> Result<Self, LocalSignerError> {
        self.derivation_path = path.as_ref().parse()?;
        Ok(self)
    }

    /// Sets the password used to construct the seed from the mnemonic phrase.
    pub fn password<T: Into<String>>(mut self, password: T) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the path to which the randomly generated phrase will be written to. This field is
    /// ignored when building a wallet from the provided mnemonic phrase.
    pub fn write_to<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.write_to = Some(path.into());
        self
    }

    /// Builds a `PrivateKeySigner` using the parameters set in mnemonic builder. This method
    /// expects the phrase field to be set.
    pub fn build(&self) -> Result<PrivateKeySigner, LocalSignerError> {
        let mnemonic = match &self.phrase {
            Some(phrase) => Mnemonic::<W>::new_from_phrase(phrase)?,
            None => return Err(MnemonicBuilderError::ExpectedPhraseNotFound.into()),
        };
        self.mnemonic_to_signer(&mnemonic)
    }

    /// Builds a `PrivateKeySigner` using the parameters set in the mnemonic builder and
    /// constructing the phrase using the thread RNG.
    pub fn build_random(&self) -> Result<PrivateKeySigner, LocalSignerError> {
        self.build_random_with(&mut rand::thread_rng())
    }

    /// Builds a `PrivateKeySigner` using the parameters set in the mnemonic builder and
    /// constructing the phrase using the provided random number generator.
    pub fn build_random_with<R: Rng>(
        &self,
        rng: &mut R,
    ) -> Result<PrivateKeySigner, LocalSignerError> {
        let mnemonic = match &self.phrase {
            None => Mnemonic::<W>::new_with_count(rng, self.word_count)?,
            _ => return Err(MnemonicBuilderError::UnexpectedPhraseFound.into()),
        };
        let signer = self.mnemonic_to_signer(&mnemonic)?;

        // Write the mnemonic phrase to storage if a directory has been provided.
        if let Some(dir) = &self.write_to {
            std::fs::write(dir.join(signer.address.to_string()), mnemonic.to_phrase().as_bytes())?;
        }

        Ok(signer)
    }

    fn mnemonic_to_signer(
        &self,
        mnemonic: &Mnemonic<W>,
    ) -> Result<PrivateKeySigner, LocalSignerError> {
        let derived_priv_key =
            mnemonic.derive_key(&self.derivation_path, self.password.as_deref())?;
        let key: &coins_bip32::prelude::SigningKey = derived_priv_key.as_ref();
        let credential = SigningKey::from_bytes(&key.to_bytes())?;
        let address = secret_key_to_address(&credential);
        Ok(LocalSigner::<SigningKey> { credential, address, chain_id: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coins_bip39::English;
    use tempfile::tempdir;

    const TEST_DERIVATION_PATH: &str = "m/44'/60'/0'/2/1";

    #[test]
    fn mnemonic_deterministic() {
        // Testcases have been taken from MyCryptoWallet
        let tests = [
            (
                "work man father plunge mystery proud hollow address reunion sauce theory bonus",
                0u32,
                Some("TREZOR123"),
                "0x431a00DA1D54c281AeF638A73121B3D153e0b0F6",
            ),
            (
                "inject danger program federal spice bitter term garbage coyote breeze thought funny",
                1u32,
                Some("LEDGER321"),
                "0x231a3D0a05d13FAf93078C779FeeD3752ea1350C",
            ),
            (
                "fire evolve buddy tenant talent favorite ankle stem regret myth dream fresh",
                2u32,
                None,
                "0x1D86AD5eBb2380dAdEAF52f61f4F428C485460E9",
            ),
            (
                "thumb soda tape crunch maple fresh imitate cancel order blind denial giraffe",
                3u32,
                None,
                "0xFB78b25f69A8e941036fEE2A5EeAf349D81D4ccc",
            ),
        ];
        for (phrase, index, password, expected_addr) in tests {
            let mut builder =
                MnemonicBuilder::<English>::default().phrase(phrase).index(index).unwrap();
            if let Some(psswd) = password {
                builder = builder.password(psswd);
            }
            let signer = builder.build().unwrap();
            assert_eq!(&signer.address.to_string(), expected_addr);
        }
    }

    #[test]
    fn mnemonic_write_read() {
        let dir = tempdir().unwrap();

        let signer1 = MnemonicBuilder::<English>::default()
            .word_count(24)
            .derivation_path(TEST_DERIVATION_PATH)
            .unwrap()
            .write_to(dir.as_ref())
            .build_random()
            .unwrap();

        // Ensure that only one file has been created.
        let paths = std::fs::read_dir(dir.as_ref()).unwrap();
        assert_eq!(paths.count(), 1);

        // Use the newly created mnemonic to instantiate signer.
        let phrase_path = dir.as_ref().join(signer1.address.to_string());
        let phrase = std::fs::read_to_string(phrase_path).unwrap();
        let signer2 = MnemonicBuilder::<English>::default()
            .phrase(phrase)
            .derivation_path(TEST_DERIVATION_PATH)
            .unwrap()
            .build()
            .unwrap();

        // Ensure that both signers belong to the same address.
        assert_eq!(signer1.address, signer2.address);

        dir.close().unwrap();
    }
}
