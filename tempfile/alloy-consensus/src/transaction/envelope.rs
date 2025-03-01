use crate::{
    transaction::{
        eip4844::{TxEip4844, TxEip4844Variant, TxEip4844WithSidecar},
        RlpEcdsaTx,
    },
    Signed, Transaction, TxEip1559, TxEip2930, TxEip7702, TxLegacy,
};
use alloy_eips::{
    eip2718::{Decodable2718, Eip2718Error, Eip2718Result, Encodable2718},
    eip2930::AccessList,
    Typed2718,
};
use alloy_primitives::{
    Bytes, ChainId, PrimitiveSignature as Signature, TxKind, B256, U256, U64, U8,
};
use alloy_rlp::{Decodable, Encodable};
use core::fmt;

/// Ethereum `TransactionType` flags as specified in EIPs [2718], [1559], [2930],
/// [4844], and [7702].
///
/// [2718]: https://eips.ethereum.org/EIPS/eip-2718
/// [1559]: https://eips.ethereum.org/EIPS/eip-1559
/// [2930]: https://eips.ethereum.org/EIPS/eip-2930
/// [4844]: https://eips.ethereum.org/EIPS/eip-4844
/// [7702]: https://eips.ethereum.org/EIPS/eip-7702
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "U8", try_from = "U64"))]
#[doc(alias = "TransactionType")]
pub enum TxType {
    /// Legacy transaction type.
    #[default]
    Legacy = 0,
    /// EIP-2930 transaction type.
    Eip2930 = 1,
    /// EIP-1559 transaction type.
    Eip1559 = 2,
    /// EIP-4844 transaction type.
    Eip4844 = 3,
    /// EIP-7702 transaction type.
    Eip7702 = 4,
}

impl From<TxType> for u8 {
    fn from(value: TxType) -> Self {
        value as Self
    }
}

impl From<TxType> for U8 {
    fn from(tx_type: TxType) -> Self {
        Self::from(u8::from(tx_type))
    }
}

impl fmt::Display for TxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => write!(f, "Legacy"),
            Self::Eip2930 => write!(f, "EIP-2930"),
            Self::Eip1559 => write!(f, "EIP-1559"),
            Self::Eip4844 => write!(f, "EIP-4844"),
            Self::Eip7702 => write!(f, "EIP-7702"),
        }
    }
}

impl PartialEq<u8> for TxType {
    fn eq(&self, other: &u8) -> bool {
        (*self as u8) == *other
    }
}

impl PartialEq<TxType> for u8 {
    fn eq(&self, other: &TxType) -> bool {
        *self == *other as Self
    }
}

#[cfg(any(test, feature = "arbitrary"))]
impl arbitrary::Arbitrary<'_> for TxType {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(u.int_in_range(0u8..=4)?.try_into().unwrap())
    }
}

impl TryFrom<u8> for TxType {
    type Error = Eip2718Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Legacy,
            1 => Self::Eip2930,
            2 => Self::Eip1559,
            3 => Self::Eip4844,
            4 => Self::Eip7702,
            _ => return Err(Eip2718Error::UnexpectedType(value)),
        })
    }
}

impl TryFrom<u64> for TxType {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let err = || "invalid tx type";
        let value: u8 = value.try_into().map_err(|_| err())?;
        Self::try_from(value).map_err(|_| err())
    }
}

impl TryFrom<U64> for TxType {
    type Error = &'static str;

    fn try_from(value: U64) -> Result<Self, Self::Error> {
        value.to::<u64>().try_into()
    }
}

impl Encodable for TxType {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        (*self as u8).encode(out);
    }

    fn length(&self) -> usize {
        1
    }
}

impl Decodable for TxType {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let ty = u8::decode(buf)?;
        Self::try_from(ty).map_err(|_| alloy_rlp::Error::Custom("invalid transaction type"))
    }
}

impl Typed2718 for TxType {
    fn ty(&self) -> u8 {
        (*self).into()
    }
}

/// The Ethereum [EIP-2718] Transaction Envelope.
///
/// # Note:
///
/// This enum distinguishes between tagged and untagged legacy transactions, as
/// the in-protocol merkle tree may commit to EITHER 0-prefixed or raw.
/// Therefore we must ensure that encoding returns the precise byte-array that
/// was decoded, preserving the presence or absence of the `TransactionType`
/// flag.
///
/// [EIP-2718]: https://eips.ethereum.org/EIPS/eip-2718
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(into = "serde_from::TaggedTxEnvelope", from = "serde_from::MaybeTaggedTxEnvelope")
)]
#[cfg_attr(all(any(test, feature = "arbitrary"), feature = "k256"), derive(arbitrary::Arbitrary))]
#[doc(alias = "TransactionEnvelope")]
pub enum TxEnvelope {
    /// An untagged [`TxLegacy`].
    Legacy(Signed<TxLegacy>),
    /// A [`TxEip2930`] tagged with type 1.
    Eip2930(Signed<TxEip2930>),
    /// A [`TxEip1559`] tagged with type 2.
    Eip1559(Signed<TxEip1559>),
    /// A TxEip4844 tagged with type 3.
    /// An EIP-4844 transaction has two network representations:
    /// 1 - The transaction itself, which is a regular RLP-encoded transaction and used to retrieve
    /// historical transactions..
    ///
    /// 2 - The transaction with a sidecar, which is the form used to
    /// send transactions to the network.
    Eip4844(Signed<TxEip4844Variant>),
    /// A [`TxEip7702`] tagged with type 4.
    Eip7702(Signed<TxEip7702>),
}

impl From<Signed<TxLegacy>> for TxEnvelope {
    fn from(v: Signed<TxLegacy>) -> Self {
        Self::Legacy(v)
    }
}

impl From<Signed<TxEip2930>> for TxEnvelope {
    fn from(v: Signed<TxEip2930>) -> Self {
        Self::Eip2930(v)
    }
}

impl From<Signed<TxEip1559>> for TxEnvelope {
    fn from(v: Signed<TxEip1559>) -> Self {
        Self::Eip1559(v)
    }
}

impl From<Signed<TxEip4844Variant>> for TxEnvelope {
    fn from(v: Signed<TxEip4844Variant>) -> Self {
        Self::Eip4844(v)
    }
}

impl From<Signed<TxEip4844>> for TxEnvelope {
    fn from(v: Signed<TxEip4844>) -> Self {
        let (tx, signature, hash) = v.into_parts();
        Self::Eip4844(Signed::new_unchecked(tx.into(), signature, hash))
    }
}

impl From<Signed<TxEip4844WithSidecar>> for TxEnvelope {
    fn from(v: Signed<TxEip4844WithSidecar>) -> Self {
        let (tx, signature, hash) = v.into_parts();
        Self::Eip4844(Signed::new_unchecked(tx.into(), signature, hash))
    }
}

impl From<Signed<TxEip7702>> for TxEnvelope {
    fn from(v: Signed<TxEip7702>) -> Self {
        Self::Eip7702(v)
    }
}

impl TxEnvelope {
    /// Returns true if the transaction is a legacy transaction.
    #[inline]
    pub const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    /// Returns true if the transaction is an EIP-2930 transaction.
    #[inline]
    pub const fn is_eip2930(&self) -> bool {
        matches!(self, Self::Eip2930(_))
    }

    /// Returns true if the transaction is an EIP-1559 transaction.
    #[inline]
    pub const fn is_eip1559(&self) -> bool {
        matches!(self, Self::Eip1559(_))
    }

    /// Returns true if the transaction is an EIP-4844 transaction.
    #[inline]
    pub const fn is_eip4844(&self) -> bool {
        matches!(self, Self::Eip4844(_))
    }

    /// Returns true if the transaction is an EIP-7702 transaction.
    #[inline]
    pub const fn is_eip7702(&self) -> bool {
        matches!(self, Self::Eip7702(_))
    }

    /// Returns true if the transaction is replay protected.
    ///
    /// All non-legacy transactions are replay protected, as the chain id is
    /// included in the transaction body. Legacy transactions are considered
    /// replay protected if the `v` value is not 27 or 28, according to the
    /// rules of [EIP-155].
    ///
    /// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
    #[inline]
    pub const fn is_replay_protected(&self) -> bool {
        match self {
            Self::Legacy(tx) => tx.tx().chain_id.is_some(),
            _ => true,
        }
    }

    /// Returns the [`TxLegacy`] variant if the transaction is a legacy transaction.
    pub const fn as_legacy(&self) -> Option<&Signed<TxLegacy>> {
        match self {
            Self::Legacy(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns the [`TxEip2930`] variant if the transaction is an EIP-2930 transaction.
    pub const fn as_eip2930(&self) -> Option<&Signed<TxEip2930>> {
        match self {
            Self::Eip2930(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns the [`TxEip1559`] variant if the transaction is an EIP-1559 transaction.
    pub const fn as_eip1559(&self) -> Option<&Signed<TxEip1559>> {
        match self {
            Self::Eip1559(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns the [`TxEip4844Variant`] variant if the transaction is an EIP-4844 transaction.
    pub const fn as_eip4844(&self) -> Option<&Signed<TxEip4844Variant>> {
        match self {
            Self::Eip4844(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns the [`TxEip7702`] variant if the transaction is an EIP-7702 transaction.
    pub const fn as_eip7702(&self) -> Option<&Signed<TxEip7702>> {
        match self {
            Self::Eip7702(tx) => Some(tx),
            _ => None,
        }
    }

    /// Recover the signer of the transaction.
    #[cfg(feature = "k256")]
    pub fn recover_signer(
        &self,
    ) -> Result<alloy_primitives::Address, alloy_primitives::SignatureError> {
        match self {
            Self::Legacy(tx) => tx.recover_signer(),
            Self::Eip2930(tx) => tx.recover_signer(),
            Self::Eip1559(tx) => tx.recover_signer(),
            Self::Eip4844(tx) => tx.recover_signer(),
            Self::Eip7702(tx) => tx.recover_signer(),
        }
    }

    /// Calculate the signing hash for the transaction.
    pub fn signature_hash(&self) -> B256 {
        match self {
            Self::Legacy(tx) => tx.signature_hash(),
            Self::Eip2930(tx) => tx.signature_hash(),
            Self::Eip1559(tx) => tx.signature_hash(),
            Self::Eip4844(tx) => tx.signature_hash(),
            Self::Eip7702(tx) => tx.signature_hash(),
        }
    }

    /// Return the reference to signature.
    pub const fn signature(&self) -> &Signature {
        match self {
            Self::Legacy(tx) => tx.signature(),
            Self::Eip2930(tx) => tx.signature(),
            Self::Eip1559(tx) => tx.signature(),
            Self::Eip4844(tx) => tx.signature(),
            Self::Eip7702(tx) => tx.signature(),
        }
    }

    /// Return the hash of the inner Signed.
    #[doc(alias = "transaction_hash")]
    pub const fn tx_hash(&self) -> &B256 {
        match self {
            Self::Legacy(tx) => tx.hash(),
            Self::Eip2930(tx) => tx.hash(),
            Self::Eip1559(tx) => tx.hash(),
            Self::Eip4844(tx) => tx.hash(),
            Self::Eip7702(tx) => tx.hash(),
        }
    }

    /// Return the [`TxType`] of the inner txn.
    #[doc(alias = "transaction_type")]
    pub const fn tx_type(&self) -> TxType {
        match self {
            Self::Legacy(_) => TxType::Legacy,
            Self::Eip2930(_) => TxType::Eip2930,
            Self::Eip1559(_) => TxType::Eip1559,
            Self::Eip4844(_) => TxType::Eip4844,
            Self::Eip7702(_) => TxType::Eip7702,
        }
    }

    /// Return the length of the inner txn, including type byte length
    pub fn eip2718_encoded_length(&self) -> usize {
        match self {
            Self::Legacy(t) => t.eip2718_encoded_length(),
            Self::Eip2930(t) => t.eip2718_encoded_length(),
            Self::Eip1559(t) => t.eip2718_encoded_length(),
            Self::Eip4844(t) => t.eip2718_encoded_length(),
            Self::Eip7702(t) => t.eip2718_encoded_length(),
        }
    }
}

impl Encodable for TxEnvelope {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.network_encode(out)
    }

    fn length(&self) -> usize {
        self.network_len()
    }
}

impl Decodable for TxEnvelope {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Ok(Self::network_decode(buf)?)
    }
}

impl Decodable2718 for TxEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Eip2718Result<Self> {
        match ty.try_into().map_err(|_| alloy_rlp::Error::Custom("unexpected tx type"))? {
            TxType::Eip2930 => Ok(TxEip2930::rlp_decode_signed(buf)?.into()),
            TxType::Eip1559 => Ok(TxEip1559::rlp_decode_signed(buf)?.into()),
            TxType::Eip4844 => Ok(TxEip4844Variant::rlp_decode_signed(buf)?.into()),
            TxType::Eip7702 => Ok(TxEip7702::rlp_decode_signed(buf)?.into()),
            TxType::Legacy => Err(Eip2718Error::UnexpectedType(0)),
        }
    }

    fn fallback_decode(buf: &mut &[u8]) -> Eip2718Result<Self> {
        TxLegacy::rlp_decode_signed(buf).map(Into::into).map_err(Into::into)
    }
}

impl Encodable2718 for TxEnvelope {
    fn encode_2718_len(&self) -> usize {
        self.eip2718_encoded_length()
    }

    fn encode_2718(&self, out: &mut dyn alloy_rlp::BufMut) {
        match self {
            // Legacy transactions have no difference between network and 2718
            Self::Legacy(tx) => tx.eip2718_encode(out),
            Self::Eip2930(tx) => {
                tx.eip2718_encode(out);
            }
            Self::Eip1559(tx) => {
                tx.eip2718_encode(out);
            }
            Self::Eip4844(tx) => {
                tx.eip2718_encode(out);
            }
            Self::Eip7702(tx) => {
                tx.eip2718_encode(out);
            }
        }
    }

    fn trie_hash(&self) -> B256 {
        match self {
            Self::Legacy(tx) => *tx.hash(),
            Self::Eip2930(tx) => *tx.hash(),
            Self::Eip1559(tx) => *tx.hash(),
            Self::Eip4844(tx) => *tx.hash(),
            Self::Eip7702(tx) => *tx.hash(),
        }
    }
}

impl Transaction for TxEnvelope {
    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        match self {
            Self::Legacy(tx) => tx.tx().chain_id(),
            Self::Eip2930(tx) => tx.tx().chain_id(),
            Self::Eip1559(tx) => tx.tx().chain_id(),
            Self::Eip4844(tx) => tx.tx().chain_id(),
            Self::Eip7702(tx) => tx.tx().chain_id(),
        }
    }

    #[inline]
    fn nonce(&self) -> u64 {
        match self {
            Self::Legacy(tx) => tx.tx().nonce(),
            Self::Eip2930(tx) => tx.tx().nonce(),
            Self::Eip1559(tx) => tx.tx().nonce(),
            Self::Eip4844(tx) => tx.tx().nonce(),
            Self::Eip7702(tx) => tx.tx().nonce(),
        }
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        match self {
            Self::Legacy(tx) => tx.tx().gas_limit(),
            Self::Eip2930(tx) => tx.tx().gas_limit(),
            Self::Eip1559(tx) => tx.tx().gas_limit(),
            Self::Eip4844(tx) => tx.tx().gas_limit(),
            Self::Eip7702(tx) => tx.tx().gas_limit(),
        }
    }

    #[inline]
    fn gas_price(&self) -> Option<u128> {
        match self {
            Self::Legacy(tx) => tx.tx().gas_price(),
            Self::Eip2930(tx) => tx.tx().gas_price(),
            Self::Eip1559(tx) => tx.tx().gas_price(),
            Self::Eip4844(tx) => tx.tx().gas_price(),
            Self::Eip7702(tx) => tx.tx().gas_price(),
        }
    }

    #[inline]
    fn max_fee_per_gas(&self) -> u128 {
        match self {
            Self::Legacy(tx) => tx.tx().max_fee_per_gas(),
            Self::Eip2930(tx) => tx.tx().max_fee_per_gas(),
            Self::Eip1559(tx) => tx.tx().max_fee_per_gas(),
            Self::Eip4844(tx) => tx.tx().max_fee_per_gas(),
            Self::Eip7702(tx) => tx.tx().max_fee_per_gas(),
        }
    }

    #[inline]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        match self {
            Self::Legacy(tx) => tx.tx().max_priority_fee_per_gas(),
            Self::Eip2930(tx) => tx.tx().max_priority_fee_per_gas(),
            Self::Eip1559(tx) => tx.tx().max_priority_fee_per_gas(),
            Self::Eip4844(tx) => tx.tx().max_priority_fee_per_gas(),
            Self::Eip7702(tx) => tx.tx().max_priority_fee_per_gas(),
        }
    }

    #[inline]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        match self {
            Self::Legacy(tx) => tx.tx().max_fee_per_blob_gas(),
            Self::Eip2930(tx) => tx.tx().max_fee_per_blob_gas(),
            Self::Eip1559(tx) => tx.tx().max_fee_per_blob_gas(),
            Self::Eip4844(tx) => tx.tx().max_fee_per_blob_gas(),
            Self::Eip7702(tx) => tx.tx().max_fee_per_blob_gas(),
        }
    }

    #[inline]
    fn priority_fee_or_price(&self) -> u128 {
        match self {
            Self::Legacy(tx) => tx.tx().priority_fee_or_price(),
            Self::Eip2930(tx) => tx.tx().priority_fee_or_price(),
            Self::Eip1559(tx) => tx.tx().priority_fee_or_price(),
            Self::Eip4844(tx) => tx.tx().priority_fee_or_price(),
            Self::Eip7702(tx) => tx.tx().priority_fee_or_price(),
        }
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        match self {
            Self::Legacy(tx) => tx.tx().effective_gas_price(base_fee),
            Self::Eip2930(tx) => tx.tx().effective_gas_price(base_fee),
            Self::Eip1559(tx) => tx.tx().effective_gas_price(base_fee),
            Self::Eip4844(tx) => tx.tx().effective_gas_price(base_fee),
            Self::Eip7702(tx) => tx.tx().effective_gas_price(base_fee),
        }
    }

    #[inline]
    fn is_dynamic_fee(&self) -> bool {
        match self {
            Self::Legacy(tx) => tx.tx().is_dynamic_fee(),
            Self::Eip2930(tx) => tx.tx().is_dynamic_fee(),
            Self::Eip1559(tx) => tx.tx().is_dynamic_fee(),
            Self::Eip4844(tx) => tx.tx().is_dynamic_fee(),
            Self::Eip7702(tx) => tx.tx().is_dynamic_fee(),
        }
    }

    #[inline]
    fn kind(&self) -> TxKind {
        match self {
            Self::Legacy(tx) => tx.tx().kind(),
            Self::Eip2930(tx) => tx.tx().kind(),
            Self::Eip1559(tx) => tx.tx().kind(),
            Self::Eip4844(tx) => tx.tx().kind(),
            Self::Eip7702(tx) => tx.tx().kind(),
        }
    }

    #[inline]
    fn is_create(&self) -> bool {
        match self {
            Self::Legacy(tx) => tx.tx().is_create(),
            Self::Eip2930(tx) => tx.tx().is_create(),
            Self::Eip1559(tx) => tx.tx().is_create(),
            Self::Eip4844(tx) => tx.tx().is_create(),
            Self::Eip7702(tx) => tx.tx().is_create(),
        }
    }

    #[inline]
    fn value(&self) -> U256 {
        match self {
            Self::Legacy(tx) => tx.tx().value(),
            Self::Eip2930(tx) => tx.tx().value(),
            Self::Eip1559(tx) => tx.tx().value(),
            Self::Eip4844(tx) => tx.tx().value(),
            Self::Eip7702(tx) => tx.tx().value(),
        }
    }

    #[inline]
    fn input(&self) -> &Bytes {
        match self {
            Self::Legacy(tx) => tx.tx().input(),
            Self::Eip2930(tx) => tx.tx().input(),
            Self::Eip1559(tx) => tx.tx().input(),
            Self::Eip4844(tx) => tx.tx().input(),
            Self::Eip7702(tx) => tx.tx().input(),
        }
    }

    #[inline]
    fn access_list(&self) -> Option<&AccessList> {
        match self {
            Self::Legacy(tx) => tx.tx().access_list(),
            Self::Eip2930(tx) => tx.tx().access_list(),
            Self::Eip1559(tx) => tx.tx().access_list(),
            Self::Eip4844(tx) => tx.tx().access_list(),
            Self::Eip7702(tx) => tx.tx().access_list(),
        }
    }

    #[inline]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        match self {
            Self::Legacy(tx) => tx.tx().blob_versioned_hashes(),
            Self::Eip2930(tx) => tx.tx().blob_versioned_hashes(),
            Self::Eip1559(tx) => tx.tx().blob_versioned_hashes(),
            Self::Eip4844(tx) => tx.tx().blob_versioned_hashes(),
            Self::Eip7702(tx) => tx.tx().blob_versioned_hashes(),
        }
    }

    fn authorization_list(&self) -> Option<&[alloy_eips::eip7702::SignedAuthorization]> {
        match self {
            Self::Legacy(tx) => tx.tx().authorization_list(),
            Self::Eip2930(tx) => tx.tx().authorization_list(),
            Self::Eip1559(tx) => tx.tx().authorization_list(),
            Self::Eip4844(tx) => tx.tx().authorization_list(),
            Self::Eip7702(tx) => tx.tx().authorization_list(),
        }
    }
}

impl Typed2718 for TxEnvelope {
    fn ty(&self) -> u8 {
        match self {
            Self::Legacy(tx) => tx.tx().ty(),
            Self::Eip2930(tx) => tx.tx().ty(),
            Self::Eip1559(tx) => tx.tx().ty(),
            Self::Eip4844(tx) => tx.tx().ty(),
            Self::Eip7702(tx) => tx.tx().ty(),
        }
    }
}

#[cfg(feature = "serde")]
mod serde_from {
    //! NB: Why do we need this?
    //!
    //! Because the tag may be missing, we need an abstraction over tagged (with
    //! type) and untagged (always legacy). This is [`MaybeTaggedTxEnvelope`].
    //!
    //! The tagged variant is [`TaggedTxEnvelope`], which always has a type tag.
    //!
    //! We serialize via [`TaggedTxEnvelope`] and deserialize via
    //! [`MaybeTaggedTxEnvelope`].
    use crate::{Signed, TxEip1559, TxEip2930, TxEip4844Variant, TxEip7702, TxEnvelope, TxLegacy};

    #[derive(Debug, serde::Deserialize)]
    pub(crate) struct UntaggedLegacy {
        #[serde(default, rename = "type", deserialize_with = "alloy_serde::reject_if_some")]
        pub _ty: Option<()>,
        #[serde(flatten, with = "crate::transaction::signed_legacy_serde")]
        pub tx: Signed<TxLegacy>,
    }

    #[derive(Debug)]
    pub(crate) enum MaybeTaggedTxEnvelope {
        Tagged(TaggedTxEnvelope),
        Untagged(UntaggedLegacy),
    }

    // Manually modified derived serde(untagged) to preserve the error of the [`TaggedTxEnvelope`]
    // attempt. Note: This use private serde API
    impl<'de> serde::Deserialize<'de> for MaybeTaggedTxEnvelope {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let content = serde::__private::de::Content::deserialize(deserializer)?;
            let deserializer =
                serde::__private::de::ContentRefDeserializer::<D::Error>::new(&content);

            let tagged_res =
                TaggedTxEnvelope::deserialize(deserializer).map(MaybeTaggedTxEnvelope::Tagged);

            if tagged_res.is_ok() {
                // return tagged if successful
                return tagged_res;
            }

            // proceed with untagged legacy
            if let Ok(val) =
                UntaggedLegacy::deserialize(deserializer).map(MaybeTaggedTxEnvelope::Untagged)
            {
                return Ok(val);
            }

            // return the original error, which is more useful than the untagged error
            //  > "data did not match any variant of untagged enum MaybeTaggedTxEnvelope"
            tagged_res
        }
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    #[serde(tag = "type")]
    pub(crate) enum TaggedTxEnvelope {
        #[serde(rename = "0x0", alias = "0x00", with = "crate::transaction::signed_legacy_serde")]
        Legacy(Signed<TxLegacy>),
        #[serde(rename = "0x1", alias = "0x01")]
        Eip2930(Signed<TxEip2930>),
        #[serde(rename = "0x2", alias = "0x02")]
        Eip1559(Signed<TxEip1559>),
        #[serde(rename = "0x3", alias = "0x03")]
        Eip4844(Signed<TxEip4844Variant>),
        #[serde(rename = "0x4", alias = "0x04")]
        Eip7702(Signed<TxEip7702>),
    }

    impl From<MaybeTaggedTxEnvelope> for TxEnvelope {
        fn from(value: MaybeTaggedTxEnvelope) -> Self {
            match value {
                MaybeTaggedTxEnvelope::Tagged(tagged) => tagged.into(),
                MaybeTaggedTxEnvelope::Untagged(UntaggedLegacy { tx, .. }) => Self::Legacy(tx),
            }
        }
    }

    impl From<TaggedTxEnvelope> for TxEnvelope {
        fn from(value: TaggedTxEnvelope) -> Self {
            match value {
                TaggedTxEnvelope::Legacy(signed) => Self::Legacy(signed),
                TaggedTxEnvelope::Eip2930(signed) => Self::Eip2930(signed),
                TaggedTxEnvelope::Eip1559(signed) => Self::Eip1559(signed),
                TaggedTxEnvelope::Eip4844(signed) => Self::Eip4844(signed),
                TaggedTxEnvelope::Eip7702(signed) => Self::Eip7702(signed),
            }
        }
    }

    impl From<TxEnvelope> for TaggedTxEnvelope {
        fn from(value: TxEnvelope) -> Self {
            match value {
                TxEnvelope::Legacy(signed) => Self::Legacy(signed),
                TxEnvelope::Eip2930(signed) => Self::Eip2930(signed),
                TxEnvelope::Eip1559(signed) => Self::Eip1559(signed),
                TxEnvelope::Eip4844(signed) => Self::Eip4844(signed),
                TxEnvelope::Eip7702(signed) => Self::Eip7702(signed),
            }
        }
    }

    // <https://github.com/succinctlabs/kona/issues/31>
    #[test]
    fn serde_block_tx() {
        let rpc_tx = r#"{
      "blockHash": "0xc0c3190292a82c2ee148774e37e5665f6a205f5ef0cd0885e84701d90ebd442e",
      "blockNumber": "0x6edcde",
      "transactionIndex": "0x7",
      "hash": "0x2cb125e083d6d2631e3752bd2b3d757bf31bf02bfe21de0ffa46fbb118d28b19",
      "from": "0x03e5badf3bb1ade1a8f33f94536c827b6531948d",
      "to": "0x3267e72dc8780a1512fa69da7759ec66f30350e3",
      "input": "0x62e4c545000000000000000000000000464c8ec100f2f42fb4e42e07e203da2324f9fc6700000000000000000000000003e5badf3bb1ade1a8f33f94536c827b6531948d000000000000000000000000a064bfb5c7e81426647dc20a0d854da1538559dc00000000000000000000000000000000000000000000000000c6f3b40b6c0000",
      "nonce": "0x2a8",
      "value": "0x0",
      "gas": "0x28afd",
      "gasPrice": "0x23ec5dbc2",
      "accessList": [],
      "chainId": "0xaa36a7",
      "type": "0x0",
      "v": "0x1546d71",
      "r": "0x809b9f0a1777e376cd1ee5d2f551035643755edf26ea65b7a00c822a24504962",
      "s": "0x6a57bb8e21fe85c7e092868ee976fef71edca974d8c452fcf303f9180c764f64"
    }"#;

        let _ = serde_json::from_str::<MaybeTaggedTxEnvelope>(rpc_tx).unwrap();
    }

    // <https://github.com/succinctlabs/kona/issues/31>
    #[test]
    fn serde_block_tx_legacy_chain_id() {
        let rpc_tx = r#"{
      "blockHash": "0xc0c3190292a82c2ee148774e37e5665f6a205f5ef0cd0885e84701d90ebd442e",
      "blockNumber": "0x6edcde",
      "transactionIndex": "0x8",
      "hash": "0xe5b458ba9de30b47cb7c0ea836bec7b072053123a7416c5082c97f959a4eebd6",
      "from": "0x8b87f0a788cc14b4f0f374da59920f5017ff05de",
      "to": "0xcb33aa5b38d79e3d9fa8b10aff38aa201399a7e3",
      "input": "0xaf7b421018842e4628f3d9ee0e2c7679e29ed5dbaa75be75efecd392943503c9c68adce80000000000000000000000000000000000000000000000000000000000000064",
      "nonce": "0x2",
      "value": "0x0",
      "gas": "0x2dc6c0",
      "gasPrice": "0x18ef61d0a",
      "accessList": [],
      "chainId": "0xaa36a7",
      "type": "0x0",
      "v": "0x1c",
      "r": "0x5e28679806caa50d25e9cb16aef8c0c08b235241b8f6e9d86faadf70421ba664",
      "s": "0x2353bba82ef2c7ce4dd6695942399163160000272b14f9aa6cbadf011b76efa4"
    }"#;

        let _ = serde_json::from_str::<TaggedTxEnvelope>(rpc_tx).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::SignableTransaction;
    use alloc::vec::Vec;
    use alloy_eips::{
        eip2930::{AccessList, AccessListItem},
        eip4844::BlobTransactionSidecar,
        eip7702::Authorization,
    };
    #[allow(unused_imports)]
    use alloy_primitives::{b256, Bytes, TxKind};
    use alloy_primitives::{hex, Address, PrimitiveSignature as Signature, U256};
    use std::{fs, path::PathBuf, str::FromStr, vec};

    #[test]
    fn check_u8_id() {
        assert_eq!(TxType::Legacy, TxType::Legacy as u8);
        assert_eq!(TxType::Eip2930, TxType::Eip2930 as u8);
        assert_eq!(TxType::Eip1559, TxType::Eip1559 as u8);
        assert_eq!(TxType::Eip7702, TxType::Eip7702 as u8);
        assert_eq!(TxType::Eip4844, TxType::Eip4844 as u8);
    }

    #[test]
    #[cfg(feature = "k256")]
    // Test vector from https://etherscan.io/tx/0xce4dc6d7a7549a98ee3b071b67e970879ff51b5b95d1c340bacd80fa1e1aab31
    fn test_decode_live_1559_tx() {
        use alloy_primitives::address;

        let raw_tx = alloy_primitives::hex::decode("02f86f0102843b9aca0085029e7822d68298f094d9e1459a7a482635700cbc20bbaf52d495ab9c9680841b55ba3ac080a0c199674fcb29f353693dd779c017823b954b3c69dffa3cd6b2a6ff7888798039a028ca912de909e7e6cdef9cdcaf24c54dd8c1032946dfa1d85c206b32a9064fe8").unwrap();
        let res = TxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();

        assert_eq!(res.tx_type(), TxType::Eip1559);

        let tx = match res {
            TxEnvelope::Eip1559(tx) => tx,
            _ => unreachable!(),
        };

        assert_eq!(tx.tx().to, TxKind::Call(address!("D9e1459A7A482635700cBc20BBAF52D495Ab9C96")));
        let from = tx.recover_signer().unwrap();
        assert_eq!(from, address!("001e2b7dE757bA469a57bF6b23d982458a07eFcE"));
    }

    #[test]
    fn test_is_replay_protected_v() {
        let sig = Signature::test_signature();
        assert!(!&TxEnvelope::Legacy(Signed::new_unchecked(
            TxLegacy::default(),
            sig,
            Default::default(),
        ))
        .is_replay_protected());
        let r = b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565");
        let s = b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1");
        let v = false;
        let valid_sig = Signature::from_scalars_and_parity(r, s, v);
        assert!(!&TxEnvelope::Legacy(Signed::new_unchecked(
            TxLegacy::default(),
            valid_sig,
            Default::default(),
        ))
        .is_replay_protected());
        assert!(&TxEnvelope::Eip2930(Signed::new_unchecked(
            TxEip2930::default(),
            sig,
            Default::default(),
        ))
        .is_replay_protected());
        assert!(&TxEnvelope::Eip1559(Signed::new_unchecked(
            TxEip1559::default(),
            sig,
            Default::default(),
        ))
        .is_replay_protected());
        assert!(&TxEnvelope::Eip4844(Signed::new_unchecked(
            TxEip4844Variant::TxEip4844(TxEip4844::default()),
            sig,
            Default::default(),
        ))
        .is_replay_protected());
        assert!(&TxEnvelope::Eip7702(Signed::new_unchecked(
            TxEip7702::default(),
            sig,
            Default::default(),
        ))
        .is_replay_protected());
    }

    #[test]
    #[cfg(feature = "k256")]
    // Test vector from https://etherscan.io/tx/0x280cde7cdefe4b188750e76c888f13bd05ce9a4d7767730feefe8a0e50ca6fc4
    fn test_decode_live_legacy_tx() {
        use alloy_primitives::address;

        let raw_tx = alloy_primitives::bytes!("f9015482078b8505d21dba0083022ef1947a250d5630b4cf539739df2c5dacb4c659f2488d880c46549a521b13d8b8e47ff36ab50000000000000000000000000000000000000000000066ab5a608bd00a23f2fe000000000000000000000000000000000000000000000000000000000000008000000000000000000000000048c04ed5691981c42154c6167398f95e8f38a7ff00000000000000000000000000000000000000000000000000000000632ceac70000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000006c6ee5e31d828de241282b9606c8e98ea48526e225a0c9077369501641a92ef7399ff81c21639ed4fd8fc69cb793cfa1dbfab342e10aa0615facb2f1bcf3274a354cfe384a38d0cc008a11c2dd23a69111bc6930ba27a8");
        let res = TxEnvelope::decode_2718(&mut raw_tx.as_ref()).unwrap();
        assert_eq!(res.tx_type(), TxType::Legacy);

        let tx = match res {
            TxEnvelope::Legacy(tx) => tx,
            _ => unreachable!(),
        };

        assert_eq!(tx.tx().chain_id(), Some(1));

        assert_eq!(tx.tx().to, TxKind::Call(address!("7a250d5630B4cF539739dF2C5dAcb4c659F2488D")));
        assert_eq!(
            tx.hash().to_string(),
            "0x280cde7cdefe4b188750e76c888f13bd05ce9a4d7767730feefe8a0e50ca6fc4"
        );
        let from = tx.recover_signer().unwrap();
        assert_eq!(from, address!("a12e1462d0ceD572f396F58B6E2D03894cD7C8a4"));
    }

    #[test]
    #[cfg(feature = "k256")]
    // Test vector from https://sepolia.etherscan.io/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    // Blobscan: https://sepolia.blobscan.com/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    fn test_decode_live_4844_tx() {
        use crate::Transaction;
        use alloy_primitives::{address, b256};

        // https://sepolia.etherscan.io/getRawTx?tx=0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
        let raw_tx = alloy_primitives::hex::decode("0x03f9011d83aa36a7820fa28477359400852e90edd0008252089411e9ca82a3a762b4b5bd264d4173a242e7a770648080c08504a817c800f8a5a0012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921aa00152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4a0013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7a001148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1a0011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e654901a0c8de4cced43169f9aa3d36506363b2d2c44f6c49fc1fd91ea114c86f3757077ea01e11fdd0d1934eda0492606ee0bb80a7bf8f35cc5f86ec60fe5031ba48bfd544").unwrap();

        let res = TxEnvelope::decode_2718(&mut raw_tx.as_slice()).unwrap();
        assert_eq!(res.tx_type(), TxType::Eip4844);

        let tx = match res {
            TxEnvelope::Eip4844(tx) => tx,
            _ => unreachable!(),
        };

        assert_eq!(
            tx.tx().kind(),
            TxKind::Call(address!("11E9CA82A3a762b4B5bd264d4173a242e7a77064"))
        );

        // Assert this is the correct variant of the EIP-4844 enum, which only contains the tx.
        assert!(matches!(tx.tx(), TxEip4844Variant::TxEip4844(_)));

        assert_eq!(
            tx.tx().tx().blob_versioned_hashes,
            vec![
                b256!("012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921a"),
                b256!("0152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4"),
                b256!("013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7"),
                b256!("01148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1"),
                b256!("011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e6549")
            ]
        );

        let from = tx.recover_signer().unwrap();
        assert_eq!(from, address!("A83C816D4f9b2783761a22BA6FADB0eB0606D7B2"));
    }

    fn test_encode_decode_roundtrip<T: SignableTransaction<Signature>>(
        tx: T,
        signature: Option<Signature>,
    ) where
        Signed<T>: Into<TxEnvelope>,
    {
        let signature = signature.unwrap_or_else(Signature::test_signature);
        let tx_signed = tx.into_signed(signature);
        let tx_envelope: TxEnvelope = tx_signed.into();
        let encoded = tx_envelope.encoded_2718();
        let mut slice = encoded.as_slice();
        let decoded = TxEnvelope::decode_2718(&mut slice).unwrap();
        assert_eq!(encoded.len(), tx_envelope.encode_2718_len());
        assert_eq!(decoded, tx_envelope);
        assert_eq!(slice.len(), 0);
    }

    #[test]
    fn test_encode_decode_legacy() {
        let tx = TxLegacy {
            chain_id: None,
            nonce: 2,
            gas_limit: 1000000,
            gas_price: 10000000000,
            to: Address::left_padding_from(&[6]).into(),
            value: U256::from(7_u64),
            ..Default::default()
        };
        test_encode_decode_roundtrip(tx, Some(Signature::test_signature().with_parity(true)));
    }

    #[test]
    fn test_encode_decode_eip1559() {
        let tx = TxEip1559 {
            chain_id: 1u64,
            nonce: 2,
            max_fee_per_gas: 3,
            max_priority_fee_per_gas: 4,
            gas_limit: 5,
            to: Address::left_padding_from(&[6]).into(),
            value: U256::from(7_u64),
            input: vec![8].into(),
            access_list: Default::default(),
        };
        test_encode_decode_roundtrip(tx, None);
    }

    #[test]
    fn test_encode_decode_eip1559_parity_eip155() {
        let tx = TxEip1559 {
            chain_id: 1u64,
            nonce: 2,
            max_fee_per_gas: 3,
            max_priority_fee_per_gas: 4,
            gas_limit: 5,
            to: Address::left_padding_from(&[6]).into(),
            value: U256::from(7_u64),
            input: vec![8].into(),
            access_list: Default::default(),
        };
        let signature = Signature::test_signature().with_parity(true);

        test_encode_decode_roundtrip(tx, Some(signature));
    }

    #[test]
    fn test_encode_decode_eip2930_parity_eip155() {
        let tx = TxEip2930 {
            chain_id: 1u64,
            nonce: 2,
            gas_price: 3,
            gas_limit: 4,
            to: Address::left_padding_from(&[5]).into(),
            value: U256::from(6_u64),
            input: vec![7].into(),
            access_list: Default::default(),
        };
        let signature = Signature::test_signature().with_parity(true);
        test_encode_decode_roundtrip(tx, Some(signature));
    }

    #[test]
    fn test_encode_decode_eip4844_parity_eip155() {
        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 100,
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000_000,
            gas_limit: 1_000_000,
            to: Address::random(),
            value: U256::from(10e18),
            input: Bytes::new(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::random(),
                storage_keys: vec![B256::random()],
            }]),
            blob_versioned_hashes: vec![B256::random()],
            max_fee_per_blob_gas: 0,
        };
        let signature = Signature::test_signature().with_parity(true);
        test_encode_decode_roundtrip(tx, Some(signature));
    }

    #[test]
    fn test_encode_decode_eip4844_sidecar_parity_eip155() {
        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 100,
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000_000,
            gas_limit: 1_000_000,
            to: Address::random(),
            value: U256::from(10e18),
            input: Bytes::new(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::random(),
                storage_keys: vec![B256::random()],
            }]),
            blob_versioned_hashes: vec![B256::random()],
            max_fee_per_blob_gas: 0,
        };
        let sidecar = BlobTransactionSidecar {
            blobs: vec![[2; 131072].into()],
            commitments: vec![[3; 48].into()],
            proofs: vec![[4; 48].into()],
        };
        let tx = TxEip4844WithSidecar { tx, sidecar };
        let signature = Signature::test_signature().with_parity(true);

        let tx_signed = tx.into_signed(signature);
        let tx_envelope: TxEnvelope = tx_signed.into();

        let mut out = Vec::new();
        tx_envelope.network_encode(&mut out);
        let mut slice = out.as_slice();
        let decoded = TxEnvelope::network_decode(&mut slice).unwrap();
        assert_eq!(slice.len(), 0);
        assert_eq!(out.len(), tx_envelope.network_len());
        assert_eq!(decoded, tx_envelope);
    }

    #[test]
    fn test_encode_decode_eip4844_variant_parity_eip155() {
        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 100,
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000_000,
            gas_limit: 1_000_000,
            to: Address::random(),
            value: U256::from(10e18),
            input: Bytes::new(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::random(),
                storage_keys: vec![B256::random()],
            }]),
            blob_versioned_hashes: vec![B256::random()],
            max_fee_per_blob_gas: 0,
        };
        let tx = TxEip4844Variant::TxEip4844(tx);
        let signature = Signature::test_signature().with_parity(true);
        test_encode_decode_roundtrip(tx, Some(signature));
    }

    #[test]
    fn test_encode_decode_eip2930() {
        let tx = TxEip2930 {
            chain_id: 1u64,
            nonce: 2,
            gas_price: 3,
            gas_limit: 4,
            to: Address::left_padding_from(&[5]).into(),
            value: U256::from(6_u64),
            input: vec![7].into(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::left_padding_from(&[8]),
                storage_keys: vec![B256::left_padding_from(&[9])],
            }]),
        };
        test_encode_decode_roundtrip(tx, None);
    }

    #[test]
    fn test_encode_decode_eip7702() {
        let tx = TxEip7702 {
            chain_id: 1u64,
            nonce: 2,
            gas_limit: 3,
            max_fee_per_gas: 4,
            max_priority_fee_per_gas: 5,
            to: Address::left_padding_from(&[5]),
            value: U256::from(6_u64),
            input: vec![7].into(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::left_padding_from(&[8]),
                storage_keys: vec![B256::left_padding_from(&[9])],
            }]),
            authorization_list: vec![(Authorization {
                chain_id: U256::from(1),
                address: Address::left_padding_from(&[10]),
                nonce: 1u64,
            })
            .into_signed(Signature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap())],
        };
        test_encode_decode_roundtrip(tx, None);
    }

    #[test]
    fn test_encode_decode_transaction_list() {
        let signature = Signature::test_signature();
        let tx = TxEnvelope::Eip1559(
            TxEip1559 {
                chain_id: 1u64,
                nonce: 2,
                max_fee_per_gas: 3,
                max_priority_fee_per_gas: 4,
                gas_limit: 5,
                to: Address::left_padding_from(&[6]).into(),
                value: U256::from(7_u64),
                input: vec![8].into(),
                access_list: Default::default(),
            }
            .into_signed(signature),
        );
        let transactions = vec![tx.clone(), tx];
        let encoded = alloy_rlp::encode(&transactions);
        let decoded = Vec::<TxEnvelope>::decode(&mut &encoded[..]).unwrap();
        assert_eq!(transactions, decoded);
    }

    #[test]
    fn decode_encode_known_rpc_transaction() {
        // test data pulled from hive test that sends blob transactions
        let network_data_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/rpc_blob_transaction.rlp");
        let data = fs::read_to_string(network_data_path).expect("Unable to read file");
        let hex_data = hex::decode(data.trim()).unwrap();

        let tx: TxEnvelope = TxEnvelope::decode_2718(&mut hex_data.as_slice()).unwrap();
        let encoded = tx.encoded_2718();
        assert_eq!(encoded, hex_data);
        assert_eq!(tx.encode_2718_len(), hex_data.len());
    }

    #[cfg(feature = "serde")]
    fn test_serde_roundtrip<T: SignableTransaction<Signature>>(tx: T)
    where
        Signed<T>: Into<TxEnvelope>,
    {
        let signature = Signature::test_signature();
        let tx_envelope: TxEnvelope = tx.into_signed(signature).into();

        let serialized = serde_json::to_string(&tx_envelope).unwrap();

        let deserialized: TxEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(tx_envelope, deserialized);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_roundtrip_legacy() {
        let tx = TxLegacy {
            chain_id: Some(1),
            nonce: 100,
            gas_price: 3_000_000_000,
            gas_limit: 50_000,
            to: Address::default().into(),
            value: U256::from(10e18),
            input: Bytes::new(),
        };
        test_serde_roundtrip(tx);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_roundtrip_eip1559() {
        let tx = TxEip1559 {
            chain_id: 1,
            nonce: 100,
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000_000,
            gas_limit: 1_000_000,
            to: TxKind::Create,
            value: U256::from(10e18),
            input: Bytes::new(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::random(),
                storage_keys: vec![B256::random()],
            }]),
        };
        test_serde_roundtrip(tx);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_roundtrip_eip2930() {
        let tx = TxEip2930 {
            chain_id: u64::MAX,
            nonce: u64::MAX,
            gas_price: u128::MAX,
            gas_limit: u64::MAX,
            to: Address::random().into(),
            value: U256::MAX,
            input: Bytes::new(),
            access_list: Default::default(),
        };
        test_serde_roundtrip(tx);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_roundtrip_eip4844() {
        let tx = TxEip4844Variant::TxEip4844(TxEip4844 {
            chain_id: 1,
            nonce: 100,
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000_000,
            gas_limit: 1_000_000,
            to: Address::random(),
            value: U256::from(10e18),
            input: Bytes::new(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::random(),
                storage_keys: vec![B256::random()],
            }]),
            blob_versioned_hashes: vec![B256::random()],
            max_fee_per_blob_gas: 0,
        });
        test_serde_roundtrip(tx);

        let tx = TxEip4844Variant::TxEip4844WithSidecar(TxEip4844WithSidecar {
            tx: TxEip4844 {
                chain_id: 1,
                nonce: 100,
                max_fee_per_gas: 50_000_000_000,
                max_priority_fee_per_gas: 1_000_000_000_000,
                gas_limit: 1_000_000,
                to: Address::random(),
                value: U256::from(10e18),
                input: Bytes::new(),
                access_list: AccessList(vec![AccessListItem {
                    address: Address::random(),
                    storage_keys: vec![B256::random()],
                }]),
                blob_versioned_hashes: vec![B256::random()],
                max_fee_per_blob_gas: 0,
            },
            sidecar: Default::default(),
        });
        test_serde_roundtrip(tx);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_roundtrip_eip7702() {
        let tx = TxEip7702 {
            chain_id: u64::MAX,
            nonce: u64::MAX,
            gas_limit: u64::MAX,
            max_fee_per_gas: u128::MAX,
            max_priority_fee_per_gas: u128::MAX,
            to: Address::random(),
            value: U256::MAX,
            input: Bytes::new(),
            access_list: AccessList(vec![AccessListItem {
                address: Address::random(),
                storage_keys: vec![B256::random()],
            }]),
            authorization_list: vec![(Authorization {
                chain_id: U256::from(1),
                address: Address::left_padding_from(&[1]),
                nonce: 1u64,
            })
            .into_signed(Signature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap())],
        };
        test_serde_roundtrip(tx);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_tx_from_contract_call() {
        let rpc_tx = r#"{"hash":"0x018b2331d461a4aeedf6a1f9cc37463377578244e6a35216057a8370714e798f","nonce":"0x1","blockHash":"0x3ca295f1dcaf8ac073c543dc0eccf18859f411206df181731e374e9917252931","blockNumber":"0x2","transactionIndex":"0x0","from":"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266","to":"0x5fbdb2315678afecb367f032d93f642f64180aa3","value":"0x0","gasPrice":"0x3a29f0f8","gas":"0x1c9c380","maxFeePerGas":"0xba43b7400","maxPriorityFeePerGas":"0x5f5e100","input":"0xd09de08a","r":"0xd309309a59a49021281cb6bb41d164c96eab4e50f0c1bd24c03ca336e7bc2bb7","s":"0x28a7f089143d0a1355ebeb2a1b9f0e5ad9eca4303021c1400d61bc23c9ac5319","v":"0x0","yParity":"0x0","chainId":"0x7a69","accessList":[],"type":"0x2"}"#;

        let te = serde_json::from_str::<TxEnvelope>(rpc_tx).unwrap();

        assert_eq!(
            *te.tx_hash(),
            alloy_primitives::b256!(
                "018b2331d461a4aeedf6a1f9cc37463377578244e6a35216057a8370714e798f"
            )
        );
    }

    #[test]
    #[cfg(feature = "k256")]
    fn test_arbitrary_envelope() {
        use arbitrary::Arbitrary;
        let mut unstructured = arbitrary::Unstructured::new(b"arbitrary tx envelope");
        let tx = TxEnvelope::arbitrary(&mut unstructured).unwrap();

        assert!(tx.recover_signer().is_ok());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_untagged_legacy() {
        let data = r#"{
            "hash": "0x97efb58d2b42df8d68ab5899ff42b16c7e0af35ed86ae4adb8acaad7e444220c",
            "input": "0x",
            "r": "0x5d71a4a548503f2916d10c6b1a1557a0e7352eb041acb2bac99d1ad6bb49fd45",
            "s": "0x2627bf6d35be48b0e56c61733f63944c0ebcaa85cb4ed6bc7cba3161ba85e0e8",
            "v": "0x1c",
            "gas": "0x15f90",
            "from": "0x2a65aca4d5fc5b5c859090a6c34d164135398226",
            "to": "0x8fbeb4488a08d60979b5aa9e13dd00b2726320b2",
            "value": "0xf606682badd7800",
            "nonce": "0x11f398",
            "gasPrice": "0x4a817c800"
        }"#;

        let tx: TxEnvelope = serde_json::from_str(data).unwrap();

        assert!(matches!(tx, TxEnvelope::Legacy(_)));

        let data_with_wrong_type = r#"{
            "hash": "0x97efb58d2b42df8d68ab5899ff42b16c7e0af35ed86ae4adb8acaad7e444220c",
            "input": "0x",
            "r": "0x5d71a4a548503f2916d10c6b1a1557a0e7352eb041acb2bac99d1ad6bb49fd45",
            "s": "0x2627bf6d35be48b0e56c61733f63944c0ebcaa85cb4ed6bc7cba3161ba85e0e8",
            "v": "0x1c",
            "gas": "0x15f90",
            "from": "0x2a65aca4d5fc5b5c859090a6c34d164135398226",
            "to": "0x8fbeb4488a08d60979b5aa9e13dd00b2726320b2",
            "value": "0xf606682badd7800",
            "nonce": "0x11f398",
            "gasPrice": "0x4a817c800",
            "type": "0x12"
        }"#;

        assert!(serde_json::from_str::<TxEnvelope>(data_with_wrong_type).is_err());
    }

    #[test]
    fn test_tx_type_try_from_u8() {
        assert_eq!(TxType::try_from(0u8).unwrap(), TxType::Legacy);
        assert_eq!(TxType::try_from(1u8).unwrap(), TxType::Eip2930);
        assert_eq!(TxType::try_from(2u8).unwrap(), TxType::Eip1559);
        assert_eq!(TxType::try_from(3u8).unwrap(), TxType::Eip4844);
        assert_eq!(TxType::try_from(4u8).unwrap(), TxType::Eip7702);
        assert!(TxType::try_from(5u8).is_err()); // Invalid case
    }

    #[test]
    fn test_tx_type_try_from_u64() {
        assert_eq!(TxType::try_from(0u64).unwrap(), TxType::Legacy);
        assert_eq!(TxType::try_from(1u64).unwrap(), TxType::Eip2930);
        assert_eq!(TxType::try_from(2u64).unwrap(), TxType::Eip1559);
        assert_eq!(TxType::try_from(3u64).unwrap(), TxType::Eip4844);
        assert_eq!(TxType::try_from(4u64).unwrap(), TxType::Eip7702);
        assert!(TxType::try_from(10u64).is_err()); // Invalid case
    }

    #[test]
    fn test_tx_type_from_conversions() {
        let legacy_tx = Signed::new_unchecked(
            TxLegacy::default(),
            Signature::test_signature(),
            Default::default(),
        );
        let eip2930_tx = Signed::new_unchecked(
            TxEip2930::default(),
            Signature::test_signature(),
            Default::default(),
        );
        let eip1559_tx = Signed::new_unchecked(
            TxEip1559::default(),
            Signature::test_signature(),
            Default::default(),
        );
        let eip4844_variant = Signed::new_unchecked(
            TxEip4844Variant::TxEip4844(TxEip4844::default()),
            Signature::test_signature(),
            Default::default(),
        );
        let eip7702_tx = Signed::new_unchecked(
            TxEip7702::default(),
            Signature::test_signature(),
            Default::default(),
        );

        assert!(matches!(TxEnvelope::from(legacy_tx), TxEnvelope::Legacy(_)));
        assert!(matches!(TxEnvelope::from(eip2930_tx), TxEnvelope::Eip2930(_)));
        assert!(matches!(TxEnvelope::from(eip1559_tx), TxEnvelope::Eip1559(_)));
        assert!(matches!(TxEnvelope::from(eip4844_variant), TxEnvelope::Eip4844(_)));
        assert!(matches!(TxEnvelope::from(eip7702_tx), TxEnvelope::Eip7702(_)));
    }

    #[test]
    fn test_tx_type_is_methods() {
        let legacy_tx = TxEnvelope::Legacy(Signed::new_unchecked(
            TxLegacy::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip2930_tx = TxEnvelope::Eip2930(Signed::new_unchecked(
            TxEip2930::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip1559_tx = TxEnvelope::Eip1559(Signed::new_unchecked(
            TxEip1559::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip4844_tx = TxEnvelope::Eip4844(Signed::new_unchecked(
            TxEip4844Variant::TxEip4844(TxEip4844::default()),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip7702_tx = TxEnvelope::Eip7702(Signed::new_unchecked(
            TxEip7702::default(),
            Signature::test_signature(),
            Default::default(),
        ));

        assert!(legacy_tx.is_legacy());
        assert!(!legacy_tx.is_eip2930());
        assert!(!legacy_tx.is_eip1559());
        assert!(!legacy_tx.is_eip4844());
        assert!(!legacy_tx.is_eip7702());

        assert!(eip2930_tx.is_eip2930());
        assert!(!eip2930_tx.is_legacy());
        assert!(!eip2930_tx.is_eip1559());
        assert!(!eip2930_tx.is_eip4844());
        assert!(!eip2930_tx.is_eip7702());

        assert!(eip1559_tx.is_eip1559());
        assert!(!eip1559_tx.is_legacy());
        assert!(!eip1559_tx.is_eip2930());
        assert!(!eip1559_tx.is_eip4844());
        assert!(!eip1559_tx.is_eip7702());

        assert!(eip4844_tx.is_eip4844());
        assert!(!eip4844_tx.is_legacy());
        assert!(!eip4844_tx.is_eip2930());
        assert!(!eip4844_tx.is_eip1559());
        assert!(!eip4844_tx.is_eip7702());

        assert!(eip7702_tx.is_eip7702());
        assert!(!eip7702_tx.is_legacy());
        assert!(!eip7702_tx.is_eip2930());
        assert!(!eip7702_tx.is_eip1559());
        assert!(!eip7702_tx.is_eip4844());
    }

    #[test]
    fn test_tx_type() {
        let legacy_tx = TxEnvelope::Legacy(Signed::new_unchecked(
            TxLegacy::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip2930_tx = TxEnvelope::Eip2930(Signed::new_unchecked(
            TxEip2930::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip1559_tx = TxEnvelope::Eip1559(Signed::new_unchecked(
            TxEip1559::default(),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip4844_tx = TxEnvelope::Eip4844(Signed::new_unchecked(
            TxEip4844Variant::TxEip4844(TxEip4844::default()),
            Signature::test_signature(),
            Default::default(),
        ));
        let eip7702_tx = TxEnvelope::Eip7702(Signed::new_unchecked(
            TxEip7702::default(),
            Signature::test_signature(),
            Default::default(),
        ));

        assert_eq!(legacy_tx.tx_type(), TxType::Legacy);
        assert_eq!(eip2930_tx.tx_type(), TxType::Eip2930);
        assert_eq!(eip1559_tx.tx_type(), TxType::Eip1559);
        assert_eq!(eip4844_tx.tx_type(), TxType::Eip4844);
        assert_eq!(eip7702_tx.tx_type(), TxType::Eip7702);
    }

    // <https://sepolia.etherscan.io/getRawTx?tx=0xe5b458ba9de30b47cb7c0ea836bec7b072053123a7416c5082c97f959a4eebd6>
    #[test]
    fn decode_raw_legacy() {
        let raw = hex!("f8aa0285018ef61d0a832dc6c094cb33aa5b38d79e3d9fa8b10aff38aa201399a7e380b844af7b421018842e4628f3d9ee0e2c7679e29ed5dbaa75be75efecd392943503c9c68adce800000000000000000000000000000000000000000000000000000000000000641ca05e28679806caa50d25e9cb16aef8c0c08b235241b8f6e9d86faadf70421ba664a02353bba82ef2c7ce4dd6695942399163160000272b14f9aa6cbadf011b76efa4");
        let tx = TxEnvelope::decode_2718(&mut raw.as_ref()).unwrap();
        assert!(tx.chain_id().is_none());
    }
}
