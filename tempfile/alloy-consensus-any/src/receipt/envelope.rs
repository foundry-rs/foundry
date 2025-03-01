use alloy_consensus::{Eip658Value, Receipt, ReceiptWithBloom, TxReceipt};
use alloy_eips::{
    eip2718::{Decodable2718, Eip2718Result, Encodable2718},
    Typed2718,
};
use alloy_primitives::{bytes::BufMut, Bloom, Log};
use alloy_rlp::{Decodable, Encodable};
use core::fmt;

/// Receipt envelope, as defined in [EIP-2718].
///
/// This enum distinguishes between tagged and untagged legacy receipts, as the
/// in-protocol Merkle tree may commit to EITHER 0-prefixed or raw. Therefore
/// we must ensure that encoding returns the precise byte-array that was
/// decoded, preserving the presence or absence of the `TransactionType` flag.
///
/// Transaction receipt payloads are specified in their respective EIPs.
///
/// [EIP-2718]: https://eips.ethereum.org/EIPS/eip-2718
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[doc(alias = "AnyTransactionReceiptEnvelope", alias = "AnyTxReceiptEnvelope")]
pub struct AnyReceiptEnvelope<T = Log> {
    /// The receipt envelope.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub inner: ReceiptWithBloom<Receipt<T>>,
    /// The transaction type.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub r#type: u8,
}

impl<T> AnyReceiptEnvelope<T> {
    /// Returns whether this is a legacy receipt (type 0)
    pub const fn is_legacy(&self) -> bool {
        self.r#type == 0
    }
}

impl<T: Encodable> AnyReceiptEnvelope<T> {
    /// Calculate the length of the rlp payload of the network encoded receipt.
    pub fn rlp_payload_length(&self) -> usize {
        let length = self.inner.length();
        if self.is_legacy() {
            length
        } else {
            length + 1
        }
    }
}

impl<T> AnyReceiptEnvelope<T> {
    /// Return true if the transaction was successful.
    ///
    /// ## Note
    ///
    /// This method may not accurately reflect the status of the transaction
    /// for transactions before [EIP-658].
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    pub const fn is_success(&self) -> bool {
        self.status()
    }

    /// Returns the success status of the receipt's transaction.
    ///
    /// ## Note
    ///
    /// This method may not accurately reflect the status of the transaction
    /// for transactions before [EIP-658].
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    pub const fn status(&self) -> bool {
        self.inner.receipt.status.coerce_status()
    }

    /// Return the receipt's bloom.
    pub const fn bloom(&self) -> Bloom {
        self.inner.logs_bloom
    }

    /// Returns the cumulative gas used at this receipt.
    pub const fn cumulative_gas_used(&self) -> u64 {
        self.inner.receipt.cumulative_gas_used
    }

    /// Return the receipt logs.
    pub fn logs(&self) -> &[T] {
        &self.inner.receipt.logs
    }
}

impl<T> TxReceipt for AnyReceiptEnvelope<T>
where
    T: Clone + fmt::Debug + PartialEq + Eq + Send + Sync,
{
    type Log = T;

    fn status_or_post_state(&self) -> Eip658Value {
        self.inner.receipt.status
    }

    fn status(&self) -> bool {
        self.status()
    }

    fn bloom(&self) -> Bloom {
        self.bloom()
    }

    fn cumulative_gas_used(&self) -> u64 {
        self.cumulative_gas_used()
    }

    fn logs(&self) -> &[T] {
        self.logs()
    }
}

impl Typed2718 for AnyReceiptEnvelope {
    fn ty(&self) -> u8 {
        self.r#type
    }
}

impl Encodable2718 for AnyReceiptEnvelope {
    fn encode_2718_len(&self) -> usize {
        self.inner.length() + !self.is_legacy() as usize
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        match self.type_flag() {
            None => {}
            Some(ty) => out.put_u8(ty),
        }
        self.inner.encode(out);
    }
}

impl Decodable2718 for AnyReceiptEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Eip2718Result<Self> {
        let receipt = Decodable::decode(buf)?;
        Ok(Self { inner: receipt, r#type: ty })
    }

    fn fallback_decode(buf: &mut &[u8]) -> Eip2718Result<Self> {
        Self::typed_decode(0, buf)
    }
}
