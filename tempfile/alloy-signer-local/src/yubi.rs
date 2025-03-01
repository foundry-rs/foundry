//! [YubiHSM2](yubihsm) signer implementation.

use super::LocalSigner;
use alloy_signer::utils::raw_public_key_to_address;
use elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use k256::{PublicKey, Secp256k1};
use yubihsm::{
    asymmetric::Algorithm::EcK256, ecdsa::Signer as YubiSigner, object, object::Label, Capability,
    Client, Connector, Credentials, Domain,
};

impl LocalSigner<YubiSigner<Secp256k1>> {
    /// Connects to a yubi key's ECDSA account at the provided id
    pub fn connect(connector: Connector, credentials: Credentials, id: object::Id) -> Self {
        let client = Client::open(connector, credentials, true).unwrap();
        let signer = YubiSigner::create(client, id).unwrap();
        signer.into()
    }

    /// Creates a new random ECDSA keypair on the yubi at the provided id
    pub fn new(
        connector: Connector,
        credentials: Credentials,
        id: object::Id,
        label: Label,
        domain: Domain,
    ) -> Self {
        let client = Client::open(connector, credentials, true).unwrap();
        let id = client
            .generate_asymmetric_key(id, label, domain, Capability::SIGN_ECDSA, EcK256)
            .unwrap();
        let signer = YubiSigner::create(client, id).unwrap();
        signer.into()
    }

    /// Uploads the provided keypair on the yubi at the provided id
    pub fn from_key(
        connector: Connector,
        credentials: Credentials,
        id: object::Id,
        label: Label,
        domain: Domain,
        key: impl Into<Vec<u8>>,
    ) -> Self {
        let client = Client::open(connector, credentials, true).unwrap();
        let id = client
            .put_asymmetric_key(id, label, domain, Capability::SIGN_ECDSA, EcK256, key)
            .unwrap();
        let signer = YubiSigner::create(client, id).unwrap();
        signer.into()
    }
}

impl From<YubiSigner<Secp256k1>> for LocalSigner<YubiSigner<Secp256k1>> {
    fn from(credential: YubiSigner<Secp256k1>) -> Self {
        // Uncompress the public key.
        let pubkey = PublicKey::from_encoded_point(credential.public_key()).unwrap();
        let pubkey = pubkey.to_encoded_point(false);
        let bytes = pubkey.as_bytes();
        debug_assert_eq!(bytes[0], 0x04);
        let address = raw_public_key_to_address(&bytes[1..]);
        Self::new_with_credential(credential, address, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SignerSync;
    use alloy_primitives::{address, hex};

    #[test]
    fn from_key() {
        let key = hex::decode("2d8c44dc2dd2f0bea410e342885379192381e82d855b1b112f9b55544f1e0900")
            .unwrap();

        let connector = yubihsm::Connector::mockhsm();
        let signer = LocalSigner::from_key(
            connector,
            Credentials::default(),
            0,
            Label::from_bytes(&[]).unwrap(),
            Domain::at(1).unwrap(),
            key,
        );

        let msg = "Some data";
        let sig = signer.sign_message_sync(msg.as_bytes()).unwrap();
        assert_eq!(sig.recover_address_from_msg(msg).unwrap(), signer.address());
        assert_eq!(signer.address(), address!("2DE2C386082Cff9b28D62E60983856CE1139eC49"));
    }

    #[test]
    fn new_key() {
        let connector = yubihsm::Connector::mockhsm();
        let signer = LocalSigner::<YubiSigner<Secp256k1>>::new(
            connector,
            Credentials::default(),
            0,
            Label::from_bytes(&[]).unwrap(),
            Domain::at(1).unwrap(),
        );

        let msg = "Some data";
        let sig = signer.sign_message_sync(msg.as_bytes()).unwrap();
        assert_eq!(sig.recover_address_from_msg(msg).unwrap(), signer.address());
    }
}
