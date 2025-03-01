//! [`k256`] signer implementation.

use super::{LocalSigner, LocalSignerError};
use alloy_primitives::{hex, B256};
use alloy_signer::utils::secret_key_to_address;
use k256::{
    ecdsa::{self, SigningKey},
    FieldBytes, NonZeroScalar, SecretKey as K256SecretKey,
};
use rand::{CryptoRng, Rng};
use std::str::FromStr;

#[cfg(feature = "keystore")]
use std::path::Path;

impl LocalSigner<SigningKey> {
    /// Creates a new [`LocalSigner`] instance from a [`SigningKey`].
    ///
    /// This can also be used to create a [`LocalSigner`] from a [`SecretKey`](K256SecretKey).
    /// See also the `From` implementations.
    #[doc(alias = "from_private_key")]
    #[doc(alias = "new_private_key")]
    #[doc(alias = "new_pk")]
    #[inline]
    pub fn from_signing_key(credential: SigningKey) -> Self {
        let address = secret_key_to_address(&credential);
        Self::new_with_credential(credential, address, None)
    }

    /// Creates a new [`LocalSigner`] instance from a raw scalar serialized as a [`B256`] byte
    /// array.
    ///
    /// This is identical to [`from_field_bytes`](Self::from_field_bytes).
    #[inline]
    pub fn from_bytes(bytes: &B256) -> Result<Self, ecdsa::Error> {
        Self::from_field_bytes((&bytes.0).into())
    }

    /// Creates a new [`LocalSigner`] instance from a raw scalar serialized as a [`FieldBytes`] byte
    /// array.
    #[inline]
    pub fn from_field_bytes(bytes: &FieldBytes) -> Result<Self, ecdsa::Error> {
        SigningKey::from_bytes(bytes).map(Self::from_signing_key)
    }

    /// Creates a new [`LocalSigner`] instance from a raw scalar serialized as a byte slice.
    ///
    /// Byte slices shorter than the field size (32 bytes) are handled by zero padding the input.
    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ecdsa::Error> {
        SigningKey::from_slice(bytes).map(Self::from_signing_key)
    }

    /// Creates a new random keypair seeded with [`rand::thread_rng()`].
    #[inline]
    pub fn random() -> Self {
        Self::random_with(&mut rand::thread_rng())
    }

    /// Creates a new random keypair seeded with the provided RNG.
    #[inline]
    pub fn random_with<R: Rng + CryptoRng>(rng: &mut R) -> Self {
        Self::from_signing_key(SigningKey::random(rng))
    }

    /// Borrow the secret [`NonZeroScalar`] value for this key.
    ///
    /// # ⚠️ Warning
    ///
    /// This value is key material.
    ///
    /// Please treat it with the care it deserves!
    #[inline]
    pub fn as_nonzero_scalar(&self) -> &NonZeroScalar {
        self.credential.as_nonzero_scalar()
    }

    /// Serialize this [`LocalSigner`]'s [`SigningKey`] as a [`B256`] byte array.
    #[inline]
    pub fn to_bytes(&self) -> B256 {
        B256::new(<[u8; 32]>::from(self.to_field_bytes()))
    }

    /// Serialize this [`LocalSigner`]'s [`SigningKey`] as a [`FieldBytes`] byte array.
    #[inline]
    pub fn to_field_bytes(&self) -> FieldBytes {
        self.credential.to_bytes()
    }
}

#[cfg(feature = "keystore")]
impl LocalSigner<SigningKey> {
    /// Creates a new random encrypted JSON with the provided password and stores it in the
    /// provided directory. Returns a tuple (LocalSigner, String) of the signer instance for the
    /// keystore with its random UUID. Accepts an optional name for the keystore file. If `None`,
    /// the keystore is stored as the stringified UUID.
    #[inline]
    pub fn new_keystore<P, R, S>(
        dir: P,
        rng: &mut R,
        password: S,
        name: Option<&str>,
    ) -> Result<(Self, String), LocalSignerError>
    where
        P: AsRef<Path>,
        R: Rng + CryptoRng,
        S: AsRef<[u8]>,
    {
        let (secret, uuid) = eth_keystore::new(dir, rng, password, name)?;
        Ok((Self::from_slice(&secret)?, uuid))
    }

    /// Decrypts an encrypted JSON from the provided path to construct a [`LocalSigner`] instance
    #[inline]
    pub fn decrypt_keystore<P, S>(keypath: P, password: S) -> Result<Self, LocalSignerError>
    where
        P: AsRef<Path>,
        S: AsRef<[u8]>,
    {
        let secret = eth_keystore::decrypt_key(keypath, password)?;
        Ok(Self::from_slice(&secret)?)
    }

    /// Creates a new encrypted JSON with the provided private key and password and stores it in the
    /// provided directory. Returns a tuple (LocalSigner, String) of the signer instance for the
    /// keystore with its random UUID. Accepts an optional name for the keystore file. If `None`,
    /// the keystore is stored as the stringified UUID.
    #[inline]
    pub fn encrypt_keystore<P, R, B, S>(
        keypath: P,
        rng: &mut R,
        pk: B,
        password: S,
        name: Option<&str>,
    ) -> Result<(Self, String), LocalSignerError>
    where
        P: AsRef<Path>,
        R: Rng + CryptoRng,
        B: AsRef<[u8]>,
        S: AsRef<[u8]>,
    {
        let pk = pk.as_ref();
        let uuid = eth_keystore::encrypt_key(keypath, rng, pk, password, name)?;
        Ok((Self::from_slice(pk)?, uuid))
    }
}

impl PartialEq for LocalSigner<SigningKey> {
    fn eq(&self, other: &Self) -> bool {
        self.credential.to_bytes().eq(&other.credential.to_bytes())
            && self.address == other.address
            && self.chain_id == other.chain_id
    }
}

impl From<SigningKey> for LocalSigner<SigningKey> {
    fn from(value: SigningKey) -> Self {
        Self::from_signing_key(value)
    }
}

impl From<K256SecretKey> for LocalSigner<SigningKey> {
    fn from(value: K256SecretKey) -> Self {
        Self::from_signing_key(value.into())
    }
}

impl FromStr for LocalSigner<SigningKey> {
    type Err = LocalSignerError;

    fn from_str(src: &str) -> Result<Self, Self::Err> {
        let array = hex::decode_to_array::<_, 32>(src)?;
        Ok(Self::from_slice(&array)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PrivateKeySigner, SignerSync};
    use alloy_primitives::{address, b256};

    #[cfg(feature = "keystore")]
    use tempfile::tempdir;

    #[test]
    fn parse_pk() {
        let s = "6f142508b4eea641e33cb2a0161221105086a84584c74245ca463a49effea30b";
        let _pk: PrivateKeySigner = s.parse().unwrap();
    }

    #[test]
    fn parse_short_key() {
        let s = "6f142508b4eea641e33cb2a0161221105086a84584c74245ca463a49effea3";
        assert!(s.len() < 64);
        let pk = s.parse::<PrivateKeySigner>().unwrap_err();
        match pk {
            LocalSignerError::HexError(hex::FromHexError::InvalidStringLength) => {}
            _ => panic!("Unexpected error"),
        }
    }

    #[cfg(feature = "keystore")]
    fn test_encrypted_json_keystore(key: LocalSigner<SigningKey>, uuid: &str, dir: &Path) {
        // sign a message using the given key
        let message = "Some data";
        let signature = key.sign_message_sync(message.as_bytes()).unwrap();

        // read from the encrypted JSON keystore and decrypt it, while validating that the
        // signatures produced by both the keys should match
        let path = Path::new(dir).join(uuid);
        let key2 = LocalSigner::<SigningKey>::decrypt_keystore(path.clone(), "randpsswd").unwrap();

        let signature2 = key2.sign_message_sync(message.as_bytes()).unwrap();
        assert_eq!(signature, signature2);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    #[cfg(feature = "keystore")]
    fn encrypted_json_keystore_new() {
        // create and store an encrypted JSON keystore in this directory
        let dir = tempdir().unwrap();
        let mut rng = rand::thread_rng();
        let (key, uuid) =
            LocalSigner::<SigningKey>::new_keystore(&dir, &mut rng, "randpsswd", None).unwrap();

        test_encrypted_json_keystore(key, &uuid, dir.path());
    }

    #[test]
    #[cfg(feature = "keystore")]
    fn encrypted_json_keystore_from_pk() {
        // create and store an encrypted JSON keystore in this directory
        let dir = tempdir().unwrap();
        let mut rng = rand::thread_rng();

        let private_key =
            hex::decode("6f142508b4eea641e33cb2a0161221105086a84584c74245ca463a49effea30b")
                .unwrap();

        let (key, uuid) = LocalSigner::<SigningKey>::encrypt_keystore(
            &dir,
            &mut rng,
            private_key,
            "randpsswd",
            None,
        )
        .unwrap();

        test_encrypted_json_keystore(key, &uuid, dir.path());
    }

    #[test]
    #[cfg(feature = "keystore-geth-compat")]
    fn test_encrypted_json_keystore_with_address() {
        // create and store an encrypted JSON keystore in this directory

        use std::fs::File;

        use eth_keystore::EthKeystore;
        let dir = tempdir().unwrap();
        let mut rng = rand::thread_rng();
        let (key, uuid) =
            LocalSigner::<SigningKey>::new_keystore(&dir, &mut rng, "randpsswd", None).unwrap();

        let path = Path::new(dir.path()).join(uuid.clone());
        let file = File::open(path).unwrap();
        let keystore = serde_json::from_reader::<_, EthKeystore>(file).unwrap();

        assert!(!keystore.address.is_zero());

        test_encrypted_json_keystore(key, &uuid, dir.path());
    }

    #[test]
    fn signs_msg() {
        let message = "Some data";
        let hash = alloy_primitives::utils::eip191_hash_message(message);
        let key = LocalSigner::<SigningKey>::random_with(&mut rand::thread_rng());
        let address = key.address;

        // sign a message
        let signature = key.sign_message_sync(message.as_bytes()).unwrap();

        // ecrecover via the message will hash internally
        let recovered = signature.recover_address_from_msg(message).unwrap();
        assert_eq!(recovered, address);

        // if provided with a hash, it will skip hashing
        let recovered2 = signature.recover_address_from_prehash(&hash).unwrap();
        assert_eq!(recovered2, address);
    }

    #[test]
    #[cfg(feature = "eip712")]
    fn typed_data() {
        use alloy_dyn_abi::eip712::TypedData;
        use alloy_primitives::{keccak256, Address, I256, U256};
        use alloy_sol_types::{eip712_domain, sol, SolStruct};
        use serde::Serialize;

        sol! {
            #[derive(Debug, Serialize)]
            struct FooBar {
                int256 foo;
                uint256 bar;
                bytes fizz;
                bytes32 buzz;
                string far;
                address out;
            }
        }

        let domain = eip712_domain! {
            name: "Eip712Test",
            version: "1",
            chain_id: 1,
            verifying_contract: address!("0000000000000000000000000000000000000001"),
            salt: keccak256("eip712-test-75F0CCte"),
        };
        let foo_bar = FooBar {
            foo: I256::try_from(10u64).unwrap(),
            bar: U256::from(20u64),
            fizz: b"fizz".to_vec().into(),
            buzz: keccak256("buzz"),
            far: "space".into(),
            out: Address::ZERO,
        };
        let signer = LocalSigner::random();
        let hash = foo_bar.eip712_signing_hash(&domain);
        let sig = signer.sign_typed_data_sync(&foo_bar, &domain).unwrap();
        assert_eq!(sig.recover_address_from_prehash(&hash).unwrap(), signer.address());
        assert_eq!(signer.sign_hash_sync(&hash).unwrap(), sig);
        let foo_bar_dynamic = TypedData::from_struct(&foo_bar, Some(domain));
        let dynamic_hash = foo_bar_dynamic.eip712_signing_hash().unwrap();
        let sig_dynamic = signer.sign_dynamic_typed_data_sync(&foo_bar_dynamic).unwrap();
        assert_eq!(
            sig_dynamic.recover_address_from_prehash(&dynamic_hash).unwrap(),
            signer.address()
        );
        assert_eq!(signer.sign_hash_sync(&dynamic_hash).unwrap(), sig_dynamic);
    }

    #[test]
    fn key_to_address() {
        let signer: LocalSigner<SigningKey> =
            "0000000000000000000000000000000000000000000000000000000000000001".parse().unwrap();
        assert_eq!(signer.address, address!("7E5F4552091A69125d5DfCb7b8C2659029395Bdf"));

        let signer: LocalSigner<SigningKey> =
            "0000000000000000000000000000000000000000000000000000000000000002".parse().unwrap();
        assert_eq!(signer.address, address!("2B5AD5c4795c026514f8317c7a215E218DcCD6cF"));

        let signer: LocalSigner<SigningKey> =
            "0000000000000000000000000000000000000000000000000000000000000003".parse().unwrap();
        assert_eq!(signer.address, address!("6813Eb9362372EEF6200f3b1dbC3f819671cBA69"));
    }

    #[test]
    fn conversions() {
        let key = b256!("0000000000000000000000000000000000000000000000000000000000000001");

        let signer_b256: LocalSigner<SigningKey> = LocalSigner::from_bytes(&key).unwrap();
        assert_eq!(signer_b256.address, address!("7E5F4552091A69125d5DfCb7b8C2659029395Bdf"));
        assert_eq!(signer_b256.chain_id, None);
        assert_eq!(signer_b256.credential, SigningKey::from_bytes((&key.0).into()).unwrap());

        let signer_str = LocalSigner::from_str(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
        assert_eq!(signer_str.address, signer_b256.address);
        assert_eq!(signer_str.chain_id, signer_b256.chain_id);
        assert_eq!(signer_str.credential, signer_b256.credential);
        assert_eq!(signer_str.to_bytes(), key);
        assert_eq!(signer_str.to_field_bytes(), key.0.into());

        let signer_slice = LocalSigner::from_slice(&key[..]).unwrap();
        assert_eq!(signer_slice.address, signer_b256.address);
        assert_eq!(signer_slice.chain_id, signer_b256.chain_id);
        assert_eq!(signer_slice.credential, signer_b256.credential);
        assert_eq!(signer_slice.to_bytes(), key);
        assert_eq!(signer_slice.to_field_bytes(), key.0.into());

        let signer_field_bytes = LocalSigner::from_field_bytes((&key.0).into()).unwrap();
        assert_eq!(signer_field_bytes.address, signer_b256.address);
        assert_eq!(signer_field_bytes.chain_id, signer_b256.chain_id);
        assert_eq!(signer_field_bytes.credential, signer_b256.credential);
        assert_eq!(signer_field_bytes.to_bytes(), key);
        assert_eq!(signer_field_bytes.to_field_bytes(), key.0.into());
    }

    #[test]
    fn key_from_str() {
        let signer: LocalSigner<SigningKey> =
            "0000000000000000000000000000000000000000000000000000000000000001".parse().unwrap();

        // Check FromStr and `0x`
        let signer_0x: LocalSigner<SigningKey> =
            "0x0000000000000000000000000000000000000000000000000000000000000001".parse().unwrap();
        assert_eq!(signer.address, signer_0x.address);
        assert_eq!(signer.chain_id, signer_0x.chain_id);
        assert_eq!(signer.credential, signer_0x.credential);

        // Must fail because of `0z`
        "0z0000000000000000000000000000000000000000000000000000000000000001"
            .parse::<LocalSigner<SigningKey>>()
            .unwrap_err();
    }
}
