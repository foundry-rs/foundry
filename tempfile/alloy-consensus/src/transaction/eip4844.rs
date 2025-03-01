use crate::{SignableTransaction, Signed, Transaction, TxType};

use alloc::vec::Vec;
use alloy_eips::{
    eip2930::AccessList, eip4844::DATA_GAS_PER_BLOB, eip7702::SignedAuthorization, Typed2718,
};
use alloy_primitives::{
    Address, Bytes, ChainId, PrimitiveSignature as Signature, TxKind, B256, U256,
};
use alloy_rlp::{BufMut, Decodable, Encodable, Header};
use core::mem;

#[doc(inline)]
pub use alloy_eips::eip4844::BlobTransactionSidecar;

#[cfg(feature = "kzg")]
#[doc(inline)]
pub use alloy_eips::eip4844::BlobTransactionValidationError;

use super::RlpEcdsaTx;

/// [EIP-4844 Blob Transaction](https://eips.ethereum.org/EIPS/eip-4844#blob-transaction)
///
/// A transaction with blob hashes and max blob fee.
/// It can either be a standalone transaction, mainly seen when retrieving historical transactions,
/// or a transaction with a sidecar, which is used when submitting a transaction to the network and
/// when receiving and sending transactions during the gossip stage.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[doc(alias = "Eip4844TransactionVariant")]
pub enum TxEip4844Variant {
    /// A standalone transaction with blob hashes and max blob fee.
    TxEip4844(TxEip4844),
    /// A transaction with a sidecar, which contains the blob data, commitments, and proofs.
    TxEip4844WithSidecar(TxEip4844WithSidecar),
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for TxEip4844Variant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct TxEip4844SerdeHelper {
            #[serde(flatten)]
            #[doc(alias = "transaction")]
            tx: TxEip4844,
            #[serde(flatten)]
            sidecar: Option<BlobTransactionSidecar>,
        }

        let tx = TxEip4844SerdeHelper::deserialize(deserializer)?;

        if let Some(sidecar) = tx.sidecar {
            Ok(TxEip4844WithSidecar::from_tx_and_sidecar(tx.tx, sidecar).into())
        } else {
            Ok(tx.tx.into())
        }
    }
}

impl From<Signed<TxEip4844>> for Signed<TxEip4844Variant> {
    fn from(value: Signed<TxEip4844>) -> Self {
        let (tx, signature, hash) = value.into_parts();
        Self::new_unchecked(TxEip4844Variant::TxEip4844(tx), signature, hash)
    }
}

impl From<Signed<TxEip4844WithSidecar>> for Signed<TxEip4844Variant> {
    fn from(value: Signed<TxEip4844WithSidecar>) -> Self {
        let (tx, signature, hash) = value.into_parts();
        Self::new_unchecked(TxEip4844Variant::TxEip4844WithSidecar(tx), signature, hash)
    }
}

impl From<TxEip4844WithSidecar> for TxEip4844Variant {
    fn from(tx: TxEip4844WithSidecar) -> Self {
        Self::TxEip4844WithSidecar(tx)
    }
}

impl From<TxEip4844> for TxEip4844Variant {
    fn from(tx: TxEip4844) -> Self {
        Self::TxEip4844(tx)
    }
}

impl From<(TxEip4844, BlobTransactionSidecar)> for TxEip4844Variant {
    fn from((tx, sidecar): (TxEip4844, BlobTransactionSidecar)) -> Self {
        TxEip4844WithSidecar::from_tx_and_sidecar(tx, sidecar).into()
    }
}

impl From<TxEip4844Variant> for TxEip4844 {
    fn from(tx: TxEip4844Variant) -> Self {
        match tx {
            TxEip4844Variant::TxEip4844(tx) => tx,
            TxEip4844Variant::TxEip4844WithSidecar(tx) => tx.tx,
        }
    }
}

impl TxEip4844Variant {
    /// Verifies that the transaction's blob data, commitments, and proofs are all valid.
    ///
    /// See also [TxEip4844::validate_blob]
    #[cfg(feature = "kzg")]
    pub fn validate(
        &self,
        proof_settings: &c_kzg::KzgSettings,
    ) -> Result<(), BlobTransactionValidationError> {
        match self {
            Self::TxEip4844(_) => Err(BlobTransactionValidationError::MissingSidecar),
            Self::TxEip4844WithSidecar(tx) => tx.validate_blob(proof_settings),
        }
    }

    /// Get the transaction type.
    #[doc(alias = "transaction_type")]
    pub const fn tx_type() -> TxType {
        TxType::Eip4844
    }

    /// Get access to the inner tx [TxEip4844].
    #[doc(alias = "transaction")]
    pub const fn tx(&self) -> &TxEip4844 {
        match self {
            Self::TxEip4844(tx) => tx,
            Self::TxEip4844WithSidecar(tx) => tx.tx(),
        }
    }

    /// Calculates a heuristic for the in-memory size of the [TxEip4844Variant] transaction.
    #[inline]
    pub fn size(&self) -> usize {
        match self {
            Self::TxEip4844(tx) => tx.size(),
            Self::TxEip4844WithSidecar(tx) => tx.size(),
        }
    }

    /// Tries to unwrap the [`TxEip4844WithSidecar`] returns the transaction as error if it is not a
    /// [`TxEip4844WithSidecar`]
    pub fn try_into_4844_with_sidecar(self) -> Result<TxEip4844WithSidecar, Self> {
        match self {
            Self::TxEip4844WithSidecar(tx) => Ok(tx),
            _ => Err(self),
        }
    }
}

impl Transaction for TxEip4844Variant {
    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        match self {
            Self::TxEip4844(tx) => Some(tx.chain_id),
            Self::TxEip4844WithSidecar(tx) => Some(tx.tx().chain_id),
        }
    }

    #[inline]
    fn nonce(&self) -> u64 {
        match self {
            Self::TxEip4844(tx) => tx.nonce,
            Self::TxEip4844WithSidecar(tx) => tx.tx().nonce,
        }
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        match self {
            Self::TxEip4844(tx) => tx.gas_limit,
            Self::TxEip4844WithSidecar(tx) => tx.tx().gas_limit,
        }
    }

    #[inline]
    fn gas_price(&self) -> Option<u128> {
        None
    }

    #[inline]
    fn max_fee_per_gas(&self) -> u128 {
        match self {
            Self::TxEip4844(tx) => tx.max_fee_per_gas(),
            Self::TxEip4844WithSidecar(tx) => tx.max_fee_per_gas(),
        }
    }

    #[inline]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        match self {
            Self::TxEip4844(tx) => tx.max_priority_fee_per_gas(),
            Self::TxEip4844WithSidecar(tx) => tx.max_priority_fee_per_gas(),
        }
    }

    #[inline]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        match self {
            Self::TxEip4844(tx) => tx.max_fee_per_blob_gas(),
            Self::TxEip4844WithSidecar(tx) => tx.max_fee_per_blob_gas(),
        }
    }

    #[inline]
    fn priority_fee_or_price(&self) -> u128 {
        match self {
            Self::TxEip4844(tx) => tx.priority_fee_or_price(),
            Self::TxEip4844WithSidecar(tx) => tx.priority_fee_or_price(),
        }
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        match self {
            Self::TxEip4844(tx) => tx.effective_gas_price(base_fee),
            Self::TxEip4844WithSidecar(tx) => tx.effective_gas_price(base_fee),
        }
    }

    #[inline]
    fn is_dynamic_fee(&self) -> bool {
        match self {
            Self::TxEip4844(tx) => tx.is_dynamic_fee(),
            Self::TxEip4844WithSidecar(tx) => tx.is_dynamic_fee(),
        }
    }

    #[inline]
    fn kind(&self) -> TxKind {
        match self {
            Self::TxEip4844(tx) => tx.to,
            Self::TxEip4844WithSidecar(tx) => tx.tx.to,
        }
        .into()
    }

    #[inline]
    fn is_create(&self) -> bool {
        false
    }

    #[inline]
    fn value(&self) -> U256 {
        match self {
            Self::TxEip4844(tx) => tx.value,
            Self::TxEip4844WithSidecar(tx) => tx.tx.value,
        }
    }

    #[inline]
    fn input(&self) -> &Bytes {
        match self {
            Self::TxEip4844(tx) => tx.input(),
            Self::TxEip4844WithSidecar(tx) => tx.tx().input(),
        }
    }

    #[inline]
    fn access_list(&self) -> Option<&AccessList> {
        match self {
            Self::TxEip4844(tx) => tx.access_list(),
            Self::TxEip4844WithSidecar(tx) => tx.access_list(),
        }
    }

    #[inline]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        match self {
            Self::TxEip4844(tx) => tx.blob_versioned_hashes(),
            Self::TxEip4844WithSidecar(tx) => tx.blob_versioned_hashes(),
        }
    }

    #[inline]
    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        None
    }
}
impl Typed2718 for TxEip4844 {
    fn ty(&self) -> u8 {
        TxType::Eip4844 as u8
    }
}

impl RlpEcdsaTx for TxEip4844Variant {
    const DEFAULT_TX_TYPE: u8 = { Self::tx_type() as u8 };

    fn rlp_encoded_fields_length(&self) -> usize {
        match self {
            Self::TxEip4844(inner) => inner.rlp_encoded_fields_length(),
            Self::TxEip4844WithSidecar(inner) => inner.rlp_encoded_fields_length(),
        }
    }

    fn rlp_encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        match self {
            Self::TxEip4844(inner) => inner.rlp_encode_fields(out),
            Self::TxEip4844WithSidecar(inner) => inner.rlp_encode_fields(out),
        }
    }

    fn rlp_header_signed(&self, signature: &Signature) -> Header {
        match self {
            Self::TxEip4844(inner) => inner.rlp_header_signed(signature),
            Self::TxEip4844WithSidecar(inner) => inner.rlp_header_signed(signature),
        }
    }

    fn rlp_encode_signed(&self, signature: &Signature, out: &mut dyn BufMut) {
        match self {
            Self::TxEip4844(inner) => inner.rlp_encode_signed(signature, out),
            Self::TxEip4844WithSidecar(inner) => inner.rlp_encode_signed(signature, out),
        }
    }

    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let needle = &mut &**buf;

        // We also need to do a trial decoding of WithSidecar to see if it
        // works. The trial ref is consumed to look for a WithSidecar.
        let trial = &mut &**buf;

        // If the next bytes are a header, one of 3 things is true:
        // - If the header is a list, this is a WithSidecar tx
        // - If there is no header, this is a non-sidecar tx with a single-byte chain ID.
        // - If there is a string header, this is a non-sidecar tx with a multi-byte chain ID.
        // To check these, we first try to decode the header. If it fails or is
        // not a list, we lmow that it is a non-sidecar transaction.
        if Header::decode(needle).is_ok_and(|h| h.list) {
            if let Ok(tx) = TxEip4844WithSidecar::rlp_decode_fields(trial) {
                *buf = *trial;
                return Ok(tx.into());
            }
        }
        TxEip4844::rlp_decode_fields(buf).map(Into::into)
    }

    fn rlp_decode_with_signature(buf: &mut &[u8]) -> alloy_rlp::Result<(Self, Signature)> {
        // We need to determine if this has a sidecar tx or not. The needle ref
        // is consumed to look for headers.
        let needle = &mut &**buf;

        // We also need to do a trial decoding of WithSidecar to see if it
        // works. The original ref is consumed to look for a WithSidecar.
        let trial = &mut &**buf;

        // First we decode the outer header
        Header::decode(needle)?;

        // If the next bytes are a header, one of 3 things is true:
        // - If the header is a list, this is a WithSidecar tx
        // - If there is no header, this is a non-sidecar tx with a single-byte chain ID.
        // - If there is a string header, this is a non-sidecar tx with a multi-byte chain ID.
        // To check these, we first try to decode the header. If it fails or is
        // not a list, we lmow that it is a non-sidecar transaction.
        if Header::decode(needle).is_ok_and(|h| h.list) {
            if let Ok((tx, signature)) = TxEip4844WithSidecar::rlp_decode_with_signature(trial) {
                // If succesful, we need to consume the trial buffer up to
                // the same point.
                *buf = *trial;
                return Ok((tx.into(), signature));
            }
        }
        TxEip4844::rlp_decode_with_signature(buf).map(|(tx, signature)| (tx.into(), signature))
    }

    fn tx_hash_with_type(&self, signature: &Signature, ty: u8) -> alloy_primitives::TxHash {
        match self {
            Self::TxEip4844(inner) => inner.tx_hash_with_type(signature, ty),
            Self::TxEip4844WithSidecar(inner) => inner.tx_hash_with_type(signature, ty),
        }
    }
}

impl Typed2718 for TxEip4844Variant {
    fn ty(&self) -> u8 {
        TxType::Eip4844 as u8
    }
}

impl SignableTransaction<Signature> for TxEip4844Variant {
    fn set_chain_id(&mut self, chain_id: ChainId) {
        match self {
            Self::TxEip4844(inner) => {
                inner.set_chain_id(chain_id);
            }
            Self::TxEip4844WithSidecar(inner) => {
                inner.set_chain_id(chain_id);
            }
        }
    }

    fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut) {
        // A signature for a [TxEip4844WithSidecar] is a signature over the [TxEip4844Variant]
        // EIP-2718 payload fields:
        // (BLOB_TX_TYPE ||
        //   rlp([chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas, gas_limit, to, value,
        //     data, access_list, max_fee_per_blob_gas, blob_versioned_hashes]))
        self.tx().encode_for_signing(out);
    }

    fn payload_len_for_signature(&self) -> usize {
        self.tx().payload_len_for_signature()
    }

    fn into_signed(self, signature: Signature) -> Signed<Self> {
        let hash = self.tx_hash(&signature);

        Signed::new_unchecked(self, signature, hash)
    }
}

/// [EIP-4844 Blob Transaction](https://eips.ethereum.org/EIPS/eip-4844#blob-transaction)
///
/// A transaction with blob hashes and max blob fee. It does not have the Blob sidecar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[doc(alias = "Eip4844Transaction", alias = "TransactionEip4844", alias = "Eip4844Tx")]
pub struct TxEip4844 {
    /// Added as EIP-pub 155: Simple replay attack protection
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub chain_id: ChainId,
    /// A scalar value equal to the number of transactions sent by the sender; formally Tn.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub nonce: u64,
    /// A scalar value equal to the maximum
    /// amount of gas that should be used in executing
    /// this transaction. This is paid up-front, before any
    /// computation is done and may not be increased
    /// later; formally Tg.
    #[cfg_attr(
        feature = "serde",
        serde(with = "alloy_serde::quantity", rename = "gas", alias = "gasLimit")
    )]
    pub gas_limit: u64,
    /// A scalar value equal to the maximum
    /// amount of gas that should be used in executing
    /// this transaction. This is paid up-front, before any
    /// computation is done and may not be increased
    /// later; formally Tg.
    ///
    /// As ethereum circulation is around 120mil eth as of 2022 that is around
    /// 120000000000000000000000000 wei we are safe to use u128 as its max number is:
    /// 340282366920938463463374607431768211455
    ///
    /// This is also known as `GasFeeCap`
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub max_fee_per_gas: u128,
    /// Max Priority fee that transaction is paying
    ///
    /// As ethereum circulation is around 120mil eth as of 2022 that is around
    /// 120000000000000000000000000 wei we are safe to use u128 as its max number is:
    /// 340282366920938463463374607431768211455
    ///
    /// This is also known as `GasTipCap`
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub max_priority_fee_per_gas: u128,
    /// The 160-bit address of the message call’s recipient.
    pub to: Address,
    /// A scalar value equal to the number of Wei to
    /// be transferred to the message call’s recipient or,
    /// in the case of contract creation, as an endowment
    /// to the newly created account; formally Tv.
    pub value: U256,
    /// The accessList specifies a list of addresses and storage keys;
    /// these addresses and storage keys are added into the `accessed_addresses`
    /// and `accessed_storage_keys` global sets (introduced in EIP-2929).
    /// A gas cost is charged, though at a discount relative to the cost of
    /// accessing outside the list.
    pub access_list: AccessList,

    /// It contains a vector of fixed size hash(32 bytes)
    pub blob_versioned_hashes: Vec<B256>,

    /// Max fee per data gas
    ///
    /// aka BlobFeeCap or blobGasFeeCap
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub max_fee_per_blob_gas: u128,

    /// Input has two uses depending if transaction is Create or Call (if `to` field is None or
    /// Some). pub init: An unlimited size byte array specifying the
    /// EVM-code for the account initialisation procedure CREATE,
    /// data: An unlimited size byte array specifying the
    /// input data of the message call, formally Td.
    pub input: Bytes,
}

impl TxEip4844 {
    /// Returns the total gas for all blobs in this transaction.
    #[inline]
    pub fn blob_gas(&self) -> u64 {
        // SAFETY: we don't expect u64::MAX / DATA_GAS_PER_BLOB hashes in a single transaction
        self.blob_versioned_hashes.len() as u64 * DATA_GAS_PER_BLOB
    }

    /// Verifies that the given blob data, commitments, and proofs are all valid for this
    /// transaction.
    ///
    /// Takes as input the [KzgSettings](c_kzg::KzgSettings), which should contain the parameters
    /// derived from the KZG trusted setup.
    ///
    /// This ensures that the blob transaction payload has the same number of blob data elements,
    /// commitments, and proofs. Each blob data element is verified against its commitment and
    /// proof.
    ///
    /// Returns [BlobTransactionValidationError::InvalidProof] if any blob KZG proof in the response
    /// fails to verify, or if the versioned hashes in the transaction do not match the actual
    /// commitment versioned hashes.
    #[cfg(feature = "kzg")]
    pub fn validate_blob(
        &self,
        sidecar: &BlobTransactionSidecar,
        proof_settings: &c_kzg::KzgSettings,
    ) -> Result<(), BlobTransactionValidationError> {
        sidecar.validate(&self.blob_versioned_hashes, proof_settings)
    }

    /// Get transaction type.
    #[doc(alias = "transaction_type")]
    pub const fn tx_type() -> TxType {
        TxType::Eip4844
    }

    /// Calculates a heuristic for the in-memory size of the [TxEip4844Variant] transaction.
    #[inline]
    pub fn size(&self) -> usize {
        mem::size_of::<ChainId>() + // chain_id
        mem::size_of::<u64>() + // nonce
        mem::size_of::<u64>() + // gas_limit
        mem::size_of::<u128>() + // max_fee_per_gas
        mem::size_of::<u128>() + // max_priority_fee_per_gas
        mem::size_of::<Address>() + // to
        mem::size_of::<U256>() + // value
        self.access_list.size() + // access_list
        self.input.len() +  // input
        self.blob_versioned_hashes.capacity() * mem::size_of::<B256>() + // blob hashes size
        mem::size_of::<u128>() // max_fee_per_data_gas
    }
}

impl RlpEcdsaTx for TxEip4844 {
    const DEFAULT_TX_TYPE: u8 = { Self::tx_type() as u8 };

    fn rlp_encoded_fields_length(&self) -> usize {
        self.chain_id.length()
            + self.nonce.length()
            + self.gas_limit.length()
            + self.max_fee_per_gas.length()
            + self.max_priority_fee_per_gas.length()
            + self.to.length()
            + self.value.length()
            + self.access_list.length()
            + self.blob_versioned_hashes.length()
            + self.max_fee_per_blob_gas.length()
            + self.input.0.length()
    }

    fn rlp_encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.chain_id.encode(out);
        self.nonce.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        self.to.encode(out);
        self.value.encode(out);
        self.input.0.encode(out);
        self.access_list.encode(out);
        self.max_fee_per_blob_gas.encode(out);
        self.blob_versioned_hashes.encode(out);
    }

    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Ok(Self {
            chain_id: Decodable::decode(buf)?,
            nonce: Decodable::decode(buf)?,
            max_priority_fee_per_gas: Decodable::decode(buf)?,
            max_fee_per_gas: Decodable::decode(buf)?,
            gas_limit: Decodable::decode(buf)?,
            to: Decodable::decode(buf)?,
            value: Decodable::decode(buf)?,
            input: Decodable::decode(buf)?,
            access_list: Decodable::decode(buf)?,
            max_fee_per_blob_gas: Decodable::decode(buf)?,
            blob_versioned_hashes: Decodable::decode(buf)?,
        })
    }
}

impl SignableTransaction<Signature> for TxEip4844 {
    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.chain_id = chain_id;
    }

    fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut) {
        out.put_u8(Self::tx_type() as u8);
        self.encode(out);
    }

    fn payload_len_for_signature(&self) -> usize {
        self.length() + 1
    }

    fn into_signed(self, signature: Signature) -> Signed<Self> {
        let hash = self.tx_hash(&signature);
        Signed::new_unchecked(self, signature, hash)
    }
}

impl Transaction for TxEip4844 {
    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        Some(self.chain_id)
    }

    #[inline]
    fn nonce(&self) -> u64 {
        self.nonce
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    #[inline]
    fn gas_price(&self) -> Option<u128> {
        None
    }

    #[inline]
    fn max_fee_per_gas(&self) -> u128 {
        self.max_fee_per_gas
    }

    #[inline]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        Some(self.max_priority_fee_per_gas)
    }

    #[inline]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        Some(self.max_fee_per_blob_gas)
    }

    #[inline]
    fn priority_fee_or_price(&self) -> u128 {
        self.max_priority_fee_per_gas
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        base_fee.map_or(self.max_fee_per_gas, |base_fee| {
            // if the tip is greater than the max priority fee per gas, set it to the max
            // priority fee per gas + base fee
            let tip = self.max_fee_per_gas.saturating_sub(base_fee as u128);
            if tip > self.max_priority_fee_per_gas {
                self.max_priority_fee_per_gas + base_fee as u128
            } else {
                // otherwise return the max fee per gas
                self.max_fee_per_gas
            }
        })
    }

    #[inline]
    fn is_dynamic_fee(&self) -> bool {
        true
    }

    #[inline]
    fn kind(&self) -> TxKind {
        self.to.into()
    }

    #[inline]
    fn is_create(&self) -> bool {
        false
    }

    #[inline]
    fn value(&self) -> U256 {
        self.value
    }

    #[inline]
    fn input(&self) -> &Bytes {
        &self.input
    }

    #[inline]
    fn access_list(&self) -> Option<&AccessList> {
        Some(&self.access_list)
    }

    #[inline]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        Some(&self.blob_versioned_hashes)
    }

    #[inline]
    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        None
    }
}

impl Encodable for TxEip4844 {
    fn encode(&self, out: &mut dyn BufMut) {
        self.rlp_encode(out);
    }

    fn length(&self) -> usize {
        self.rlp_encoded_length()
    }
}

impl Decodable for TxEip4844 {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Self::rlp_decode(buf)
    }
}

impl From<TxEip4844WithSidecar> for TxEip4844 {
    /// Consumes the [TxEip4844WithSidecar] and returns the inner [TxEip4844].
    fn from(tx_with_sidecar: TxEip4844WithSidecar) -> Self {
        tx_with_sidecar.tx
    }
}

/// [EIP-4844 Blob Transaction](https://eips.ethereum.org/EIPS/eip-4844#blob-transaction)
///
/// A transaction with blob hashes and max blob fee, which also includes the
/// [BlobTransactionSidecar]. This is the full type sent over the network as a raw transaction. It
/// wraps a [TxEip4844] to include the sidecar and the ability to decode it properly.
///
/// This is defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844#networking) as an element
/// of a `PooledTransactions` response, and is also used as the format for sending raw transactions
/// through the network (eth_sendRawTransaction/eth_sendTransaction).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[doc(alias = "Eip4844TransactionWithSidecar", alias = "Eip4844TxWithSidecar")]
pub struct TxEip4844WithSidecar {
    /// The actual transaction.
    #[cfg_attr(feature = "serde", serde(flatten))]
    #[doc(alias = "transaction")]
    pub tx: TxEip4844,
    /// The sidecar.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub sidecar: BlobTransactionSidecar,
}

impl TxEip4844WithSidecar {
    /// Constructs a new [TxEip4844WithSidecar] from a [TxEip4844] and a [BlobTransactionSidecar].
    #[doc(alias = "from_transaction_and_sidecar")]
    pub const fn from_tx_and_sidecar(tx: TxEip4844, sidecar: BlobTransactionSidecar) -> Self {
        Self { tx, sidecar }
    }

    /// Verifies that the transaction's blob data, commitments, and proofs are all valid.
    ///
    /// See also [TxEip4844::validate_blob]
    #[cfg(feature = "kzg")]
    pub fn validate_blob(
        &self,
        proof_settings: &c_kzg::KzgSettings,
    ) -> Result<(), BlobTransactionValidationError> {
        self.tx.validate_blob(&self.sidecar, proof_settings)
    }

    /// Get the transaction type.
    #[doc(alias = "transaction_type")]
    pub const fn tx_type() -> TxType {
        TxEip4844::tx_type()
    }

    /// Get access to the inner tx [TxEip4844].
    #[doc(alias = "transaction")]
    pub const fn tx(&self) -> &TxEip4844 {
        &self.tx
    }

    /// Get access to the inner sidecar [BlobTransactionSidecar].
    pub const fn sidecar(&self) -> &BlobTransactionSidecar {
        &self.sidecar
    }

    /// Consumes the [TxEip4844WithSidecar] and returns the inner sidecar [BlobTransactionSidecar].
    pub fn into_sidecar(self) -> BlobTransactionSidecar {
        self.sidecar
    }

    /// Consumes the [TxEip4844WithSidecar] and returns the inner [TxEip4844] and
    /// [BlobTransactionSidecar].
    pub fn into_parts(self) -> (TxEip4844, BlobTransactionSidecar) {
        (self.tx, self.sidecar)
    }

    /// Calculates a heuristic for the in-memory size of the [TxEip4844WithSidecar] transaction.
    #[inline]
    pub fn size(&self) -> usize {
        self.tx.size() + self.sidecar.size()
    }
}

impl SignableTransaction<Signature> for TxEip4844WithSidecar {
    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.tx.chain_id = chain_id;
    }

    fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut) {
        // A signature for a [TxEip4844WithSidecar] is a signature over the [TxEip4844] EIP-2718
        // payload fields:
        // (BLOB_TX_TYPE ||
        //   rlp([chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas, gas_limit, to, value,
        //     data, access_list, max_fee_per_blob_gas, blob_versioned_hashes]))
        self.tx.encode_for_signing(out);
    }

    fn payload_len_for_signature(&self) -> usize {
        // The payload length is the length of the `transaction_payload_body` list.
        // The sidecar is NOT included.
        self.tx.payload_len_for_signature()
    }

    fn into_signed(self, signature: Signature) -> Signed<Self, Signature> {
        // important: must hash the tx WITHOUT the sidecar
        let hash = self.tx_hash(&signature);

        Signed::new_unchecked(self, signature, hash)
    }
}

impl Transaction for TxEip4844WithSidecar {
    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        self.tx.chain_id()
    }

    #[inline]
    fn nonce(&self) -> u64 {
        self.tx.nonce()
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        self.tx.gas_limit()
    }

    #[inline]
    fn gas_price(&self) -> Option<u128> {
        self.tx.gas_price()
    }

    #[inline]
    fn max_fee_per_gas(&self) -> u128 {
        self.tx.max_fee_per_gas()
    }

    #[inline]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.tx.max_priority_fee_per_gas()
    }

    #[inline]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.tx.max_fee_per_blob_gas()
    }

    #[inline]
    fn priority_fee_or_price(&self) -> u128 {
        self.tx.priority_fee_or_price()
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.tx.effective_gas_price(base_fee)
    }

    #[inline]
    fn is_dynamic_fee(&self) -> bool {
        self.tx.is_dynamic_fee()
    }

    #[inline]
    fn kind(&self) -> TxKind {
        self.tx.kind()
    }

    #[inline]
    fn is_create(&self) -> bool {
        false
    }

    #[inline]
    fn value(&self) -> U256 {
        self.tx.value()
    }

    #[inline]
    fn input(&self) -> &Bytes {
        self.tx.input()
    }

    #[inline]
    fn access_list(&self) -> Option<&AccessList> {
        Some(&self.tx.access_list)
    }

    #[inline]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.tx.blob_versioned_hashes()
    }

    #[inline]
    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        None
    }
}

impl Typed2718 for TxEip4844WithSidecar {
    fn ty(&self) -> u8 {
        TxType::Eip4844 as u8
    }
}

impl RlpEcdsaTx for TxEip4844WithSidecar {
    const DEFAULT_TX_TYPE: u8 = { Self::tx_type() as u8 };

    fn rlp_encoded_fields_length(&self) -> usize {
        self.sidecar.rlp_encoded_fields_length() + self.tx.rlp_encoded_length()
    }

    fn rlp_encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.tx.rlp_encode(out);
        self.sidecar.rlp_encode_fields(out);
    }

    fn rlp_header_signed(&self, signature: &Signature) -> Header {
        let payload_length = self.tx.rlp_encoded_length_with_signature(signature)
            + self.sidecar.rlp_encoded_fields_length();
        Header { list: true, payload_length }
    }

    fn rlp_encode_signed(&self, signature: &Signature, out: &mut dyn BufMut) {
        self.rlp_header_signed(signature).encode(out);
        self.tx.rlp_encode_signed(signature, out);
        self.sidecar.rlp_encode_fields(out);
    }

    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let tx = TxEip4844::rlp_decode(buf)?;
        let sidecar = BlobTransactionSidecar::rlp_decode_fields(buf)?;
        Ok(Self { tx, sidecar })
    }

    fn rlp_decode_with_signature(buf: &mut &[u8]) -> alloy_rlp::Result<(Self, Signature)> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }
        let remaining = buf.len();

        let (tx, signature) = TxEip4844::rlp_decode_with_signature(buf)?;
        let sidecar = BlobTransactionSidecar::rlp_decode_fields(buf)?;

        if buf.len() + header.payload_length != remaining {
            return Err(alloy_rlp::Error::UnexpectedLength);
        }

        Ok((Self { tx, sidecar }, signature))
    }

    fn tx_hash_with_type(&self, signature: &Signature, ty: u8) -> alloy_primitives::TxHash {
        // eip4844 tx_hash is always based on the non-sidecar encoding
        self.tx.tx_hash_with_type(signature, ty)
    }
}

#[cfg(test)]
mod tests {
    use super::{BlobTransactionSidecar, TxEip4844, TxEip4844WithSidecar};
    use crate::{transaction::eip4844::TxEip4844Variant, SignableTransaction, TxEnvelope};
    use alloy_eips::eip2930::AccessList;
    use alloy_primitives::{address, b256, bytes, PrimitiveSignature as Signature, U256};
    use alloy_rlp::{Decodable, Encodable};

    #[test]
    fn different_sidecar_same_hash() {
        // this should make sure that the hash calculated for the `into_signed` conversion does not
        // change if the sidecar is different
        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 1,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit: 1,
            to: Default::default(),
            value: U256::from(1),
            access_list: Default::default(),
            blob_versioned_hashes: vec![Default::default()],
            max_fee_per_blob_gas: 1,
            input: Default::default(),
        };
        let sidecar = BlobTransactionSidecar {
            blobs: vec![[2; 131072].into()],
            commitments: vec![[3; 48].into()],
            proofs: vec![[4; 48].into()],
        };
        let mut tx = TxEip4844WithSidecar { tx, sidecar };
        let signature = Signature::test_signature();

        // turn this transaction into_signed
        let expected_signed = tx.clone().into_signed(signature);

        // change the sidecar, adding a single (blob, commitment, proof) pair
        tx.sidecar = BlobTransactionSidecar {
            blobs: vec![[1; 131072].into()],
            commitments: vec![[1; 48].into()],
            proofs: vec![[1; 48].into()],
        };

        // turn this transaction into_signed
        let actual_signed = tx.into_signed(signature);

        // the hashes should be the same
        assert_eq!(expected_signed.hash(), actual_signed.hash());

        // convert to envelopes
        let expected_envelope: TxEnvelope = expected_signed.into();
        let actual_envelope: TxEnvelope = actual_signed.into();

        // now encode the transaction and check the length
        let len = expected_envelope.length();
        let mut buf = Vec::with_capacity(len);
        expected_envelope.encode(&mut buf);
        assert_eq!(buf.len(), len);

        // ensure it's also the same size that `actual` claims to be, since we just changed the
        // sidecar values.
        assert_eq!(buf.len(), actual_envelope.length());

        // now decode the transaction and check the values
        let decoded = TxEnvelope::decode(&mut &buf[..]).unwrap();
        assert_eq!(decoded, expected_envelope);
    }

    #[test]
    fn test_4844_variant_into_signed_correct_hash() {
        // Taken from <https://etherscan.io/tx/0x93fc9daaa0726c3292a2e939df60f7e773c6a6a726a61ce43f4a217c64d85e87>
        let tx =
            TxEip4844 {
                chain_id: 1,
                nonce: 15435,
                gas_limit: 8000000,
                max_fee_per_gas: 10571233596,
                max_priority_fee_per_gas: 1000000000,
                to: address!("a8cb082a5a689e0d594d7da1e2d72a3d63adc1bd"),
                value: U256::ZERO,
                access_list: AccessList::default(),
                blob_versioned_hashes: vec![
                    b256!("01e5276d91ac1ddb3b1c2d61295211220036e9a04be24c00f76916cc2659d004"),
                    b256!("0128eb58aff09fd3a7957cd80aa86186d5849569997cdfcfa23772811b706cc2"),
                ],
                max_fee_per_blob_gas: 1,
                input: bytes!("701f58c50000000000000000000000000000000000000000000000000000000000073fb1ed12e288def5b439ea074b398dbb4c967f2852baac3238c5fe4b62b871a59a6d00000000000000000000000000000000000000000000000000000000123971da000000000000000000000000000000000000000000000000000000000000000ac39b2a24e1dbdd11a1e7bd7c0f4dfd7d9b9cfa0997d033ad05f961ba3b82c6c83312c967f10daf5ed2bffe309249416e03ee0b101f2b84d2102b9e38b0e4dfdf0000000000000000000000000000000000000000000000000000000066254c8b538dcc33ecf5334bbd294469f9d4fd084a3090693599a46d6c62567747cbc8660000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000073fb20000000000000000000000000000000000000000000000000000000066254da10000000000000000000000000000000000000000000000000000000012397d5e20b09b263779fda4171c341e720af8fa469621ff548651f8dbbc06c2d320400c000000000000000000000000000000000000000000000000000000000000000b50a833bb11af92814e99c6ff7cf7ba7042827549d6f306a04270753702d897d8fc3c411b99159939ac1c16d21d3057ddc8b2333d1331ab34c938cff0eb29ce2e43241c170344db6819f76b1f1e0ab8206f3ec34120312d275c4f5bbea7f5c55700000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000480000000000000000000000000000000000000000000000000000000000000031800000000000000000000000000000000000000000000800b0000000000000000000000000000000000000000000000000000000000000004ed12e288def5b439ea074b398dbb4c967f2852baac3238c5fe4b62b871a59a6d00000ca8000000000000000000000000000000000000800b000000000000000000000000000000000000000000000000000000000000000300000000000000000000000066254da100000000000000000000000066254e9d00010ca80000000000000000000000000000000000008001000000000000000000000000000000000000000000000000000000000000000550a833bb11af92814e99c6ff7cf7ba7042827549d6f306a04270753702d897d800010ca800000000000000000000000000000000000080010000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000b00010ca8000000000000000000000000000000000000801100000000000000000000000000000000000000000000000000000000000000075c1cd5bd0fd333ce9d7c8edfc79f43b8f345b4a394f6aba12a2cc78ce4012ed700010ca80000000000000000000000000000000000008011000000000000000000000000000000000000000000000000000000000000000845392775318aa47beaafbdc827da38c9f1e88c3bdcabba2cb493062e17cbf21e00010ca800000000000000000000000000000000000080080000000000000000000000000000000000000000000000000000000000000000c094e20e7ac9b433f44a5885e3bdc07e51b309aeb993caa24ba84a661ac010c100010ca800000000000000000000000000000000000080080000000000000000000000000000000000000000000000000000000000000001ab42db8f4ed810bdb143368a2b641edf242af6e3d0de8b1486e2b0e7880d431100010ca8000000000000000000000000000000000000800800000000000000000000000000000000000000000000000000000000000000022d94e4cc4525e4e2d81e8227b6172e97076431a2cf98792d978035edd6e6f3100000000000000000000000000000000000000000000000000000000000000000000000000000012101c74dfb80a80fccb9a4022b2406f79f56305e6a7c931d30140f5d372fe793837e93f9ec6b8d89a9d0ab222eeb27547f66b90ec40fbbdd2a4936b0b0c19ca684ff78888fbf5840d7c8dc3c493b139471750938d7d2c443e2d283e6c5ee9fde3765a756542c42f002af45c362b4b5b1687a8fc24cbf16532b903f7bb289728170dcf597f5255508c623ba247735538376f494cdcdd5bd0c4cb067526eeda0f4745a28d8baf8893ecc1b8cee80690538d66455294a028da03ff2add9d8a88e6ee03ba9ffe3ad7d91d6ac9c69a1f28c468f00fe55eba5651a2b32dc2458e0d14b4dd6d0173df255cd56aa01e8e38edec17ea8933f68543cbdc713279d195551d4211bed5c91f77259a695e6768f6c4b110b2158fcc42423a96dcc4e7f6fddb3e2369d00000000000000000000000000000000000000000000000000000000000000") };
        let variant = TxEip4844Variant::TxEip4844(tx);

        let signature = Signature::new(
            b256!("6c173c3c8db3e3299f2f728d293b912c12e75243e3aa66911c2329b58434e2a4").into(),
            b256!("7dd4d1c228cedc5a414a668ab165d9e888e61e4c3b44cd7daf9cdcc4cec5d6b2").into(),
            false,
        );

        let signed = variant.into_signed(signature);
        assert_eq!(
            *signed.hash(),
            b256!("93fc9daaa0726c3292a2e939df60f7e773c6a6a726a61ce43f4a217c64d85e87")
        );
    }
}
