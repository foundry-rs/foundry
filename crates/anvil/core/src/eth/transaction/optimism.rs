use alloy_consensus::{SignableTransaction, Signed, Transaction};
use alloy_primitives::{keccak256, Address, Bytes, ChainId, Signature, TxKind, B256, U256};
use alloy_rlp::{
    length_of_length, Decodable, Encodable, Error as DecodeError, Header as RlpHeader,
};
use bytes::BufMut;
use serde::{Deserialize, Serialize};
use std::mem;

pub const DEPOSIT_TX_TYPE_ID: u8 = 0x7E;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositTransactionRequest {
    pub source_hash: B256,
    pub from: Address,
    pub kind: TxKind,
    pub mint: U256,
    pub value: U256,
    pub gas_limit: u128,
    pub is_system_tx: bool,
    pub input: Bytes,
}

impl DepositTransactionRequest {
    pub fn hash(&self) -> B256 {
        let mut encoded = Vec::new();
        encoded.put_u8(DEPOSIT_TX_TYPE_ID);
        self.encode(&mut encoded);

        B256::from_slice(alloy_primitives::keccak256(encoded).as_slice())
    }

    /// Encodes only the transaction's fields into the desired buffer, without a RLP header.
    pub(crate) fn encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
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
    pub(crate) const fn tx_type(&self) -> u8 {
        DEPOSIT_TX_TYPE_ID
    }

    /// Calculates a heuristic for the in-memory size of the [DepositTransaction] transaction.
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
        out.put_u8(self.tx_type());
        alloy_rlp::Header { list: true, payload_length: self.fields_len() }.encode(out);
        self.encode_fields(out);
    }

    /// Outputs the length of the signature RLP encoding for the transaction.
    pub(crate) fn payload_len_for_signature(&self) -> usize {
        let payload_length = self.fields_len();
        // 'transaction type byte length' + 'header length' + 'payload length'
        1 + length_of_length(payload_length) + payload_length
    }

    fn encoded_len_with_signature(&self, signature: &Signature) -> usize {
        // this counts the tx fields and signature fields
        let payload_length = self.fields_len() + signature.rlp_vrs_len();

        // this counts:
        // * tx type byte
        // * inner header length
        // * inner payload length
        1 + alloy_rlp::Header { list: true, payload_length }.length() + payload_length
    }
}

impl Transaction for DepositTransactionRequest {
    fn input(&self) -> &[u8] {
        &self.input
    }

    /// Get `to`.
    fn to(&self) -> TxKind {
        self.kind
    }

    /// Get `value`.
    fn value(&self) -> U256 {
        self.value
    }

    /// Get `chain_id`.
    fn chain_id(&self) -> Option<ChainId> {
        None
    }

    /// Get `nonce`.
    fn nonce(&self) -> u64 {
        u64::MAX
    }

    /// Get `gas_limit`.
    fn gas_limit(&self) -> u128 {
        self.gas_limit
    }

    /// Get `gas_price`.
    fn gas_price(&self) -> Option<u128> {
        None
    }
}

impl SignableTransaction<Signature> for DepositTransactionRequest {
    fn set_chain_id(&mut self, _chain_id: ChainId) {}

    fn payload_len_for_signature(&self) -> usize {
        self.payload_len_for_signature()
    }

    fn into_signed(self, signature: Signature) -> Signed<Self> {
        let mut buf = Vec::with_capacity(self.encoded_len_with_signature(&signature));
        self.encode_with_signature(&signature, &mut buf);
        let hash = keccak256(&buf);

        // Drop any v chain id value to ensure the signature format is correct at the time of
        // combination for an EIP-4844 transaction. V should indicate the y-parity of the
        // signature.
        Signed::new_unchecked(self, signature.with_parity_bool(), hash)
    }

    fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.encode_for_signing(out);
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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DepositTransaction {
    pub nonce: u64,
    pub source_hash: B256,
    pub from: Address,
    pub kind: TxKind,
    pub mint: U256,
    pub value: U256,
    pub gas_limit: u128,
    pub is_system_tx: bool,
    pub input: Bytes,
}

impl DepositTransaction {
    pub fn nonce(&self) -> &u64 {
        &self.nonce
    }

    pub fn hash(&self) -> B256 {
        let mut encoded = Vec::new();
        self.encode_2718(&mut encoded);
        B256::from_slice(alloy_primitives::keccak256(encoded).as_slice())
    }

    // /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
        Ok(self.from)
    }

    pub fn chain_id(&self) -> Option<u64> {
        None
    }

    pub fn encode_2718(&self, out: &mut dyn alloy_rlp::BufMut) {
        out.put_u8(DEPOSIT_TX_TYPE_ID);
        self.encode(out);
    }

    /// Encodes only the transaction's fields into the desired buffer, without a RLP header.
    pub(crate) fn encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
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

    pub fn decode_2718(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        use bytes::Buf;

        let tx_type = *buf.first().ok_or(alloy_rlp::Error::Custom("empty slice"))?;

        if tx_type != DEPOSIT_TX_TYPE_ID {
            return Err(alloy_rlp::Error::Custom("invalid tx type: expected deposit tx type"));
        }

        // Skip the tx type byte
        buf.advance(1);
        Self::decode(buf)
    }

    /// Decodes the inner fields from RLP bytes
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
            nonce: 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let tx = DepositTransaction {
            nonce: 0,
            source_hash: B256::default(),
            from: Address::default(),
            kind: TxKind::Call(Address::default()),
            mint: U256::from(100),
            value: U256::from(100),
            gas_limit: 50000,
            is_system_tx: false,
            input: Bytes::default(),
        };

        let encoded_tx: Vec<u8> = alloy_rlp::encode(&tx);

        let decoded_tx = DepositTransaction::decode(&mut encoded_tx.as_slice()).unwrap();

        assert_eq!(tx, decoded_tx);
    }
    #[test]
    fn test_encode_decode_2718() {
        let tx = DepositTransaction {
            nonce: 0,
            source_hash: B256::default(),
            from: Address::default(),
            kind: TxKind::Call(Address::default()),
            mint: U256::from(100),
            value: U256::from(100),
            gas_limit: 50000,
            is_system_tx: false,
            input: Bytes::default(),
        };

        let mut encoded_tx: Vec<u8> = Vec::new();
        tx.encode_2718(&mut encoded_tx);

        let decoded_tx = DepositTransaction::decode_2718(&mut encoded_tx.as_slice()).unwrap();

        assert_eq!(tx, decoded_tx);
    }

    #[test]
    fn test_tx_request_hash_equals_tx_hash() {
        let tx = DepositTransaction {
            nonce: 0,
            source_hash: B256::default(),
            from: Address::default(),
            kind: TxKind::Call(Address::default()),
            mint: U256::from(100),
            value: U256::from(100),
            gas_limit: 50000,
            is_system_tx: false,
            input: Bytes::default(),
        };

        let tx_request = DepositTransactionRequest::from(tx.clone());

        assert_eq!(tx.hash(), tx_request.hash());
    }
}
