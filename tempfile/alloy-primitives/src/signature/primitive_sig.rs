#![allow(unknown_lints, unnameable_types)]

use crate::{hex, normalize_v, signature::SignatureError, uint, U256};
use alloc::vec::Vec;
use core::str::FromStr;

/// The order of the secp256k1 curve
const SECP256K1N_ORDER: U256 =
    uint!(0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141_U256);

/// An Ethereum ECDSA signature.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct PrimitiveSignature {
    y_parity: bool,
    r: U256,
    s: U256,
}

impl<'a> TryFrom<&'a [u8]> for PrimitiveSignature {
    type Error = SignatureError;

    /// Parses a raw signature which is expected to be 65 bytes long where
    /// the first 32 bytes is the `r` value, the second 32 bytes the `s` value
    /// and the final byte is the `v` value in 'Electrum' notation.
    fn try_from(bytes: &'a [u8]) -> Result<Self, Self::Error> {
        if bytes.len() != 65 {
            return Err(SignatureError::FromBytes("expected exactly 65 bytes"));
        }
        let parity =
            normalize_v(bytes[64] as u64).ok_or(SignatureError::InvalidParity(bytes[64] as u64))?;
        Ok(Self::from_bytes_and_parity(&bytes[..64], parity))
    }
}

impl FromStr for PrimitiveSignature {
    type Err = SignatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;
        Self::try_from(&bytes[..])
    }
}

impl From<&PrimitiveSignature> for [u8; 65] {
    #[inline]
    fn from(value: &PrimitiveSignature) -> [u8; 65] {
        value.as_bytes()
    }
}

impl From<PrimitiveSignature> for [u8; 65] {
    #[inline]
    fn from(value: PrimitiveSignature) -> [u8; 65] {
        value.as_bytes()
    }
}

impl From<&PrimitiveSignature> for Vec<u8> {
    #[inline]
    fn from(value: &PrimitiveSignature) -> Self {
        value.as_bytes().to_vec()
    }
}

impl From<PrimitiveSignature> for Vec<u8> {
    #[inline]
    fn from(value: PrimitiveSignature) -> Self {
        value.as_bytes().to_vec()
    }
}

#[cfg(feature = "k256")]
impl From<(k256::ecdsa::Signature, k256::ecdsa::RecoveryId)> for PrimitiveSignature {
    fn from(value: (k256::ecdsa::Signature, k256::ecdsa::RecoveryId)) -> Self {
        Self::from_signature_and_parity(value.0, value.1.is_y_odd())
    }
}

#[cfg(feature = "k256")]
impl TryFrom<PrimitiveSignature> for k256::ecdsa::Signature {
    type Error = k256::ecdsa::Error;

    fn try_from(value: PrimitiveSignature) -> Result<Self, Self::Error> {
        value.to_k256()
    }
}

#[cfg(feature = "rlp")]
impl PrimitiveSignature {
    /// Decode an RLP-encoded VRS signature. Accepts `decode_parity` closure which allows to
    /// customize parity decoding and possibly extract additional data from it (e.g chain_id for
    /// legacy signature).
    pub fn decode_rlp_vrs(
        buf: &mut &[u8],
        decode_parity: impl FnOnce(&mut &[u8]) -> alloy_rlp::Result<bool>,
    ) -> Result<Self, alloy_rlp::Error> {
        use alloy_rlp::Decodable;

        let parity = decode_parity(buf)?;
        let r = Decodable::decode(buf)?;
        let s = Decodable::decode(buf)?;

        Ok(Self::new(r, s, parity))
    }
}

impl PrimitiveSignature {
    #[doc(hidden)]
    pub fn test_signature() -> Self {
        Self::from_scalars_and_parity(
            b256!("0x840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
            b256!("0x25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
            false,
        )
    }

    /// Instantiate a new signature from `r`, `s`, and `v` values.
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(r: U256, s: U256, v: bool) -> Self {
        Self { r, s, y_parity: v }
    }

    /// Returns the inner ECDSA signature.
    #[cfg(feature = "k256")]
    #[deprecated(note = "use `Signature::to_k256` instead")]
    #[inline]
    pub fn into_inner(self) -> k256::ecdsa::Signature {
        self.try_into().expect("signature conversion failed")
    }

    /// Returns the inner ECDSA signature.
    #[cfg(feature = "k256")]
    #[inline]
    pub fn to_k256(self) -> Result<k256::ecdsa::Signature, k256::ecdsa::Error> {
        k256::ecdsa::Signature::from_scalars(self.r.to_be_bytes(), self.s.to_be_bytes())
    }

    /// Instantiate from a signature and recovery id
    #[cfg(feature = "k256")]
    pub fn from_signature_and_parity(sig: k256::ecdsa::Signature, v: bool) -> Self {
        let r = U256::from_be_slice(sig.r().to_bytes().as_ref());
        let s = U256::from_be_slice(sig.s().to_bytes().as_ref());
        Self { y_parity: v, r, s }
    }

    /// Creates a [`PrimitiveSignature`] from the serialized `r` and `s` scalar values, which
    /// comprise the ECDSA signature, alongside a `v` value, used to determine the recovery ID.
    #[inline]
    pub fn from_scalars_and_parity(r: crate::B256, s: crate::B256, parity: bool) -> Self {
        Self::new(U256::from_be_slice(r.as_ref()), U256::from_be_slice(s.as_ref()), parity)
    }

    /// Normalizes the signature into "low S" form as described in
    /// [BIP 0062: Dealing with Malleability][1].
    ///
    /// [1]: https://github.com/bitcoin/bips/blob/master/bip-0062.mediawiki
    #[inline]
    pub fn normalize_s(&self) -> Option<Self> {
        let s = self.s();

        if s > SECP256K1N_ORDER >> 1 {
            Some(Self { y_parity: !self.y_parity, r: self.r, s: SECP256K1N_ORDER - s })
        } else {
            None
        }
    }

    /// Returns the recovery ID.
    #[cfg(feature = "k256")]
    #[inline]
    pub const fn recid(&self) -> k256::ecdsa::RecoveryId {
        k256::ecdsa::RecoveryId::new(self.y_parity, false)
    }

    #[cfg(feature = "k256")]
    #[doc(hidden)]
    #[deprecated(note = "use `Signature::recid` instead")]
    pub const fn recovery_id(&self) -> k256::ecdsa::RecoveryId {
        self.recid()
    }

    /// Recovers an [`Address`] from this signature and the given message by first prefixing and
    /// hashing the message according to [EIP-191](crate::eip191_hash_message).
    ///
    /// [`Address`]: crate::Address
    #[cfg(feature = "k256")]
    #[inline]
    pub fn recover_address_from_msg<T: AsRef<[u8]>>(
        &self,
        msg: T,
    ) -> Result<crate::Address, SignatureError> {
        self.recover_from_msg(msg).map(|vk| crate::Address::from_public_key(&vk))
    }

    /// Recovers an [`Address`] from this signature and the given prehashed message.
    ///
    /// [`Address`]: crate::Address
    #[cfg(feature = "k256")]
    #[inline]
    pub fn recover_address_from_prehash(
        &self,
        prehash: &crate::B256,
    ) -> Result<crate::Address, SignatureError> {
        self.recover_from_prehash(prehash).map(|vk| crate::Address::from_public_key(&vk))
    }

    /// Recovers a [`VerifyingKey`] from this signature and the given message by first prefixing and
    /// hashing the message according to [EIP-191](crate::eip191_hash_message).
    ///
    /// [`VerifyingKey`]: k256::ecdsa::VerifyingKey
    #[cfg(feature = "k256")]
    #[inline]
    pub fn recover_from_msg<T: AsRef<[u8]>>(
        &self,
        msg: T,
    ) -> Result<k256::ecdsa::VerifyingKey, SignatureError> {
        self.recover_from_prehash(&crate::eip191_hash_message(msg))
    }

    /// Recovers a [`VerifyingKey`] from this signature and the given prehashed message.
    ///
    /// [`VerifyingKey`]: k256::ecdsa::VerifyingKey
    #[cfg(feature = "k256")]
    #[inline]
    pub fn recover_from_prehash(
        &self,
        prehash: &crate::B256,
    ) -> Result<k256::ecdsa::VerifyingKey, SignatureError> {
        let this = self.normalize_s().unwrap_or(*self);
        k256::ecdsa::VerifyingKey::recover_from_prehash(
            prehash.as_slice(),
            &this.to_k256()?,
            this.recid(),
        )
        .map_err(Into::into)
    }

    /// Parses a signature from a byte slice, with a v value
    ///
    /// # Panics
    ///
    /// If the slice is not at least 64 bytes long.
    #[inline]
    pub fn from_bytes_and_parity(bytes: &[u8], parity: bool) -> Self {
        let r = U256::from_be_slice(&bytes[..32]);
        let s = U256::from_be_slice(&bytes[32..64]);
        Self::new(r, s, parity)
    }

    /// Returns the `r` component of this signature.
    #[inline]
    pub const fn r(&self) -> U256 {
        self.r
    }

    /// Returns the `s` component of this signature.
    #[inline]
    pub const fn s(&self) -> U256 {
        self.s
    }

    /// Returns the recovery ID as a `bool`.
    #[inline]
    pub const fn v(&self) -> bool {
        self.y_parity
    }

    /// Returns the byte-array representation of this signature.
    ///
    /// The first 32 bytes are the `r` value, the second 32 bytes the `s` value
    /// and the final byte is the `v` value in 'Electrum' notation.
    #[inline]
    pub fn as_bytes(&self) -> [u8; 65] {
        let mut sig = [0u8; 65];
        sig[..32].copy_from_slice(&self.r.to_be_bytes::<32>());
        sig[32..64].copy_from_slice(&self.s.to_be_bytes::<32>());
        sig[64] = 27 + self.y_parity as u8;
        sig
    }

    /// Sets the recovery ID by normalizing a `v` value.
    #[inline]
    pub const fn with_parity(self, v: bool) -> Self {
        Self { y_parity: v, r: self.r, s: self.s }
    }

    /// Length of RLP RS field encoding
    #[cfg(feature = "rlp")]
    pub fn rlp_rs_len(&self) -> usize {
        alloy_rlp::Encodable::length(&self.r) + alloy_rlp::Encodable::length(&self.s)
    }

    /// Write R and S to an RLP buffer in progress.
    #[cfg(feature = "rlp")]
    pub fn write_rlp_rs(&self, out: &mut dyn alloy_rlp::BufMut) {
        alloy_rlp::Encodable::encode(&self.r, out);
        alloy_rlp::Encodable::encode(&self.s, out);
    }

    /// Write the VRS to the output.
    #[cfg(feature = "rlp")]
    pub fn write_rlp_vrs(&self, out: &mut dyn alloy_rlp::BufMut, v: impl alloy_rlp::Encodable) {
        v.encode(out);
        self.write_rlp_rs(out);
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for PrimitiveSignature {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new(u.arbitrary()?, u.arbitrary()?, u.arbitrary()?))
    }
}

#[cfg(feature = "arbitrary")]
impl proptest::arbitrary::Arbitrary for PrimitiveSignature {
    type Parameters = ();
    type Strategy = proptest::strategy::Map<
        <(U256, U256, bool) as proptest::arbitrary::Arbitrary>::Strategy,
        fn((U256, U256, bool)) -> Self,
    >;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Strategy;
        proptest::arbitrary::any::<(U256, U256, bool)>()
            .prop_map(|(r, s, parity)| Self::new(r, s, parity))
    }
}

#[cfg(feature = "serde")]
mod signature_serde {
    use serde::{Deserialize, Deserializer, Serialize};

    use crate::{normalize_v, U256, U64};

    use super::PrimitiveSignature;

    #[derive(Serialize, Deserialize)]
    struct HumanReadableRepr {
        r: U256,
        s: U256,
        #[serde(rename = "yParity")]
        y_parity: Option<U64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        v: Option<U64>,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(transparent)]
    struct NonHumanReadableRepr((U256, U256, U64));

    impl Serialize for PrimitiveSignature {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            // if the serializer is human readable, serialize as a map, otherwise as a tuple
            if serializer.is_human_readable() {
                HumanReadableRepr {
                    y_parity: Some(U64::from(self.y_parity as u64)),
                    v: Some(U64::from(self.y_parity as u64)),
                    r: self.r,
                    s: self.s,
                }
                .serialize(serializer)
            } else {
                NonHumanReadableRepr((self.r, self.s, U64::from(self.y_parity as u64)))
                    .serialize(serializer)
            }
        }
    }

    impl<'de> Deserialize<'de> for PrimitiveSignature {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let (y_parity, v, r, s) = if deserializer.is_human_readable() {
                let HumanReadableRepr { y_parity, v, r, s } = <_>::deserialize(deserializer)?;
                (y_parity, v, r, s)
            } else {
                let NonHumanReadableRepr((r, s, y_parity)) = <_>::deserialize(deserializer)?;
                (Some(y_parity), None, r, s)
            };

            // Attempt to extract `y_parity` bit from either `yParity` key or `v` value.
            let y_parity = if let Some(y_parity) = y_parity {
                if y_parity > U64::from(1) {
                    return Err(serde::de::Error::custom("invalid yParity"));
                }

                y_parity == U64::from(1)
            } else if let Some(v) = v {
                normalize_v(v.to()).ok_or(serde::de::Error::custom("invalid v"))?
            } else {
                return Err(serde::de::Error::custom("missing `yParity` or `v`"));
            };

            Ok(Self::new(r, s, y_parity))
        }
    }
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use super::*;
    use crate::Bytes;
    use core::str::FromStr;
    use hex::FromHex;

    #[cfg(feature = "rlp")]
    use alloy_rlp::{Decodable, Encodable};

    #[test]
    #[cfg(feature = "k256")]
    fn can_recover_tx_sender_not_normalized() {
        let sig = PrimitiveSignature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap();
        let hash = b256!("0x5eb4f5a33c621f32a8622d5f943b6b102994dfe4e5aebbefe69bb1b2aa0fc93e");
        let expected = address!("0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e");
        assert_eq!(sig.recover_address_from_prehash(&hash).unwrap(), expected);
    }

    #[test]
    #[cfg(feature = "k256")]
    fn recover_web3_signature() {
        // test vector taken from:
        // https://web3js.readthedocs.io/en/v1.2.2/web3-eth-accounts.html#sign
        let sig = PrimitiveSignature::from_str(
            "b91467e570a6466aa9e9876cbcd013baba02900b8979d43fe208a4a4f339f5fd6007e74cd82e037b800186422fc2da167c747ef045e5d18a5f5d4300f8e1a0291c"
        ).expect("could not parse signature");
        let expected = address!("0x2c7536E3605D9C16a7a3D7b1898e529396a65c23");
        assert_eq!(sig.recover_address_from_msg("Some data").unwrap(), expected);
    }

    #[test]
    fn signature_from_str() {
        let s1 = PrimitiveSignature::from_str(
            "0xaa231fbe0ed2b5418e6ba7c19bee2522852955ec50996c02a2fe3e71d30ddaf1645baf4823fea7cb4fcc7150842493847cfb6a6d63ab93e8ee928ee3f61f503500"
        ).expect("could not parse 0x-prefixed signature");

        let s2 = PrimitiveSignature::from_str(
            "aa231fbe0ed2b5418e6ba7c19bee2522852955ec50996c02a2fe3e71d30ddaf1645baf4823fea7cb4fcc7150842493847cfb6a6d63ab93e8ee928ee3f61f503500"
        ).expect("could not parse non-prefixed signature");

        assert_eq!(s1, s2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserialize_with_parity() {
        let raw_signature_with_y_parity = serde_json::json!({
            "r": "0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0",
            "s": "0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05",
            "v": "0x1",
            "yParity": "0x1"
        });

        let signature: PrimitiveSignature =
            serde_json::from_value(raw_signature_with_y_parity).unwrap();

        let expected = PrimitiveSignature::new(
            U256::from_str("0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0")
                .unwrap(),
            U256::from_str("0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05")
                .unwrap(),
            true,
        );

        assert_eq!(signature, expected);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serialize_both_parity() {
        // this test should be removed if the struct moves to an enum based on tx type
        let signature = PrimitiveSignature::new(
            U256::from_str("0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0")
                .unwrap(),
            U256::from_str("0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05")
                .unwrap(),
            true,
        );

        let serialized = serde_json::to_string(&signature).unwrap();
        assert_eq!(
            serialized,
            r#"{"r":"0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0","s":"0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05","yParity":"0x1","v":"0x1"}"#
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serialize_v_only() {
        // this test should be removed if the struct moves to an enum based on tx type
        let signature = PrimitiveSignature::new(
            U256::from_str("0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0")
                .unwrap(),
            U256::from_str("0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05")
                .unwrap(),
            true,
        );

        let expected = r#"{"r":"0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0","s":"0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05","yParity":"0x1","v":"0x1"}"#;

        let serialized = serde_json::to_string(&signature).unwrap();
        assert_eq!(serialized, expected);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_bincode_roundtrip() {
        let signature = PrimitiveSignature::new(
            U256::from_str("0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0")
                .unwrap(),
            U256::from_str("0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05")
                .unwrap(),
            true,
        );

        let bin = bincode::serialize(&signature).unwrap();
        assert_eq!(bincode::deserialize::<PrimitiveSignature>(&bin).unwrap(), signature);
    }

    #[cfg(feature = "rlp")]
    #[test]
    fn signature_rlp_encode() {
        // Given a Signature instance
        let sig = PrimitiveSignature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap();

        // Initialize an empty buffer
        let mut buf = vec![];

        // Encode the Signature into the buffer
        sig.write_rlp_vrs(&mut buf, sig.v());

        // Define the expected hex-encoded string
        let expected = "80a048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804";

        // Assert that the encoded buffer matches the expected hex-encoded string
        assert_eq!(hex::encode(&buf), expected);
    }

    #[cfg(feature = "rlp")]
    #[test]
    fn signature_rlp_length() {
        // Given a Signature instance
        let sig = PrimitiveSignature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap();

        // Assert that the length of the Signature matches the expected length
        assert_eq!(sig.rlp_rs_len() + sig.v().length(), 67);
    }

    #[cfg(feature = "rlp")]
    #[test]
    fn test_rlp_vrs_len() {
        let signature = PrimitiveSignature::test_signature();
        assert_eq!(67, signature.rlp_rs_len() + 1);
    }

    #[cfg(feature = "rlp")]
    #[test]
    fn test_encode_and_decode() {
        let signature = PrimitiveSignature::test_signature();

        let mut encoded = Vec::new();
        signature.write_rlp_vrs(&mut encoded, signature.v());
        assert_eq!(encoded.len(), signature.rlp_rs_len() + signature.v().length());
        let decoded = PrimitiveSignature::decode_rlp_vrs(&mut &*encoded, bool::decode).unwrap();
        assert_eq!(signature, decoded);
    }

    #[test]
    fn test_as_bytes() {
        let signature = PrimitiveSignature::new(
            U256::from_str(
                "18515461264373351373200002665853028612451056578545711640558177340181847433846",
            )
            .unwrap(),
            U256::from_str(
                "46948507304638947509940763649030358759909902576025900602547168820602576006531",
            )
            .unwrap(),
            false,
        );

        let expected = Bytes::from_hex("0x28ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa63627667cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d831b").unwrap();
        assert_eq!(signature.as_bytes(), **expected);
    }
}
