use alloy_network::TxKind;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rlp::{Decodable, Encodable, Error as DecodeError, Header as RlpHeader};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositTransactionRequest {
    pub from: Address,
    pub source_hash: B256,
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
        len += self.nonce.length();
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
    /// - `nonce`
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
            nonce: Decodable::decode(buf)?,
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
