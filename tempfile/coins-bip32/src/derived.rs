use k256::ecdsa;

use coins_core::prelude::{Hash160, Hash160Digest, MarkedDigest, MarkedDigestOutput};

use crate::{
    path::{DerivationPath, KeyDerivation},
    primitives::{Hint, KeyFingerprint, XKeyInfo},
    xkeys::{Parent, XPriv, XPub, SEED},
    Bip32Error,
};

/// Derived keys are keys coupled with their derivation. We use this trait to
/// check ancestry relationships between keys.
pub trait DerivedKey {
    /// Return this key's derivation
    fn derivation(&self) -> &KeyDerivation;

    /// `true` if the keys share a root fingerprint, `false` otherwise. Note that on key
    /// fingerprints, which may collide accidentally, or be intentionally collided.
    fn same_root<K: DerivedKey>(&self, other: &K) -> bool {
        self.derivation().same_root(other.derivation())
    }

    /// `true` if this key is a possible ancestor of the argument, `false` otherwise.
    ///
    /// Warning: this check is cheap, but imprecise. It simply compares the root fingerprints
    /// (which may collide) and checks that `self.path` is a prefix of `other.path`. This may be
    /// deliberately foold by an attacker. For a precise check, use
    /// `DerivedXPriv::is_private_ancestor_of()` or
    /// `DerivedXPub::is_public_ancestor_of()`
    fn is_possible_ancestor_of<K: DerivedKey>(&self, other: &K) -> bool {
        self.derivation()
            .is_possible_ancestor_of(other.derivation())
    }

    /// Returns the path to the descendant, or `None` if the argument is definitely not a
    /// descendant.
    ///
    /// This is useful for determining the path to reach some descendant from some ancestor.
    fn path_to_descendant<K: DerivedKey>(&self, other: &K) -> Option<DerivationPath> {
        self.derivation().path_to_descendant(other.derivation())
    }
}

/// An XPriv with its derivation.
#[derive(Debug, Clone)]
#[cfg_attr(
    any(feature = "mainnet", feature = "testnet"),
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct DerivedXPriv {
    xpriv: XPriv,
    derivation: KeyDerivation,
}

inherit_signer!(DerivedXPriv.xpriv);

impl AsRef<XPriv> for DerivedXPriv {
    fn as_ref(&self) -> &XPriv {
        &self.xpriv
    }
}

impl AsRef<XKeyInfo> for DerivedXPriv {
    fn as_ref(&self) -> &XKeyInfo {
        &self.xpriv.xkey_info
    }
}

impl AsRef<ecdsa::SigningKey> for DerivedXPriv {
    fn as_ref(&self) -> &ecdsa::SigningKey {
        &self.xpriv.key
    }
}

impl DerivedKey for DerivedXPriv {
    fn derivation(&self) -> &KeyDerivation {
        &self.derivation
    }
}

impl DerivedXPriv {
    /// Instantiate a derived XPub from the XPub and derivatin. This usually
    /// should not be called directly. Prefer deriving keys from parents.
    pub const fn new(xpriv: XPriv, derivation: KeyDerivation) -> Self {
        Self { xpriv, derivation }
    }

    /// Check if this XPriv is the private ancestor of some other derived key.
    /// To check ancestry of another private key, derive its public key first
    pub fn is_private_ancestor_of(&self, other: &DerivedXPub) -> Result<bool, Bip32Error> {
        if let Some(path) = self.path_to_descendant(other) {
            let descendant = self.derive_path(path)?;
            dbg!(descendant.verify_key());
            dbg!(&other);
            Ok(descendant.verify_key() == *other)
        } else {
            Ok(false)
        }
    }

    /// Generate a customized root node using the stati
    pub fn root_node(
        hmac_key: &[u8],
        data: &[u8],
        hint: Option<Hint>,
    ) -> Result<DerivedXPriv, Bip32Error> {
        Self::custom_root_node(hmac_key, data, hint)
    }

    /// Generate a root node from some seed data. Uses the BIP32-standard hmac key.
    ///
    ///
    /// # Important:
    ///
    /// Use a seed of AT LEAST 128 bits.
    pub fn root_from_seed(data: &[u8], hint: Option<Hint>) -> Result<DerivedXPriv, Bip32Error> {
        Self::custom_root_from_seed(data, hint)
    }

    /// Instantiate a root node using a custom HMAC key.
    pub fn custom_root_node(
        hmac_key: &[u8],
        data: &[u8],
        hint: Option<Hint>,
    ) -> Result<DerivedXPriv, Bip32Error> {
        let xpriv = XPriv::custom_root_node(hmac_key, data, hint)?;

        let derivation = KeyDerivation {
            root: xpriv.fingerprint(),
            path: vec![].into(),
        };

        Ok(DerivedXPriv { xpriv, derivation })
    }

    /// Generate a root node from some seed data. Uses the BIP32-standard hmac key.
    ///
    ///
    /// # Important:
    ///
    /// Use a seed of AT LEAST 128 bits.
    pub fn custom_root_from_seed(
        data: &[u8],
        hint: Option<Hint>,
    ) -> Result<DerivedXPriv, Bip32Error> {
        Self::custom_root_node(SEED, data, hint)
    }

    /// Derive the corresponding xpub
    pub fn verify_key(&self) -> DerivedXPub {
        DerivedXPub {
            xpub: self.xpriv.verify_key(),
            derivation: self.derivation.clone(),
        }
    }
}

impl Parent for DerivedXPriv {
    fn derive_child(&self, index: u32) -> Result<Self, Bip32Error> {
        Ok(Self {
            xpriv: self.xpriv.derive_child(index)?,
            derivation: self.derivation.extended(index),
        })
    }
}

/// An XPub with its derivation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    any(feature = "mainnet", feature = "testnet"),
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct DerivedXPub {
    xpub: XPub,
    derivation: KeyDerivation,
}

inherit_verifier!(DerivedXPub.xpub);

impl AsRef<XPub> for DerivedXPub {
    fn as_ref(&self) -> &XPub {
        &self.xpub
    }
}

impl AsRef<XKeyInfo> for DerivedXPub {
    fn as_ref(&self) -> &XKeyInfo {
        &self.xpub.xkey_info
    }
}

impl AsRef<ecdsa::VerifyingKey> for DerivedXPub {
    fn as_ref(&self) -> &ecdsa::VerifyingKey {
        &self.xpub.key
    }
}

impl Parent for DerivedXPub {
    fn derive_child(&self, index: u32) -> Result<Self, Bip32Error> {
        Ok(Self {
            xpub: self.xpub.derive_child(index)?,
            derivation: self.derivation.extended(index),
        })
    }
}

impl DerivedKey for DerivedXPub {
    fn derivation(&self) -> &KeyDerivation {
        &self.derivation
    }
}

impl DerivedXPub {
    /// Instantiate a derived XPub from the XPub and derivatin. This usually
    /// should not be called directly. Prefer deriving keys from parents.
    pub const fn new(xpub: XPub, derivation: KeyDerivation) -> Self {
        Self { xpub, derivation }
    }

    /// Check if this XPriv is the private ancestor of some other derived key
    pub fn is_public_ancestor_of(&self, other: &DerivedXPub) -> Result<bool, Bip32Error> {
        if let Some(path) = self.path_to_descendant(other) {
            let descendant = self.derive_path(path)?;
            Ok(descendant == *other)
        } else {
            Ok(false)
        }
    }
}

/// A Pubkey with its derivation. Primarily used by PSBT.
pub struct DerivedPubkey {
    key: ecdsa::VerifyingKey,
    derivation: KeyDerivation,
}

impl std::fmt::Debug for DerivedPubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedPubkey")
            .field("public key", &self.key.to_sec1_bytes())
            .field("key fingerprint", &self.fingerprint())
            .field("derivation", &self.derivation)
            .finish()
    }
}

inherit_verifier!(DerivedPubkey.key);

impl DerivedKey for DerivedPubkey {
    fn derivation(&self) -> &KeyDerivation {
        &self.derivation
    }
}

impl AsRef<ecdsa::VerifyingKey> for DerivedPubkey {
    fn as_ref(&self) -> &ecdsa::VerifyingKey {
        &self.key
    }
}

impl DerivedPubkey {
    /// Instantiate a new `DerivedPubkey`
    pub const fn new(key: ecdsa::VerifyingKey, derivation: KeyDerivation) -> Self {
        Self { key, derivation }
    }

    /// Return the hash of the compressed (Sec1) pubkey.
    pub fn pubkey_hash160(&self) -> Hash160Digest {
        Hash160::digest_marked(self.key.to_sec1_bytes().as_ref())
    }

    /// The fingerprint is the first 4 bytes of the HASH160 of the serialized
    /// public key.
    pub fn fingerprint(&self) -> KeyFingerprint {
        let digest = self.pubkey_hash160();
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&digest.as_slice()[..4]);
        buf.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        enc::{MainnetEncoder, XKeyEncoder},
        path::DerivationPath,
        prelude::*,
        primitives::*,
        BIP32_HARDEN,
    };
    use coins_core::hashes::*;
    use k256::ecdsa::signature::{DigestSigner, DigestVerifier};

    use hex;

    struct KeyDeriv<'a> {
        pub(crate) path: &'a [u32],
    }

    fn validate_descendant(d: &KeyDeriv, m: &DerivedXPriv) {
        let path: DerivationPath = d.path.into();

        let m_pub = m.verify_key();

        let xpriv = m.derive_path(&path).unwrap();
        let xpub = xpriv.verify_key();
        assert!(m.same_root(&xpriv));
        assert!(m.same_root(&xpub));
        assert!(m.is_possible_ancestor_of(&xpriv));
        assert!(m.is_possible_ancestor_of(&xpub));

        let result = m.is_private_ancestor_of(&xpub).expect("should work");

        if !result {
            panic!("failed validate_descendant is_private_ancestor_of");
        }

        let result = m_pub.is_public_ancestor_of(&xpub);

        match result {
            Ok(true) => {}
            Ok(false) => panic!("failed validate_descendant is_public_ancestor_of"),
            Err(_) => {
                let path: DerivationPath = d.path.into();
                assert!(
                    path.last_hardened().1.is_some(),
                    "is_public_ancestor_of failed for unhardened path"
                )
            }
        }

        let derived_path = m
            .path_to_descendant(&xpriv)
            .expect("expected a path to descendant");
        assert_eq!(&path, &derived_path, "derived path is not as expected");
    }

    #[test]
    fn bip32_vector_1() {
        let seed: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

        let xpriv = DerivedXPriv::root_from_seed(&seed, Some(Hint::Legacy)).unwrap();

        let descendants = [
            KeyDeriv {
                path: &[BIP32_HARDEN],
            },
            KeyDeriv {
                path: &[BIP32_HARDEN, 1],
            },
            KeyDeriv {
                path: &[BIP32_HARDEN, 1, 2 + BIP32_HARDEN],
            },
            KeyDeriv {
                path: &[BIP32_HARDEN, 1, 2 + BIP32_HARDEN, 2],
            },
            KeyDeriv {
                path: &[BIP32_HARDEN, 1, 2 + BIP32_HARDEN, 2, 1000000000],
            },
        ];

        for case in descendants.iter() {
            validate_descendant(case, &xpriv);
        }
    }

    #[test]
    fn bip32_vector_2() {
        let seed = hex::decode("fffcf9f6f3f0edeae7e4e1dedbd8d5d2cfccc9c6c3c0bdbab7b4b1aeaba8a5a29f9c999693908d8a8784817e7b7875726f6c696663605d5a5754514e4b484542").unwrap();

        let xpriv = DerivedXPriv::root_from_seed(&seed, Some(Hint::Legacy)).unwrap();

        let descendants = [
            KeyDeriv { path: &[0] },
            KeyDeriv {
                path: &[0, 2147483647 + BIP32_HARDEN],
            },
            KeyDeriv {
                path: &[0, 2147483647 + BIP32_HARDEN, 1],
            },
            KeyDeriv {
                path: &[0, 2147483647 + BIP32_HARDEN, 1, 2147483646 + BIP32_HARDEN],
            },
            KeyDeriv {
                path: &[
                    0,
                    2147483647 + BIP32_HARDEN,
                    1,
                    2147483646 + BIP32_HARDEN,
                    2,
                ],
            },
        ];

        for case in descendants.iter() {
            validate_descendant(case, &xpriv);
        }
    }

    #[test]
    fn bip32_vector_3() {
        let seed = hex::decode("4b381541583be4423346c643850da4b320e46a87ae3d2a4e6da11eba819cd4acba45d239319ac14f863b8d5ab5a0d0c64d2e8a1e7d1457df2e5a3c51c73235be").unwrap();

        let xpriv = DerivedXPriv::root_from_seed(&seed, Some(Hint::Legacy)).unwrap();

        let descendants = [KeyDeriv {
            path: &[BIP32_HARDEN],
        }];

        for case in descendants.iter() {
            validate_descendant(case, &xpriv);
        }
    }

    #[test]
    fn it_can_sign_and_verify() {
        let digest = Hash256::default();
        let mut wrong_digest = Hash256::default();
        wrong_digest.update([0u8]);

        let xpriv_str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi".to_owned();
        let xpriv = MainnetEncoder::xpriv_from_base58(&xpriv_str).unwrap();
        let fake_deriv = KeyDerivation {
            root: [0, 0, 0, 0].into(),
            path: (0..0).collect(),
        };

        let key = DerivedXPriv::new(xpriv, fake_deriv);
        let key_pub = key.verify_key();

        // sign_digest + verify_digest
        let sig: Signature = key.sign_digest(digest.clone());
        key_pub.verify_digest(digest.clone(), &sig).unwrap();

        let err_bad_sig = key_pub.verify_digest(wrong_digest.clone(), &sig);
        match err_bad_sig {
            Err(_) => {}
            _ => panic!("expected signature validation error"),
        }

        let (sig, _): (Signature, RecoveryId) = key.sign_digest(digest.clone());
        key_pub.verify_digest(digest, &sig).unwrap();

        let err_bad_sig = key_pub.verify_digest(wrong_digest.clone(), &sig);
        match err_bad_sig {
            Err(_) => {}
            _ => panic!("expected signature validation error"),
        }
    }

    #[test]
    fn it_can_descendant_sign_and_verify() {
        let digest = Hash256::default();
        let mut wrong_digest = Hash256::default();
        wrong_digest.update([0u8]);

        let path = vec![0u32, 1, 2];

        let xpriv_str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi".to_owned();
        let xpriv = MainnetEncoder::xpriv_from_base58(&xpriv_str).unwrap();
        let fake_deriv = KeyDerivation {
            root: [0, 0, 0, 0].into(),
            path: (0..0).collect(),
        };

        let key = DerivedXPriv::new(xpriv, fake_deriv.clone());
        let key_pub = key.verify_key();
        assert_eq!(key.derivation(), &fake_deriv);

        // sign_digest + verify_digest
        let sig: Signature = key.derive_path(&path).unwrap().sign_digest(digest.clone());
        key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(digest.clone(), &sig)
            .unwrap();

        let err_bad_sig = key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(wrong_digest.clone(), &sig);
        match err_bad_sig {
            Err(_) => {}
            _ => panic!("expected signature validation error"),
        }

        let (sig, _): (Signature, RecoveryId) =
            key.derive_path(&path).unwrap().sign_digest(digest.clone());
        key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(digest.clone(), &sig)
            .unwrap();

        let err_bad_sig = key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(wrong_digest.clone(), &sig);
        match err_bad_sig {
            Err(_) => {}
            _ => panic!("expected signature validation error"),
        }

        // sign + verify
        let sig: Signature = key.derive_path(&path).unwrap().sign_digest(digest.clone());
        key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(digest.clone(), &sig)
            .unwrap();

        let err_bad_sig = key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(wrong_digest.clone(), &sig);
        match err_bad_sig {
            Err(_) => {}
            _ => panic!("expected signature validation error"),
        }

        // sign_recoverable + verify_recoverable
        let (sig, recovery_id): (Signature, RecoveryId) =
            key.derive_path(&path).unwrap().sign_digest(digest.clone());
        key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(digest, &sig)
            .unwrap();

        let err_bad_sig = key_pub
            .derive_path(&path)
            .unwrap()
            .verify_digest(wrong_digest.clone(), &sig);
        match err_bad_sig {
            Err(_) => {}
            _ => panic!("expected signature validation error"),
        }

        // Sig serialize/deserialize
        let der_sig = hex::decode("304402200cc613393c11889ed1384388c9213b7778cfa0c7c2b6fcc080f0296fc8ac87d202205788d8994d61ce901d1ee22c5210994c235f17ddb3c31e0fc0ec9730ecf084ce").unwrap();
        let rsv: [u8; 65] = [
            12, 198, 19, 57, 60, 17, 136, 158, 209, 56, 67, 136, 201, 33, 59, 119, 120, 207, 160,
            199, 194, 182, 252, 192, 128, 240, 41, 111, 200, 172, 135, 210, 87, 136, 216, 153, 77,
            97, 206, 144, 29, 30, 226, 44, 82, 16, 153, 76, 35, 95, 23, 221, 179, 195, 30, 15, 192,
            236, 151, 48, 236, 240, 132, 206, 1,
        ];
        assert_eq!(sig.to_der().as_bytes(), der_sig);
        assert_eq!(&sig, &Signature::from_der(&der_sig).unwrap());
        assert_eq!(sig.r().to_bytes().as_slice(), &rsv[..32]);
        assert_eq!(sig.s().to_bytes().as_slice(), &rsv[32..64]);
        assert_eq!(recovery_id.to_byte(), rsv[64]);
    }

    #[test]
    fn it_instantiates_derived_xprivs_from_seeds() {
        DerivedXPriv::custom_root_from_seed(&[0u8; 32][..], None).unwrap();

        let err_too_short = DerivedXPriv::custom_root_from_seed(&[0u8; 2][..], None);
        match err_too_short {
            Err(Bip32Error::SeedTooShort) => {}
            _ => panic!("expected err too short"),
        }

        let err_too_short = DerivedXPriv::custom_root_from_seed(&[0u8; 2][..], None);
        match err_too_short {
            Err(Bip32Error::SeedTooShort) => {}
            _ => panic!("expected err too short"),
        }
    }

    #[test]
    fn it_checks_ancestry() {
        let m = DerivedXPriv::custom_root_from_seed(&[0u8; 32][..], None).unwrap();
        let m2 = DerivedXPriv::custom_root_from_seed(&[1u8; 32][..], None).unwrap();
        let m_pub = m.verify_key();
        let cases = [
            (&m, &m_pub, true),
            (&m2, &m_pub, false),
            (&m, &m2.verify_key(), false),
            (&m, &m.derive_child(33).unwrap().verify_key(), true),
            (&m, &m_pub.derive_child(33).unwrap(), true),
            (&m, &m2.derive_child(33).unwrap().verify_key(), false),
        ];
        for (i, case) in cases.iter().enumerate() {
            dbg!(i);
            assert_eq!(case.0.is_private_ancestor_of(case.1).unwrap(), case.2);
        }
    }
}
