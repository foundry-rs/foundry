use alloy_primitives::{Address, BlockHash, LogData, TxHash, B256};

/// Ethereum Log emitted by a transaction
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Log<T = LogData> {
    #[cfg_attr(feature = "serde", serde(flatten))]
    /// Consensus log object
    pub inner: alloy_primitives::Log<T>,
    /// Hash of the block the transaction that emitted this log was mined in
    pub block_hash: Option<BlockHash>,
    /// Number of the block the transaction that emitted this log was mined in
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity::opt"))]
    pub block_number: Option<u64>,
    /// The timestamp of the block as proposed in:
    /// <https://ethereum-magicians.org/t/proposal-for-adding-blocktimestamp-to-logs-object-returned-by-eth-getlogs-and-related-requests>
    /// <https://github.com/ethereum/execution-apis/issues/295>
    #[cfg_attr(
        feature = "serde",
        serde(
            skip_serializing_if = "Option::is_none",
            with = "alloy_serde::quantity::opt",
            default
        )
    )]
    pub block_timestamp: Option<u64>,
    /// Transaction Hash
    #[doc(alias = "tx_hash")]
    pub transaction_hash: Option<TxHash>,
    /// Index of the Transaction in the block
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity::opt"))]
    #[doc(alias = "tx_index")]
    pub transaction_index: Option<u64>,
    /// Log Index in Block
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity::opt"))]
    pub log_index: Option<u64>,
    /// Geth Compatibility Field: whether this log was removed
    #[cfg_attr(feature = "serde", serde(default))]
    pub removed: bool,
}

impl<T> Log<T> {
    /// Getter for the address field. Shortcut for `log.inner.address`.
    pub const fn address(&self) -> Address {
        self.inner.address
    }

    /// Getter for the data field. Shortcut for `log.inner.data`.
    pub const fn data(&self) -> &T {
        &self.inner.data
    }

    /// Consumes the type and returns the wrapped [`alloy_primitives::Log`]
    pub fn into_inner(self) -> alloy_primitives::Log<T> {
        self.inner
    }
}

impl Log<LogData> {
    /// Getter for the topics field. Shortcut for `log.inner.topics()`.
    pub fn topics(&self) -> &[B256] {
        self.inner.topics()
    }

    /// Getter for the topic0 field.
    #[doc(alias = "event_signature")]
    pub fn topic0(&self) -> Option<&B256> {
        self.inner.topics().first()
    }

    /// Get the topic list, mutably. This gives access to the internal
    /// array, without allowing extension of that array. Shortcut for
    /// [`LogData::topics_mut`]
    pub fn topics_mut(&mut self) -> &mut [B256] {
        self.inner.data.topics_mut()
    }

    /// Decode the log data into a typed log.
    pub fn log_decode<T: alloy_sol_types::SolEvent>(&self) -> alloy_sol_types::Result<Log<T>> {
        let decoded = T::decode_log(&self.inner, false)?;
        Ok(Log {
            inner: decoded,
            block_hash: self.block_hash,
            block_number: self.block_number,
            block_timestamp: self.block_timestamp,
            transaction_hash: self.transaction_hash,
            transaction_index: self.transaction_index,
            log_index: self.log_index,
            removed: self.removed,
        })
    }
}

impl<T> alloy_rlp::Encodable for Log<T>
where
    for<'a> &'a T: Into<LogData>,
{
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.reserialize_inner().encode(out)
    }

    fn length(&self) -> usize {
        self.reserialize_inner().length()
    }
}

impl<T> Log<T>
where
    for<'a> &'a T: Into<LogData>,
{
    /// Reserialize the inner data, returning an [`alloy_primitives::Log`].
    pub fn reserialize_inner(&self) -> alloy_primitives::Log {
        alloy_primitives::Log { address: self.inner.address, data: (&self.inner.data).into() }
    }

    /// Reserialize the data, returning a new `Log` object wrapping an
    /// [`alloy_primitives::Log`]. this copies the log metadata, preserving
    /// the original object.
    pub fn reserialize(&self) -> Log<LogData> {
        Log {
            inner: self.reserialize_inner(),
            block_hash: self.block_hash,
            block_number: self.block_number,
            block_timestamp: self.block_timestamp,
            transaction_hash: self.transaction_hash,
            transaction_index: self.transaction_index,
            log_index: self.log_index,
            removed: self.removed,
        }
    }
}

impl<T> AsRef<alloy_primitives::Log<T>> for Log<T> {
    fn as_ref(&self) -> &alloy_primitives::Log<T> {
        &self.inner
    }
}

impl<T> AsMut<alloy_primitives::Log<T>> for Log<T> {
    fn as_mut(&mut self) -> &mut alloy_primitives::Log<T> {
        &mut self.inner
    }
}

impl<T> AsRef<T> for Log<T> {
    fn as_ref(&self) -> &T {
        &self.inner.data
    }
}

impl<T> AsMut<T> for Log<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.inner.data
    }
}

impl<L> From<Log<L>> for alloy_primitives::Log<L> {
    fn from(value: Log<L>) -> Self {
        value.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Receipt, ReceiptWithBloom, TxReceipt};
    use alloy_primitives::{Address, Bytes};
    use arbitrary::Arbitrary;
    use rand::Rng;
    use similar_asserts::assert_eq;

    const fn assert_tx_receipt<T: TxReceipt>() {}

    #[test]
    const fn assert_receipt() {
        assert_tx_receipt::<ReceiptWithBloom<Receipt<Log>>>();
    }

    #[test]
    fn log_arbitrary() {
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());

        let _: Log = Log::arbitrary(&mut arbitrary::Unstructured::new(&bytes)).unwrap();
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_log() {
        let mut log = Log {
            inner: alloy_primitives::Log {
                address: Address::with_last_byte(0x69),
                data: alloy_primitives::LogData::new_unchecked(
                    vec![B256::with_last_byte(0x69)],
                    Bytes::from_static(&[0x69]),
                ),
            },
            block_hash: Some(B256::with_last_byte(0x69)),
            block_number: Some(0x69),
            block_timestamp: None,
            transaction_hash: Some(B256::with_last_byte(0x69)),
            transaction_index: Some(0x69),
            log_index: Some(0x69),
            removed: false,
        };
        let serialized = serde_json::to_string(&log).unwrap();
        assert_eq!(
            serialized,
            r#"{"address":"0x0000000000000000000000000000000000000069","topics":["0x0000000000000000000000000000000000000000000000000000000000000069"],"data":"0x69","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000069","blockNumber":"0x69","transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000069","transactionIndex":"0x69","logIndex":"0x69","removed":false}"#
        );

        let deserialized: Log = serde_json::from_str(&serialized).unwrap();
        assert_eq!(log, deserialized);

        log.block_timestamp = Some(0x69);
        let serialized = serde_json::to_string(&log).unwrap();
        assert_eq!(
            serialized,
            r#"{"address":"0x0000000000000000000000000000000000000069","topics":["0x0000000000000000000000000000000000000000000000000000000000000069"],"data":"0x69","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000069","blockNumber":"0x69","blockTimestamp":"0x69","transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000069","transactionIndex":"0x69","logIndex":"0x69","removed":false}"#
        );

        let deserialized: Log = serde_json::from_str(&serialized).unwrap();
        assert_eq!(log, deserialized);
    }
}
