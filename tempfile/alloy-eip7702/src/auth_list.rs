use core::ops::Deref;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use alloy_primitives::{keccak256, Address, PrimitiveSignature, SignatureError, B256, U256, U8};
use alloy_rlp::{
    length_of_length, BufMut, Decodable, Encodable, Header, Result as RlpResult, RlpDecodable,
    RlpEncodable,
};
use core::hash::{Hash, Hasher};

/// Represents the outcome of an attempt to recover the authority from an authorization.
/// It can either be valid (containing an [`Address`]) or invalid (indicating recovery failure).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RecoveredAuthority {
    /// Indicates a successfully recovered authority address.
    Valid(Address),
    /// Indicates a failed recovery attempt where no valid address could be recovered.
    Invalid,
}

impl RecoveredAuthority {
    /// Returns an optional address if valid.
    pub const fn address(&self) -> Option<Address> {
        match *self {
            Self::Valid(address) => Some(address),
            Self::Invalid => None,
        }
    }

    /// Returns true if the authority is valid.
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }

    /// Returns true if the authority is invalid.
    pub const fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid)
    }
}

/// An unsigned EIP-7702 authorization.
#[derive(Debug, Clone, Hash, RlpEncodable, RlpDecodable, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Authorization {
    /// The chain ID of the authorization.
    pub chain_id: U256,
    /// The address of the authorization.
    pub address: Address,
    /// The nonce for the authorization.
    #[cfg_attr(feature = "serde", serde(with = "quantity"))]
    pub nonce: u64,
}

impl Authorization {
    /// Get the `chain_id` for the authorization.
    ///
    /// # Note
    ///
    /// Implementers should check that this matches the current `chain_id` *or* is 0.
    pub const fn chain_id(&self) -> &U256 {
        &self.chain_id
    }

    /// Get the `address` for the authorization.
    pub const fn address(&self) -> &Address {
        &self.address
    }

    /// Get the `nonce` for the authorization.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Computes the signature hash used to sign the authorization, or recover the authority from a
    /// signed authorization list item.
    ///
    /// The signature hash is `keccak(MAGIC || rlp([chain_id, address, nonce]))`
    #[inline]
    pub fn signature_hash(&self) -> B256 {
        use super::constants::MAGIC;

        let mut buf = Vec::new();
        buf.put_u8(MAGIC);
        self.encode(&mut buf);

        keccak256(buf)
    }

    /// Convert to a signed authorization by adding a signature.
    pub fn into_signed(self, signature: PrimitiveSignature) -> SignedAuthorization {
        SignedAuthorization {
            inner: self,
            r: signature.r(),
            s: signature.s(),
            y_parity: U8::from(signature.v()),
        }
    }
}

/// A signed EIP-7702 authorization.
#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SignedAuthorization {
    /// Inner authorization.
    #[cfg_attr(feature = "serde", serde(flatten))]
    inner: Authorization,
    /// Signature parity value. We allow any [`U8`] here, however, the only valid values are `0`
    /// and `1` and anything else will result in error during recovery.
    #[cfg_attr(feature = "serde", serde(rename = "yParity", alias = "v"))]
    y_parity: U8,
    /// Signature `r` value.
    r: U256,
    /// Signature `s` value.
    s: U256,
}

impl SignedAuthorization {
    /// Creates a new signed authorization from raw signature values.
    pub fn new_unchecked(inner: Authorization, y_parity: u8, r: U256, s: U256) -> Self {
        Self { inner, y_parity: U8::from(y_parity), r, s }
    }

    /// Gets the `signature` for the authorization. Returns [`SignatureError`] if signature could
    /// not be constructed from vrs values.
    ///
    /// Note that this signature might still be invalid for recovery as it might have `s` value
    /// greater than [secp256k1n/2](crate::constants::SECP256K1N_HALF).
    pub fn signature(&self) -> Result<PrimitiveSignature, SignatureError> {
        if self.y_parity() <= 1 {
            Ok(PrimitiveSignature::new(self.r, self.s, self.y_parity() == 1))
        } else {
            Err(SignatureError::InvalidParity(self.y_parity() as u64))
        }
    }

    /// Returns the inner [`Authorization`].
    pub const fn strip_signature(self) -> Authorization {
        self.inner
    }

    /// Returns the inner [`Authorization`].
    pub const fn inner(&self) -> &Authorization {
        &self.inner
    }

    /// Returns the signature parity value.
    pub fn y_parity(&self) -> u8 {
        self.y_parity.to()
    }

    /// Returns the signature `r` value.
    pub const fn r(&self) -> U256 {
        self.r
    }

    /// Returns the signature `s` value.
    pub const fn s(&self) -> U256 {
        self.s
    }

    /// Decodes the transaction from RLP bytes, including the signature.
    fn decode_fields(buf: &mut &[u8]) -> RlpResult<Self> {
        Ok(Self {
            inner: Authorization {
                chain_id: Decodable::decode(buf)?,
                address: Decodable::decode(buf)?,
                nonce: Decodable::decode(buf)?,
            },
            y_parity: Decodable::decode(buf)?,
            r: Decodable::decode(buf)?,
            s: Decodable::decode(buf)?,
        })
    }

    /// Outputs the length of the transaction's fields, without a RLP header.
    fn fields_len(&self) -> usize {
        self.inner.chain_id.length()
            + self.inner.address.length()
            + self.inner.nonce.length()
            + self.y_parity.length()
            + self.r.length()
            + self.s.length()
    }
}

impl Hash for SignedAuthorization {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
        self.r.hash(state);
        self.s.hash(state);
        self.y_parity.hash(state);
    }
}

impl Decodable for SignedAuthorization {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }
        let started_len = buf.len();

        let this = Self::decode_fields(buf)?;

        let consumed = started_len - buf.len();
        if consumed != header.payload_length {
            return Err(alloy_rlp::Error::ListLengthMismatch {
                expected: header.payload_length,
                got: consumed,
            });
        }

        Ok(this)
    }
}

impl Encodable for SignedAuthorization {
    fn encode(&self, buf: &mut dyn BufMut) {
        Header { list: true, payload_length: self.fields_len() }.encode(buf);
        self.inner.chain_id.encode(buf);
        self.inner.address.encode(buf);
        self.inner.nonce.encode(buf);
        self.y_parity.encode(buf);
        self.r.encode(buf);
        self.s.encode(buf);
    }

    fn length(&self) -> usize {
        let len = self.fields_len();
        len + length_of_length(len)
    }
}

#[cfg(feature = "k256")]
impl SignedAuthorization {
    /// Recover the authority for the authorization.
    ///
    /// # Note
    ///
    /// Implementers should check that the authority has no code.
    pub fn recover_authority(&self) -> Result<Address, crate::error::Eip7702Error> {
        let signature = self.signature()?;

        if signature.s() > crate::constants::SECP256K1N_HALF {
            return Err(crate::error::Eip7702Error::InvalidSValue(signature.s()));
        }

        Ok(signature.recover_address_from_prehash(&self.inner.signature_hash())?)
    }

    /// Recover the authority and transform the signed authorization into a
    /// [`RecoveredAuthorization`].
    pub fn into_recovered(self) -> RecoveredAuthorization {
        let authority_result = self.recover_authority();
        let authority =
            authority_result.map_or(RecoveredAuthority::Invalid, RecoveredAuthority::Valid);

        RecoveredAuthorization { inner: self.inner, authority }
    }
}

impl Deref for SignedAuthorization {
    type Target = Authorization;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(all(any(test, feature = "arbitrary"), feature = "k256"))]
impl<'a> arbitrary::Arbitrary<'a> for SignedAuthorization {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        use k256::{
            ecdsa::{signature::hazmat::PrehashSigner, SigningKey},
            NonZeroScalar,
        };
        use rand::{rngs::StdRng, SeedableRng};

        let rng_seed = u.arbitrary::<[u8; 32]>()?;
        let mut rand_gen = StdRng::from_seed(rng_seed);
        let signing_key: SigningKey = NonZeroScalar::random(&mut rand_gen).into();

        let inner = u.arbitrary::<Authorization>()?;
        let signature_hash = inner.signature_hash();

        let (recoverable_sig, recovery_id) =
            signing_key.sign_prehash(signature_hash.as_ref()).unwrap();
        let signature =
            PrimitiveSignature::from_signature_and_parity(recoverable_sig, recovery_id.is_y_odd());

        Ok(inner.into_signed(signature))
    }
}

/// A recovered authorization.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RecoveredAuthorization {
    #[cfg_attr(feature = "serde", serde(flatten))]
    inner: Authorization,
    /// The result of the authority recovery process, which can either be a valid address or
    /// indicate a failure.
    authority: RecoveredAuthority,
}

impl RecoveredAuthorization {
    /// Instantiate without performing recovery. This should be used carefully.
    pub const fn new_unchecked(inner: Authorization, authority: RecoveredAuthority) -> Self {
        Self { inner, authority }
    }

    /// Returns an optional address based on the current state of the authority.
    pub const fn authority(&self) -> Option<Address> {
        self.authority.address()
    }

    /// Splits the authorization into parts.
    pub const fn into_parts(self) -> (Authorization, RecoveredAuthority) {
        (self.inner, self.authority)
    }
}

#[cfg(feature = "k256")]
impl From<SignedAuthorization> for RecoveredAuthority {
    fn from(value: SignedAuthorization) -> Self {
        value.into_recovered().authority
    }
}

#[cfg(feature = "k256")]
impl From<SignedAuthorization> for RecoveredAuthorization {
    fn from(value: SignedAuthorization) -> Self {
        value.into_recovered()
    }
}
impl Deref for RecoveredAuthorization {
    type Target = Authorization;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(feature = "serde")]
mod quantity {
    use alloy_primitives::U64;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serializes a primitive number as a "quantity" hex string.
    pub(crate) fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        U64::from(*value).serialize(serializer)
    }

    /// Deserializes a primitive number from a "quantity" hex string.
    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        U64::deserialize(deserializer).map(|value| value.to())
    }
}

/// Bincode-compatible [`SignedAuthorization`] serde implementation.
#[cfg(all(feature = "serde", feature = "serde-bincode-compat"))]
pub(super) mod serde_bincode_compat {
    use crate::Authorization;
    use alloc::borrow::Cow;
    use alloy_primitives::{U256, U8};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_with::{DeserializeAs, SerializeAs};

    /// Bincode-compatible [`super::SignedAuthorization`] serde implementation.
    ///
    /// Intended to use with the [`serde_with::serde_as`] macro in the following way:
    /// ```rust
    /// use alloy_eip7702::{serde_bincode_compat, SignedAuthorization};
    /// use serde::{Deserialize, Serialize};
    /// use serde_with::serde_as;
    ///
    /// #[serde_as]
    /// #[derive(Serialize, Deserialize)]
    /// struct Data {
    ///     #[serde_as(as = "serde_bincode_compat::SignedAuthorization")]
    ///     authorization: SignedAuthorization,
    /// }
    /// ```
    #[derive(Debug, Serialize, Deserialize)]
    pub struct SignedAuthorization<'a> {
        inner: Cow<'a, Authorization>,
        #[serde(rename = "yParity")]
        y_parity: U8,
        r: U256,
        s: U256,
    }

    impl<'a> From<&'a super::SignedAuthorization> for SignedAuthorization<'a> {
        fn from(value: &'a super::SignedAuthorization) -> Self {
            Self {
                inner: Cow::Borrowed(&value.inner),
                y_parity: value.y_parity,
                r: value.r,
                s: value.s,
            }
        }
    }

    impl<'a> From<SignedAuthorization<'a>> for super::SignedAuthorization {
        fn from(value: SignedAuthorization<'a>) -> Self {
            Self {
                inner: value.inner.into_owned(),
                y_parity: value.y_parity,
                r: value.r,
                s: value.s,
            }
        }
    }

    impl SerializeAs<super::SignedAuthorization> for SignedAuthorization<'_> {
        fn serialize_as<S>(
            source: &super::SignedAuthorization,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            SignedAuthorization::from(source).serialize(serializer)
        }
    }

    impl<'de> DeserializeAs<'de, super::SignedAuthorization> for SignedAuthorization<'de> {
        fn deserialize_as<D>(deserializer: D) -> Result<super::SignedAuthorization, D::Error>
        where
            D: Deserializer<'de>,
        {
            SignedAuthorization::deserialize(deserializer).map(Into::into)
        }
    }

    #[cfg(all(test, feature = "k256"))]
    mod tests {
        use arbitrary::Arbitrary;
        use rand::Rng;
        use serde::{Deserialize, Serialize};
        use serde_with::serde_as;

        use super::super::{serde_bincode_compat, SignedAuthorization};

        #[test]
        fn test_signed_authorization_bincode_roundtrip() {
            #[serde_as]
            #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
            struct Data {
                #[serde_as(as = "serde_bincode_compat::SignedAuthorization")]
                authorization: SignedAuthorization,
            }

            let mut bytes = [0u8; 1024];
            rand::thread_rng().fill(bytes.as_mut_slice());
            let data = Data {
                authorization: SignedAuthorization::arbitrary(&mut arbitrary::Unstructured::new(
                    &bytes,
                ))
                .unwrap(),
            };

            let encoded = bincode::serialize(&data).unwrap();
            let decoded: Data = bincode::deserialize(&encoded).unwrap();
            assert_eq!(decoded, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;
    use core::str::FromStr;

    fn test_encode_decode_roundtrip(auth: Authorization) {
        let mut buf = Vec::new();
        auth.encode(&mut buf);
        let decoded = Authorization::decode(&mut buf.as_ref()).unwrap();
        assert_eq!(buf.len(), auth.length());
        assert_eq!(decoded, auth);
    }

    #[test]
    fn test_encode_decode_auth() {
        // fully filled
        test_encode_decode_roundtrip(Authorization {
            chain_id: U256::from(1),
            address: Address::left_padding_from(&[6]),
            nonce: 1,
        });
    }

    #[test]
    fn test_encode_decode_signed_auth() {
        let auth = Authorization {
            chain_id: U256::from(1),
            address: Address::left_padding_from(&[6]),
            nonce: 1,
        };

        let auth = auth.into_signed(PrimitiveSignature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap());
        let mut buf = Vec::new();
        auth.encode(&mut buf);

        let expected = "f85a019400000000000000000000000000000000000000060180a048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804";
        assert_eq!(hex::encode(&buf), expected);

        let decoded = SignedAuthorization::decode(&mut buf.as_ref()).unwrap();
        assert_eq!(buf.len(), auth.length());
        assert_eq!(decoded, auth);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_auth_json() {
        let sig = r#"{"r":"0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0","s":"0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05","yParity":"0x1"}"#;
        let auth = Authorization {
            chain_id: U256::from(1),
            address: Address::left_padding_from(&[6]),
            nonce: 1,
        }
        .into_signed(serde_json::from_str(sig).unwrap());
        let val = serde_json::to_string(&auth).unwrap();
        let s = r#"{"chainId":"0x1","address":"0x0000000000000000000000000000000000000006","nonce":"0x1","yParity":"0x1","r":"0xc569c92f176a3be1a6352dd5005bfc751dcb32f57623dd2a23693e64bf4447b0","s":"0x1a891b566d369e79b7a66eecab1e008831e22daa15f91a0a0cf4f9f28f47ee05"}"#;
        assert_eq!(val, s);
    }

    #[cfg(all(feature = "arbitrary", feature = "k256"))]
    #[test]
    fn test_arbitrary_auth() {
        use arbitrary::Arbitrary;
        let mut unstructured = arbitrary::Unstructured::new(b"unstructured auth");
        // try this multiple times
        let _auth = SignedAuthorization::arbitrary(&mut unstructured).unwrap();
        let _auth = SignedAuthorization::arbitrary(&mut unstructured).unwrap();
        let _auth = SignedAuthorization::arbitrary(&mut unstructured).unwrap();
        let _auth = SignedAuthorization::arbitrary(&mut unstructured).unwrap();
    }
}
