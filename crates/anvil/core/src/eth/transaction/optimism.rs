use alloy_consensus::TxType;
use alloy_network::{Transaction, TxKind};
use alloy_primitives::{Address, Bytes, ChainId, Signature, B256, U256};
use alloy_rlp::{
    length_of_length, Decodable, Encodable, Error as DecodeError, Header as RlpHeader,
};
use std::mem;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositTransactionRequest {
    pub source_hash: B256,
    pub from: Address,
    pub kind: TxKind,
    pub mint: U256,
    pub value: U256,
    pub gas_limit: U256,
    pub is_system_tx: bool,
    pub input: Bytes,
}

impl DepositTransactionRequest {
    pub fn hash(&self) -> B256 {
        B256::from_slice(alloy_primitives::keccak256(alloy_rlp::encode(self)).as_slice())
    }

    /// Encodes only the transaction's fields into the desired buffer, without a RLP header.
    pub(crate) fn encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.from.encode(out);
        self.source_hash.encode(out);
        self.kind.encode(out);
        self.mint.encode(out);
        self.value.encode(out);
        self.gas_limit.encode(out);
        self.is_system_tx.encode(out);
        self.input.encode(out);
    }

    /// Calculates the length of the RLP-encoded transaction's fields.
    pub(crate) fn fields_len(&self) -> usize {
        let mut len = 0;
        len += self.source_hash.length();
        len += self.from.length();
        len += self.kind.length();
        len += self.mint.length();
        len += self.value.length();
        len += self.gas_limit.length();
        len += self.is_system_tx.length();
        len += self.input.length();
        len
    }

    /// Decodes the inner [DepositTransactionRequest] fields from RLP bytes.
    ///
    /// NOTE: This assumes a RLP header has already been decoded, and _just_ decodes the following
    /// RLP fields in the following order:
    ///
    /// - `source_hash`
    /// - `from`
    /// - `kind`
    /// - `mint`
    /// - `value`
    /// - `gas_limit`
    /// - `is_system_tx`
    /// - `input`
    pub fn decode_inner(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            source_hash: Decodable::decode(buf)?,
            from: Decodable::decode(buf)?,
            kind: Decodable::decode(buf)?,
            mint: Decodable::decode(buf)?,
            value: Decodable::decode(buf)?,
            gas_limit: Decodable::decode(buf)?,
            is_system_tx: Decodable::decode(buf)?,
            input: Decodable::decode(buf)?,
        })
    }

    /// Inner encoding function that is used for both rlp [`Encodable`] trait and for calculating
    /// hash that for eip2718 does not require rlp header
    pub(crate) fn encode_with_signature(
        &self,
        signature: &Signature,
        out: &mut dyn alloy_rlp::BufMut,
    ) {
        let payload_length = self.fields_len() + signature.rlp_vrs_len();
        let header = alloy_rlp::Header { list: true, payload_length };
        header.encode(out);
        self.encode_fields(out);
        signature.write_rlp_vrs(out);
    }

    /// Output the length of the RLP signed transaction encoding, _without_ a RLP string header.
    pub fn payload_len_with_signature_without_header(&self, signature: &Signature) -> usize {
        let payload_length = self.fields_len() + signature.rlp_vrs_len();
        // 'transaction type byte length' + 'header length' + 'payload length'
        1 + length_of_length(payload_length) + payload_length
    }

    /// Output the length of the RLP signed transaction encoding. This encodes with a RLP header.
    pub fn payload_len_with_signature(&self, signature: &Signature) -> usize {
        let len = self.payload_len_with_signature_without_header(signature);
        length_of_length(len) + len
    }

    /// Get transaction type
    pub(crate) const fn tx_type(&self) -> TxType {
        TxType::Eip1559
    }

    /// Calculates a heuristic for the in-memory size of the [DepositTransaction] transaction.
    #[inline]
    pub fn size(&self) -> usize {
        mem::size_of::<B256>() + // source_hash
        mem::size_of::<Address>() + // from
        self.kind.size() + // to
        mem::size_of::<U256>() + // mint
        mem::size_of::<U256>() + // value
        mem::size_of::<U256>() + // gas_limit
        mem::size_of::<bool>() + // is_system_transaction
        self.input.len() // input
    }

    /// Encodes the legacy transaction in RLP for signing.
    pub(crate) fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut) {
        out.put_u8(self.tx_type() as u8);
        alloy_rlp::Header { list: true, payload_length: self.fields_len() }.encode(out);
        self.encode_fields(out);
    }

    /// Outputs the length of the signature RLP encoding for the transaction.
    pub(crate) fn payload_len_for_signature(&self) -> usize {
        let payload_length = self.fields_len();
        // 'transaction type byte length' + 'header length' + 'payload length'
        1 + length_of_length(payload_length) + payload_length
    }

    /// Outputs the signature hash of the transaction by first encoding without a signature, then
    /// hashing.
    pub(crate) fn signature_hash(&self) -> B256 {
        let mut buf = Vec::with_capacity(self.payload_len_for_signature());
        self.encode_for_signing(&mut buf);
        alloy_primitives::utils::keccak256(&buf)
    }
}

impl Transaction for DepositTransactionRequest {
    type Signature = Signature;

    fn chain_id(&self) -> Option<ChainId> {
        None
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit.to::<u64>()
    }

    fn nonce(&self) -> u64 {
        u64::MAX
    }

    fn decode_signed(buf: &mut &[u8]) -> alloy_rlp::Result<alloy_network::Signed<Self>>
    where
        Self: Sized,
    {
        let header = alloy_rlp::Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }

        let tx = Self::decode_inner(buf)?;
        let signature = Signature::decode_rlp_vrs(buf)?;

        Ok(tx.into_signed(signature))
    }

    fn encode_signed(&self, signature: &Signature, out: &mut dyn bytes::BufMut) {
        self.encode_with_signature(signature, out)
    }

    fn gas_price(&self) -> Option<U256> {
        None
    }

    fn input(&self) -> &[u8] {
        &self.input
    }

    fn input_mut(&mut self) -> &mut Bytes {
        &mut self.input
    }

    fn into_signed(self, signature: Signature) -> alloy_network::Signed<Self, Self::Signature>
    where
        Self: Sized,
    {
        alloy_network::Signed::new_unchecked(self.clone(), signature, self.signature_hash())
    }

    fn set_chain_id(&mut self, _chain_id: ChainId) {}

    fn set_gas_limit(&mut self, limit: u64) {
        self.gas_limit = U256::from(limit);
    }

    fn set_gas_price(&mut self, _price: U256) {}

    fn set_input(&mut self, data: Bytes) {
        self.input = data;
    }

    fn set_nonce(&mut self, _nonce: u64) {}

    fn set_to(&mut self, to: TxKind) {
        self.kind = to;
    }

    fn set_value(&mut self, value: U256) {
        self.value = value;
    }

    fn signature_hash(&self) -> B256 {
        self.signature_hash()
    }

    fn to(&self) -> TxKind {
        self.kind
    }

    fn value(&self) -> U256 {
        self.value
    }

    fn encode_for_signing(&self, _out: &mut dyn alloy_rlp::BufMut) {
        todo!()
    }

    fn payload_len_for_signature(&self) -> usize {
        todo!()
    }
}

impl From<DepositTransaction> for DepositTransactionRequest {
    fn from(tx: DepositTransaction) -> Self {
        Self {
            from: tx.from,
            source_hash: tx.source_hash,
            kind: tx.kind,
            mint: tx.mint,
            value: tx.value,
            gas_limit: tx.gas_limit,
            is_system_tx: tx.is_system_tx,
            input: tx.input,
        }
    }
}

impl Encodable for DepositTransactionRequest {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        RlpHeader { list: true, payload_length: self.fields_len() }.encode(out);
        self.encode_fields(out);
    }
}

/// An op-stack deposit transaction.
/// See <https://github.com/ethereum-optimism/optimism/blob/develop/specs/deposits.md#the-deposited-transaction-type>
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DepositTransaction {
    pub nonce: U256,
    pub source_hash: B256,
    pub from: Address,
    pub kind: TxKind,
    pub mint: U256,
    pub value: U256,
    pub gas_limit: U256,
    pub is_system_tx: bool,
    pub input: Bytes,
}

impl DepositTransaction {
    pub fn nonce(&self) -> &U256 {
        &self.nonce
    }

    pub fn hash(&self) -> B256 {
        B256::from_slice(alloy_primitives::keccak256(alloy_rlp::encode(self)).as_slice())
    }

    // /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
        Ok(self.from)
    }

    pub fn chain_id(&self) -> Option<u64> {
        None
    }

    /// Encodes only the transaction's fields into the desired buffer, without a RLP header.
    pub(crate) fn encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.nonce.encode(out);
        self.source_hash.encode(out);
        self.from.encode(out);
        self.kind.encode(out);
        self.mint.encode(out);
        self.value.encode(out);
        self.gas_limit.encode(out);
        self.is_system_tx.encode(out);
        self.input.encode(out);
    }

    /// Calculates the length of the RLP-encoded transaction's fields.
    pub(crate) fn fields_len(&self) -> usize {
        let mut len = 0;
        len += self.source_hash.length();
        len += self.from.length();
        len += self.kind.length();
        len += self.mint.length();
        len += self.value.length();
        len += self.gas_limit.length();
        len += self.is_system_tx.length();
        len += self.input.length();
        len
    }

    /// Decodes the inner [TxDeposit] fields from RLP bytes.
    ///
    /// NOTE: This assumes a RLP header has already been decoded, and _just_ decodes the following
    /// RLP fields in the following order:
    ///
    /// - `source_hash`
    /// - `from`
    /// - `kind`
    /// - `mint`
    /// - `value`
    /// - `gas_limit`
    /// - `is_system_tx`
    /// - `input`
    pub fn decode_inner(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            nonce: U256::ZERO,
            source_hash: Decodable::decode(buf)?,
            from: Decodable::decode(buf)?,
            kind: Decodable::decode(buf)?,
            mint: Decodable::decode(buf)?,
            value: Decodable::decode(buf)?,
            gas_limit: Decodable::decode(buf)?,
            is_system_tx: Decodable::decode(buf)?,
            input: Decodable::decode(buf)?,
        })
    }
}

impl Encodable for DepositTransaction {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        RlpHeader { list: true, payload_length: self.fields_len() }.encode(out);
        self.encode_fields(out);
    }
}

impl Decodable for DepositTransaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let header = RlpHeader::decode(buf)?;
        let remaining_len = buf.len();

        if header.payload_length > remaining_len {
            return Err(alloy_rlp::Error::InputTooShort);
        }

        Self::decode_inner(buf)
    }
}
