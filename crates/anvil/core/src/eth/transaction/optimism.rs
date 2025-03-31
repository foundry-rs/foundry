use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use alloy_rlp::{Decodable, Encodable, Error as DecodeError, Header as RlpHeader};
use op_alloy_consensus::TxDeposit;
use serde::{Deserialize, Serialize};

pub const DEPOSIT_TX_TYPE_ID: u8 = 0x7E;

impl From<DepositTransaction> for TxDeposit {
    fn from(tx: DepositTransaction) -> Self {
        Self {
            from: tx.from,
            source_hash: tx.source_hash,
            to: tx.kind,
            mint: Some(tx.mint.to::<u128>()),
            value: tx.value,
            gas_limit: tx.gas_limit,
            is_system_transaction: tx.is_system_tx,
            input: tx.input,
        }
    }
}

/// An op-stack deposit transaction.
/// See <https://github.com/ethereum-optimism/optimistic-specs/blob/main/specs/deposits.md#the-deposited-transaction-type>
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DepositTransaction {
    pub nonce: u64,
    pub source_hash: B256,
    pub from: Address,
    pub kind: TxKind,
    pub mint: U256,
    pub value: U256,
    pub gas_limit: u64,
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
}
