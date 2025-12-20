use alloy_consensus::{
    Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom, TxReceipt, Typed2718,
};
use alloy_network::eip2718::{
    Decodable2718, EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID, EIP4844_TX_TYPE_ID, EIP7702_TX_TYPE_ID,
    Eip2718Error, Encodable2718, LEGACY_TX_TYPE_ID,
};
use alloy_primitives::{Bloom, Log, TxHash, logs_bloom};
use alloy_rlp::{BufMut, Decodable, Encodable, Header, bytes};
use alloy_rpc_types::{BlockNumHash, trace::otterscan::OtsReceipt};
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, OpDepositReceipt, OpDepositReceiptWithBloom};
use serde::{Deserialize, Serialize};
use tempo_primitives::TEMPO_TX_TYPE_ID;

use crate::FoundryTxType;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FoundryReceiptEnvelope<T = Log> {
    #[serde(rename = "0x0", alias = "0x00")]
    Legacy(ReceiptWithBloom<Receipt<T>>),
    #[serde(rename = "0x1", alias = "0x01")]
    Eip2930(ReceiptWithBloom<Receipt<T>>),
    #[serde(rename = "0x2", alias = "0x02")]
    Eip1559(ReceiptWithBloom<Receipt<T>>),
    #[serde(rename = "0x3", alias = "0x03")]
    Eip4844(ReceiptWithBloom<Receipt<T>>),
    #[serde(rename = "0x4", alias = "0x04")]
    Eip7702(ReceiptWithBloom<Receipt<T>>),
    #[serde(rename = "0x7E", alias = "0x7e")]
    Deposit(OpDepositReceiptWithBloom<T>),
    #[serde(rename = "0x76")]
    Tempo(ReceiptWithBloom<Receipt<T>>),
}

impl FoundryReceiptEnvelope<alloy_rpc_types::Log> {
    /// Creates a new [`FoundryReceiptEnvelope`] from the given parts.
    pub fn from_parts(
        status: bool,
        cumulative_gas_used: u64,
        logs: impl IntoIterator<Item = alloy_rpc_types::Log>,
        tx_type: FoundryTxType,
        deposit_nonce: Option<u64>,
        deposit_receipt_version: Option<u64>,
    ) -> Self {
        let logs = logs.into_iter().collect::<Vec<_>>();
        let logs_bloom = logs_bloom(logs.iter().map(|l| &l.inner).collect::<Vec<_>>());
        let inner_receipt =
            Receipt { status: Eip658Value::Eip658(status), cumulative_gas_used, logs };
        match tx_type {
            FoundryTxType::Legacy => {
                Self::Legacy(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
            FoundryTxType::Eip2930 => {
                Self::Eip2930(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
            FoundryTxType::Eip1559 => {
                Self::Eip1559(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
            FoundryTxType::Eip4844 => {
                Self::Eip4844(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
            FoundryTxType::Eip7702 => {
                Self::Eip7702(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
            FoundryTxType::Deposit => {
                let inner = OpDepositReceiptWithBloom {
                    receipt: OpDepositReceipt {
                        inner: inner_receipt,
                        deposit_nonce,
                        deposit_receipt_version,
                    },
                    logs_bloom,
                };
                Self::Deposit(inner)
            }
            FoundryTxType::Tempo => {
                Self::Tempo(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
        }
    }
}

impl FoundryReceiptEnvelope<Log> {
    pub fn convert_logs_rpc(
        self,
        block_numhash: BlockNumHash,
        block_timestamp: u64,
        transaction_hash: TxHash,
        transaction_index: u64,
        next_log_index: usize,
    ) -> FoundryReceiptEnvelope<alloy_rpc_types::Log> {
        let logs = self
            .logs()
            .iter()
            .enumerate()
            .map(|(index, log)| alloy_rpc_types::Log {
                inner: log.clone(),
                block_hash: Some(block_numhash.hash),
                block_number: Some(block_numhash.number),
                block_timestamp: Some(block_timestamp),
                transaction_hash: Some(transaction_hash),
                transaction_index: Some(transaction_index),
                log_index: Some((next_log_index + index) as u64),
                removed: false,
            })
            .collect::<Vec<_>>();
        FoundryReceiptEnvelope::<alloy_rpc_types::Log>::from_parts(
            self.status(),
            self.cumulative_gas_used(),
            logs,
            self.tx_type(),
            self.deposit_nonce(),
            self.deposit_receipt_version(),
        )
    }
}

impl<T> FoundryReceiptEnvelope<T> {
    /// Return the [`FoundryTxType`] of the inner receipt.
    pub const fn tx_type(&self) -> FoundryTxType {
        match self {
            Self::Legacy(_) => FoundryTxType::Legacy,
            Self::Eip2930(_) => FoundryTxType::Eip2930,
            Self::Eip1559(_) => FoundryTxType::Eip1559,
            Self::Eip4844(_) => FoundryTxType::Eip4844,
            Self::Eip7702(_) => FoundryTxType::Eip7702,
            Self::Deposit(_) => FoundryTxType::Deposit,
            Self::Tempo(_) => FoundryTxType::Tempo,
        }
    }

    /// Returns the success status of the receipt's transaction.
    pub const fn status(&self) -> bool {
        self.as_receipt().status.coerce_status()
    }

    /// Returns the cumulative gas used at this receipt.
    pub const fn cumulative_gas_used(&self) -> u64 {
        self.as_receipt().cumulative_gas_used
    }

    /// Converts the receipt's log type by applying a function to each log.
    ///
    /// Returns the receipt with the new log type.
    pub fn map_logs<U>(self, f: impl FnMut(T) -> U) -> FoundryReceiptEnvelope<U> {
        match self {
            Self::Legacy(r) => FoundryReceiptEnvelope::Legacy(r.map_logs(f)),
            Self::Eip2930(r) => FoundryReceiptEnvelope::Eip2930(r.map_logs(f)),
            Self::Eip1559(r) => FoundryReceiptEnvelope::Eip1559(r.map_logs(f)),
            Self::Eip4844(r) => FoundryReceiptEnvelope::Eip4844(r.map_logs(f)),
            Self::Eip7702(r) => FoundryReceiptEnvelope::Eip7702(r.map_logs(f)),
            Self::Deposit(r) => FoundryReceiptEnvelope::Deposit(r.map_receipt(|r| r.map_logs(f))),
            Self::Tempo(r) => FoundryReceiptEnvelope::Tempo(r.map_logs(f)),
        }
    }

    /// Return the receipt logs.
    pub fn logs(&self) -> &[T] {
        &self.as_receipt().logs
    }

    /// Consumes the type and returns the logs.
    pub fn into_logs(self) -> Vec<T> {
        self.into_receipt().logs
    }

    /// Return the receipt's bloom.
    pub const fn logs_bloom(&self) -> &Bloom {
        match self {
            Self::Legacy(t) => &t.logs_bloom,
            Self::Eip2930(t) => &t.logs_bloom,
            Self::Eip1559(t) => &t.logs_bloom,
            Self::Eip4844(t) => &t.logs_bloom,
            Self::Eip7702(t) => &t.logs_bloom,
            Self::Deposit(t) => &t.logs_bloom,
            Self::Tempo(t) => &t.logs_bloom,
        }
    }

    /// Return the receipt's deposit_nonce if it is a deposit receipt.
    pub fn deposit_nonce(&self) -> Option<u64> {
        self.as_deposit_receipt().and_then(|r| r.deposit_nonce)
    }

    /// Return the receipt's deposit version if it is a deposit receipt.
    pub fn deposit_receipt_version(&self) -> Option<u64> {
        self.as_deposit_receipt().and_then(|r| r.deposit_receipt_version)
    }

    /// Returns the deposit receipt if it is a deposit receipt.
    pub const fn as_deposit_receipt_with_bloom(&self) -> Option<&OpDepositReceiptWithBloom<T>> {
        match self {
            Self::Deposit(t) => Some(t),
            _ => None,
        }
    }

    /// Returns the deposit receipt if it is a deposit receipt.
    pub const fn as_deposit_receipt(&self) -> Option<&OpDepositReceipt<T>> {
        match self {
            Self::Deposit(t) => Some(&t.receipt),
            _ => None,
        }
    }

    /// Consumes the type and returns the underlying [`Receipt`].
    pub fn into_receipt(self) -> Receipt<T> {
        match self {
            Self::Legacy(t)
            | Self::Eip2930(t)
            | Self::Eip1559(t)
            | Self::Eip4844(t)
            | Self::Eip7702(t)
            | Self::Tempo(t) => t.receipt,
            Self::Deposit(t) => t.receipt.into_inner(),
        }
    }

    /// Return the inner receipt.
    pub const fn as_receipt(&self) -> &Receipt<T> {
        match self {
            Self::Legacy(t)
            | Self::Eip2930(t)
            | Self::Eip1559(t)
            | Self::Eip4844(t)
            | Self::Eip7702(t)
            | Self::Tempo(t) => &t.receipt,
            Self::Deposit(t) => &t.receipt.inner,
        }
    }
}

impl<T> TxReceipt for FoundryReceiptEnvelope<T>
where
    T: Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync,
{
    type Log = T;

    fn status_or_post_state(&self) -> Eip658Value {
        self.as_receipt().status
    }

    fn status(&self) -> bool {
        self.status()
    }

    /// Return the receipt's bloom.
    fn bloom(&self) -> Bloom {
        *self.logs_bloom()
    }

    fn bloom_cheap(&self) -> Option<Bloom> {
        Some(self.bloom())
    }

    /// Returns the cumulative gas used at this receipt.
    fn cumulative_gas_used(&self) -> u64 {
        self.cumulative_gas_used()
    }

    /// Return the receipt logs.
    fn logs(&self) -> &[T] {
        self.logs()
    }
}

impl Encodable for FoundryReceiptEnvelope {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        match self {
            Self::Legacy(r) => r.encode(out),
            receipt => {
                let payload_len = match receipt {
                    Self::Eip2930(r) => r.length() + 1,
                    Self::Eip1559(r) => r.length() + 1,
                    Self::Eip4844(r) => r.length() + 1,
                    Self::Eip7702(r) => r.length() + 1,
                    Self::Deposit(r) => r.length() + 1,
                    Self::Tempo(r) => r.length() + 1,
                    _ => unreachable!("receipt already matched"),
                };

                match receipt {
                    Self::Eip2930(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        EIP2930_TX_TYPE_ID.encode(out);
                        r.encode(out);
                    }
                    Self::Eip1559(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        EIP1559_TX_TYPE_ID.encode(out);
                        r.encode(out);
                    }
                    Self::Eip4844(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        EIP4844_TX_TYPE_ID.encode(out);
                        r.encode(out);
                    }
                    Self::Eip7702(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        EIP7702_TX_TYPE_ID.encode(out);
                        r.encode(out);
                    }
                    Self::Deposit(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        DEPOSIT_TX_TYPE_ID.encode(out);
                        r.encode(out);
                    }
                    Self::Tempo(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        TEMPO_TX_TYPE_ID.encode(out);
                        r.encode(out);
                    }
                    _ => unreachable!("receipt already matched"),
                }
            }
        }
    }
}

impl Decodable for FoundryReceiptEnvelope {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        use bytes::Buf;
        use std::cmp::Ordering;

        // a receipt is either encoded as a string (non legacy) or a list (legacy).
        // We should not consume the buffer if we are decoding a legacy receipt, so let's
        // check if the first byte is between 0x80 and 0xbf.
        let rlp_type = *buf
            .first()
            .ok_or(alloy_rlp::Error::Custom("cannot decode a receipt from empty bytes"))?;

        match rlp_type.cmp(&alloy_rlp::EMPTY_LIST_CODE) {
            Ordering::Less => {
                // strip out the string header
                let _header = Header::decode(buf)?;
                let receipt_type = *buf.first().ok_or(alloy_rlp::Error::Custom(
                    "typed receipt cannot be decoded from an empty slice",
                ))?;
                if receipt_type == EIP2930_TX_TYPE_ID {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf)
                        .map(FoundryReceiptEnvelope::Eip2930)
                } else if receipt_type == EIP1559_TX_TYPE_ID {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf)
                        .map(FoundryReceiptEnvelope::Eip1559)
                } else if receipt_type == EIP4844_TX_TYPE_ID {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf)
                        .map(FoundryReceiptEnvelope::Eip4844)
                } else if receipt_type == EIP7702_TX_TYPE_ID {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf)
                        .map(FoundryReceiptEnvelope::Eip7702)
                } else if receipt_type == DEPOSIT_TX_TYPE_ID {
                    buf.advance(1);
                    <OpDepositReceiptWithBloom as Decodable>::decode(buf)
                        .map(FoundryReceiptEnvelope::Deposit)
                } else if receipt_type == TEMPO_TX_TYPE_ID {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(FoundryReceiptEnvelope::Tempo)
                } else {
                    Err(alloy_rlp::Error::Custom("invalid receipt type"))
                }
            }
            Ordering::Equal => {
                Err(alloy_rlp::Error::Custom("an empty list is not a valid receipt encoding"))
            }
            Ordering::Greater => {
                <ReceiptWithBloom as Decodable>::decode(buf).map(FoundryReceiptEnvelope::Legacy)
            }
        }
    }
}

impl Typed2718 for FoundryReceiptEnvelope {
    fn ty(&self) -> u8 {
        match self {
            Self::Legacy(_) => LEGACY_TX_TYPE_ID,
            Self::Eip2930(_) => EIP2930_TX_TYPE_ID,
            Self::Eip1559(_) => EIP1559_TX_TYPE_ID,
            Self::Eip4844(_) => EIP4844_TX_TYPE_ID,
            Self::Eip7702(_) => EIP7702_TX_TYPE_ID,
            Self::Deposit(_) => DEPOSIT_TX_TYPE_ID,
            Self::Tempo(_) => TEMPO_TX_TYPE_ID,
        }
    }
}

impl Encodable2718 for FoundryReceiptEnvelope {
    fn encode_2718_len(&self) -> usize {
        match self {
            Self::Legacy(r) => ReceiptEnvelope::Legacy(r.clone()).encode_2718_len(),
            Self::Eip2930(r) => ReceiptEnvelope::Eip2930(r.clone()).encode_2718_len(),
            Self::Eip1559(r) => ReceiptEnvelope::Eip1559(r.clone()).encode_2718_len(),
            Self::Eip4844(r) => ReceiptEnvelope::Eip4844(r.clone()).encode_2718_len(),
            Self::Eip7702(r) => 1 + r.length(),
            Self::Deposit(r) => 1 + r.length(),
            Self::Tempo(r) => 1 + r.length(),
        }
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        if let Some(ty) = self.type_flag() {
            out.put_u8(ty);
        }
        match self {
            Self::Legacy(r)
            | Self::Eip2930(r)
            | Self::Eip1559(r)
            | Self::Eip4844(r)
            | Self::Eip7702(r)
            | Self::Tempo(r) => r.encode(out),
            Self::Deposit(r) => r.encode(out),
        }
    }
}

impl Decodable2718 for FoundryReceiptEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        if ty == DEPOSIT_TX_TYPE_ID {
            return Ok(Self::Deposit(OpDepositReceiptWithBloom::decode(buf)?));
        }
        if ty == TEMPO_TX_TYPE_ID {
            return Ok(Self::Tempo(ReceiptWithBloom::decode(buf)?));
        }
        match ReceiptEnvelope::typed_decode(ty, buf)? {
            ReceiptEnvelope::Eip2930(tx) => Ok(Self::Eip2930(tx)),
            ReceiptEnvelope::Eip1559(tx) => Ok(Self::Eip1559(tx)),
            ReceiptEnvelope::Eip4844(tx) => Ok(Self::Eip4844(tx)),
            ReceiptEnvelope::Eip7702(tx) => Ok(Self::Eip7702(tx)),
            _ => Err(Eip2718Error::RlpError(alloy_rlp::Error::Custom("unexpected tx type"))),
        }
    }

    fn fallback_decode(buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        match ReceiptEnvelope::fallback_decode(buf)? {
            ReceiptEnvelope::Legacy(tx) => Ok(Self::Legacy(tx)),
            _ => Err(Eip2718Error::RlpError(alloy_rlp::Error::Custom("unexpected tx type"))),
        }
    }
}

impl From<FoundryReceiptEnvelope<alloy_rpc_types::Log>> for OtsReceipt {
    fn from(receipt: FoundryReceiptEnvelope<alloy_rpc_types::Log>) -> Self {
        Self {
            status: receipt.status(),
            cumulative_gas_used: receipt.cumulative_gas_used(),
            logs: Some(receipt.logs().to_vec()),
            logs_bloom: Some(receipt.logs_bloom().to_owned()),
            r#type: receipt.tx_type() as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256, Bytes, LogData, hex};
    use std::str::FromStr;

    #[test]
    fn encode_legacy_receipt() {
        let expected = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let mut data = vec![];
        let receipt = FoundryReceiptEnvelope::Legacy(ReceiptWithBloom {
            receipt: Receipt {
                status: false.into(),
                cumulative_gas_used: 0x1,
                logs: vec![Log {
                    address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000dead",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000beef",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str("0100ff").unwrap(),
                    ),
                }],
            },
            logs_bloom: [0; 256].into(),
        });

        receipt.encode(&mut data);

        // check that the rlp length equals the length of the expected rlp
        assert_eq!(receipt.length(), expected.len());
        assert_eq!(data, expected);
    }

    #[test]
    fn decode_legacy_receipt() {
        let data = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let expected = FoundryReceiptEnvelope::Legacy(ReceiptWithBloom {
            receipt: Receipt {
                status: false.into(),
                cumulative_gas_used: 0x1,
                logs: vec![Log {
                    address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000dead",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000beef",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str("0100ff").unwrap(),
                    ),
                }],
            },
            logs_bloom: [0; 256].into(),
        });

        let receipt = FoundryReceiptEnvelope::decode(&mut &data[..]).unwrap();

        assert_eq!(receipt, expected);
    }

    #[test]
    fn encode_tempo_receipt() {
        use alloy_network::eip2718::Encodable2718;
        use tempo_primitives::TEMPO_TX_TYPE_ID;

        let receipt = FoundryReceiptEnvelope::Tempo(ReceiptWithBloom {
            receipt: Receipt {
                status: true.into(),
                cumulative_gas_used: 157716,
                logs: vec![Log {
                    address: Address::from_str("20c0000000000000000000000000000000000000").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000566ff0f4a6114f8072ecdc8a7a8a13d8d0c6b45f",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000dec0000000000000000000000000000000000000",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str(
                            "0000000000000000000000000000000000000000000000000000000000989680",
                        )
                        .unwrap(),
                    ),
                }],
            },
            logs_bloom: [0; 256].into(),
        });

        assert_eq!(receipt.tx_type(), FoundryTxType::Tempo);
        assert_eq!(receipt.ty(), TEMPO_TX_TYPE_ID);
        assert!(receipt.status());
        assert_eq!(receipt.cumulative_gas_used(), 157716);
        assert_eq!(receipt.logs().len(), 1);

        // Encode and decode round-trip
        let mut encoded = Vec::new();
        receipt.encode_2718(&mut encoded);

        // First byte should be the Tempo type ID
        assert_eq!(encoded[0], TEMPO_TX_TYPE_ID);

        // Decode it back
        let decoded = FoundryReceiptEnvelope::decode(&mut &encoded[..]).unwrap();
        assert_eq!(receipt, decoded);
    }

    #[test]
    fn decode_tempo_receipt() {
        use alloy_network::eip2718::Encodable2718;
        use tempo_primitives::TEMPO_TX_TYPE_ID;

        let receipt = FoundryReceiptEnvelope::Tempo(ReceiptWithBloom {
            receipt: Receipt { status: true.into(), cumulative_gas_used: 21000, logs: vec![] },
            logs_bloom: [0; 256].into(),
        });

        // Encode and decode via 2718
        let mut encoded = Vec::new();
        receipt.encode_2718(&mut encoded);
        assert_eq!(encoded[0], TEMPO_TX_TYPE_ID);

        use alloy_network::eip2718::Decodable2718;
        let decoded = FoundryReceiptEnvelope::decode_2718(&mut &encoded[..]).unwrap();
        assert_eq!(receipt, decoded);
    }

    #[test]
    fn tempo_receipt_from_parts() {
        let receipt = FoundryReceiptEnvelope::<alloy_rpc_types::Log>::from_parts(
            true,
            100000,
            vec![],
            FoundryTxType::Tempo,
            None,
            None,
        );

        assert_eq!(receipt.tx_type(), FoundryTxType::Tempo);
        assert!(receipt.status());
        assert_eq!(receipt.cumulative_gas_used(), 100000);
        assert!(receipt.logs().is_empty());
        assert!(receipt.deposit_nonce().is_none());
        assert!(receipt.deposit_receipt_version().is_none());
    }

    #[test]
    fn tempo_receipt_map_logs() {
        let receipt = FoundryReceiptEnvelope::Tempo(ReceiptWithBloom {
            receipt: Receipt {
                status: true.into(),
                cumulative_gas_used: 21000,
                logs: vec![Log {
                    address: Address::from_str("20c0000000000000000000000000000000000000").unwrap(),
                    data: LogData::new_unchecked(vec![], Bytes::default()),
                }],
            },
            logs_bloom: [0; 256].into(),
        });

        // Map logs to a different type (just clone in this case)
        let mapped = receipt.map_logs(|log| log);
        assert_eq!(mapped.logs().len(), 1);
        assert_eq!(mapped.tx_type(), FoundryTxType::Tempo);
    }
}
