use crate::{SignableTransaction, Signed};
use alloc::vec::Vec;
use alloy_eips::eip2718::{Eip2718Error, Eip2718Result};
use alloy_primitives::{keccak256, PrimitiveSignature as Signature, TxHash};
use alloy_rlp::{Buf, BufMut, Decodable, Encodable, Header};

/// Helper trait for managing RLP encoding of transactions inside 2718
/// envelopes.
#[doc(hidden)]
pub trait RlpEcdsaTx: SignableTransaction<Signature> + Sized {
    /// The default transaction type for this transaction.
    const DEFAULT_TX_TYPE: u8;

    /// Calculate the encoded length of the transaction's fields, without a RLP
    /// header.
    fn rlp_encoded_fields_length(&self) -> usize;

    /// Encodes only the transaction's fields into the desired buffer, without
    /// a RLP header.
    fn rlp_encode_fields(&self, out: &mut dyn alloy_rlp::BufMut);

    /// Create an list rlp header for the unsigned transaction.
    fn rlp_header(&self) -> Header {
        Header { list: true, payload_length: self.rlp_encoded_fields_length() }
    }

    /// Get the length of the transaction when RLP encoded.
    fn rlp_encoded_length(&self) -> usize {
        self.rlp_header().length_with_payload()
    }

    /// RLP encodes the transaction.
    fn rlp_encode(&self, out: &mut dyn BufMut) {
        self.rlp_header().encode(out);
        self.rlp_encode_fields(out);
    }

    /// Create an rlp list header for the signed transaction.
    fn rlp_header_signed(&self, signature: &Signature) -> Header {
        let payload_length =
            self.rlp_encoded_fields_length() + signature.rlp_rs_len() + signature.v().length();
        Header { list: true, payload_length }
    }

    /// Get the length of the transaction when RLP encoded with the given
    /// signature.
    fn rlp_encoded_length_with_signature(&self, signature: &Signature) -> usize {
        self.rlp_header_signed(signature).length_with_payload()
    }

    /// RLP encodes the transaction with the given signature.
    fn rlp_encode_signed(&self, signature: &Signature, out: &mut dyn BufMut) {
        self.rlp_header_signed(signature).encode(out);
        self.rlp_encode_fields(out);
        signature.write_rlp_vrs(out, signature.v());
    }

    /// Get the length of the transaction when EIP-2718 encoded. This is the
    /// 1 byte type flag + the length of the RLP encoded transaction.
    fn eip2718_encoded_length(&self, signature: &Signature) -> usize {
        self.rlp_encoded_length_with_signature(signature) + 1
    }

    /// EIP-2718 encode the transaction with the given signature and type flag.
    fn eip2718_encode_with_type(&self, signature: &Signature, ty: u8, out: &mut dyn BufMut) {
        out.put_u8(ty);
        self.rlp_encode_signed(signature, out);
    }

    /// EIP-2718 encode the transaction with the given signature and the default
    /// type flag.
    fn eip2718_encode(&self, signature: &Signature, out: &mut dyn BufMut) {
        self.eip2718_encode_with_type(signature, Self::DEFAULT_TX_TYPE, out);
    }

    /// Create an rlp header for the network encoded transaction. This will
    /// usually be a string header, however, legacy transactions' network
    /// encoding is a list.
    fn network_header(&self, signature: &Signature) -> Header {
        let payload_length = self.eip2718_encoded_length(signature);
        Header { list: false, payload_length }
    }

    /// Get the length of the transaction when network encoded. This is the
    /// EIP-2718 encoded length with an outer RLP header.
    fn network_encoded_length(&self, signature: &Signature) -> usize {
        self.network_header(signature).length_with_payload()
    }

    /// Network encode the transaction with the given signature.
    fn network_encode_with_type(&self, signature: &Signature, ty: u8, out: &mut dyn BufMut) {
        self.network_header(signature).encode(out);
        self.eip2718_encode_with_type(signature, ty, out);
    }

    /// Network encode the transaction with the given signature and the default
    /// type flag.
    fn network_encode(&self, signature: &Signature, out: &mut dyn BufMut) {
        self.network_encode_with_type(signature, Self::DEFAULT_TX_TYPE, out);
    }

    /// Decodes the fields of the transaction from RLP bytes. Do not decode a
    /// header. You may assume the buffer is long enough to contain the
    /// transaction.
    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self>;

    /// Decodes the transaction from RLP bytes.
    fn rlp_decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }
        let remaining = buf.len();

        if header.payload_length > remaining {
            return Err(alloy_rlp::Error::InputTooShort);
        }

        let this = Self::rlp_decode_fields(buf)?;

        if buf.len() + header.payload_length != remaining {
            return Err(alloy_rlp::Error::UnexpectedLength);
        }

        Ok(this)
    }

    /// Decodes the transaction from RLP bytes, including the signature.
    fn rlp_decode_with_signature(buf: &mut &[u8]) -> alloy_rlp::Result<(Self, Signature)> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }

        let remaining = buf.len();
        let tx = Self::rlp_decode_fields(buf)?;
        let signature = Signature::decode_rlp_vrs(buf, bool::decode)?;

        if buf.len() + header.payload_length != remaining {
            return Err(alloy_rlp::Error::ListLengthMismatch {
                expected: header.payload_length,
                got: remaining - buf.len(),
            });
        }

        Ok((tx, signature))
    }

    /// Decodes the transaction from RLP bytes, including the signature
    /// Produces a [`Signed`].
    fn rlp_decode_signed(buf: &mut &[u8]) -> alloy_rlp::Result<Signed<Self>> {
        Self::rlp_decode_with_signature(buf).map(|(tx, signature)| tx.into_signed(signature))
    }

    /// Decodes the transaction from eip2718 bytes, expecting the given type
    /// flag.
    fn eip2718_decode_with_type(buf: &mut &[u8], ty: u8) -> Eip2718Result<Signed<Self>> {
        let original_buf = *buf;

        if buf.remaining() < 1 {
            return Err(alloy_rlp::Error::InputTooShort.into());
        }
        let actual = buf.get_u8();
        if actual != ty {
            return Err(Eip2718Error::UnexpectedType(actual));
        }

        // OPT: We avoid re-serializing by calculating the hash directly
        // from the original buffer contents.
        let (tx, signature) = Self::rlp_decode_with_signature(buf)?;
        let total_len = tx.eip2718_encoded_length(&signature);
        let hash = keccak256(&original_buf[..total_len]);

        Ok(Signed::new_unchecked(tx, signature, hash))
    }

    /// Decodes the transaction from eip2718 bytes, expecting the default type
    /// flag.
    fn eip2718_decode(buf: &mut &[u8]) -> Eip2718Result<Signed<Self>> {
        Self::eip2718_decode_with_type(buf, Self::DEFAULT_TX_TYPE)
    }

    /// Decodes the transaction from network bytes.
    fn network_decode_with_type(buf: &mut &[u8], ty: u8) -> Eip2718Result<Signed<Self>> {
        let header = Header::decode(buf)?;
        if header.list {
            return Err(alloy_rlp::Error::UnexpectedList.into());
        }

        let remaining = buf.len();
        let res = Self::eip2718_decode_with_type(buf, ty)?;

        if buf.len() + header.payload_length != remaining {
            return Err(alloy_rlp::Error::UnexpectedLength.into());
        }

        Ok(res)
    }

    /// Decodes the transaction from network bytes, expecting the default type
    /// flag.
    fn network_decode(buf: &mut &[u8]) -> Eip2718Result<Signed<Self>> {
        Self::network_decode_with_type(buf, Self::DEFAULT_TX_TYPE)
    }

    /// Calculate the transaction hash for the given signature and type.
    fn tx_hash_with_type(&self, signature: &Signature, ty: u8) -> TxHash {
        let mut buf = Vec::with_capacity(self.eip2718_encoded_length(signature));
        self.eip2718_encode_with_type(signature, ty, &mut buf);
        keccak256(&buf)
    }

    /// Calculate the transaction hash for the given signature.
    fn tx_hash(&self, signature: &Signature) -> TxHash {
        self.tx_hash_with_type(signature, Self::DEFAULT_TX_TYPE)
    }
}
