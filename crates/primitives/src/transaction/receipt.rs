use alloy_consensus::{
    Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom, TxReceipt, Typed2718,
};
use alloy_network::eip2718::{
    Decodable2718, EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID, EIP4844_TX_TYPE_ID, EIP7702_TX_TYPE_ID,
    Eip2718Error, Encodable2718, LEGACY_TX_TYPE_ID,
};
use alloy_primitives::{Bloom, Log, TxHash, logs_bloom};
use alloy_rlp::{BufMut, Decodable, Encodable, bytes};
use alloy_rpc_types::{BlockNumHash, trace::otterscan::OtsReceipt};
#[cfg(feature = "optimism")]
use op_alloy_consensus::{
    DEPOSIT_TX_TYPE_ID, OpDepositReceipt, OpDepositReceiptWithBloom, POST_EXEC_TX_TYPE_ID,
};
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
    #[cfg(feature = "optimism")]
    #[serde(rename = "0x7D", alias = "0x7d")]
    PostExec(ReceiptWithBloom<Receipt<T>>),
    #[cfg(feature = "optimism")]
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
        #[cfg_attr(not(feature = "optimism"), allow(unused_variables))] deposit_nonce: Option<u64>,
        #[cfg_attr(not(feature = "optimism"), allow(unused_variables))]
        deposit_receipt_version: Option<u64>,
    ) -> Self {
        let logs = logs.into_iter().collect::<Vec<_>>();
        let logs_bloom = logs_bloom(logs.iter().map(|l| &l.inner));
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
            #[cfg(feature = "optimism")]
            FoundryTxType::PostExec => {
                Self::PostExec(ReceiptWithBloom { receipt: inner_receipt, logs_bloom })
            }
            #[cfg(feature = "optimism")]
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
        let mut index = 0;
        self.map_logs(|inner| {
            let log = alloy_rpc_types::Log {
                inner,
                block_hash: Some(block_numhash.hash),
                block_number: Some(block_numhash.number),
                block_timestamp: Some(block_timestamp),
                transaction_hash: Some(transaction_hash),
                transaction_index: Some(transaction_index),
                log_index: Some((next_log_index + index) as u64),
                removed: false,
            };
            index += 1;
            log
        })
    }
}

impl<T> FoundryReceiptEnvelope<T> {
    /// Returns `true` if this is a legacy receipt.
    pub const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    /// Returns `true` if this is an EIP-2930 receipt.
    pub const fn is_eip2930(&self) -> bool {
        matches!(self, Self::Eip2930(_))
    }

    /// Returns `true` if this is an EIP-1559 receipt.
    pub const fn is_eip1559(&self) -> bool {
        matches!(self, Self::Eip1559(_))
    }

    /// Returns `true` if this is an EIP-4844 receipt.
    pub const fn is_eip4844(&self) -> bool {
        matches!(self, Self::Eip4844(_))
    }

    /// Returns `true` if this is an EIP-7702 receipt.
    pub const fn is_eip7702(&self) -> bool {
        matches!(self, Self::Eip7702(_))
    }

    /// Returns `true` if this is an OP stack deposit receipt.
    #[cfg(feature = "optimism")]
    pub const fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit(_))
    }

    /// Returns `true` if this is an OP stack post-execution synthetic receipt.
    #[cfg(feature = "optimism")]
    pub const fn is_post_exec(&self) -> bool {
        matches!(self, Self::PostExec(_))
    }

    /// Returns `true` if this is a Tempo receipt.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo(_))
    }

    /// Return the [`FoundryTxType`] of the inner receipt.
    pub const fn tx_type(&self) -> FoundryTxType {
        match self {
            Self::Legacy(_) => FoundryTxType::Legacy,
            Self::Eip2930(_) => FoundryTxType::Eip2930,
            Self::Eip1559(_) => FoundryTxType::Eip1559,
            Self::Eip4844(_) => FoundryTxType::Eip4844,
            Self::Eip7702(_) => FoundryTxType::Eip7702,
            #[cfg(feature = "optimism")]
            Self::PostExec(_) => FoundryTxType::PostExec,
            #[cfg(feature = "optimism")]
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
            #[cfg(feature = "optimism")]
            Self::PostExec(r) => FoundryReceiptEnvelope::PostExec(r.map_logs(f)),
            #[cfg(feature = "optimism")]
            Self::Deposit(r) => FoundryReceiptEnvelope::Deposit(
                r.map_receipt(|r: OpDepositReceipt<T>| r.map_logs(f)),
            ),
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
            #[cfg(feature = "optimism")]
            Self::PostExec(t) => &t.logs_bloom,
            #[cfg(feature = "optimism")]
            Self::Deposit(t) => &t.logs_bloom,
            Self::Tempo(t) => &t.logs_bloom,
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
            #[cfg(feature = "optimism")]
            Self::PostExec(t) => t.receipt,
            #[cfg(feature = "optimism")]
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
            #[cfg(feature = "optimism")]
            Self::PostExec(t) => &t.receipt,
            #[cfg(feature = "optimism")]
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
        self.network_encode(out);
    }

    fn length(&self) -> usize {
        self.network_len()
    }
}

impl Decodable for FoundryReceiptEnvelope {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Self::network_decode(buf).map_err(Into::into)
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
            #[cfg(feature = "optimism")]
            Self::PostExec(_) => POST_EXEC_TX_TYPE_ID,
            #[cfg(feature = "optimism")]
            Self::Deposit(_) => DEPOSIT_TX_TYPE_ID,
            Self::Tempo(_) => TEMPO_TX_TYPE_ID,
        }
    }
}

impl Encodable2718 for FoundryReceiptEnvelope {
    fn encode_2718_len(&self) -> usize {
        match self {
            Self::Legacy(r) => r.length(),
            Self::Eip2930(r) => 1 + r.length(),
            Self::Eip1559(r) => 1 + r.length(),
            Self::Eip4844(r) => 1 + r.length(),
            Self::Eip7702(r) => 1 + r.length(),
            #[cfg(feature = "optimism")]
            Self::PostExec(r) => 1 + r.length(),
            #[cfg(feature = "optimism")]
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
            #[cfg(feature = "optimism")]
            Self::PostExec(r) => r.encode(out),
            #[cfg(feature = "optimism")]
            Self::Deposit(r) => r.encode(out),
        }
    }
}

impl Decodable2718 for FoundryReceiptEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        #[cfg(feature = "optimism")]
        {
            if ty == DEPOSIT_TX_TYPE_ID {
                return Ok(Self::Deposit(OpDepositReceiptWithBloom::decode(buf)?));
            }
            if ty == POST_EXEC_TX_TYPE_ID {
                return Ok(Self::PostExec(ReceiptWithBloom::decode(buf)?));
            }
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

    fn receipt_for(tx_type: FoundryTxType) -> FoundryReceiptEnvelope {
        FoundryReceiptEnvelope::<alloy_rpc_types::Log>::from_parts(
            true,
            0,
            Vec::new(),
            tx_type,
            None,
            None,
        )
        .map_logs(|log| log.inner)
    }

    #[test]
    fn rlp_roundtrip() {
        fn assert_roundtrip(receipt: FoundryReceiptEnvelope) {
            let mut encoded = Vec::new();
            receipt.encode(&mut encoded);
            assert_eq!(encoded.len(), receipt.length());

            let mut encoded = encoded.as_slice();
            let decoded = FoundryReceiptEnvelope::decode(&mut encoded).unwrap();
            assert_eq!(decoded, receipt);
            assert!(encoded.is_empty());
        }

        for tx_type in [
            FoundryTxType::Legacy,
            FoundryTxType::Eip2930,
            FoundryTxType::Eip1559,
            FoundryTxType::Eip4844,
            FoundryTxType::Eip7702,
            FoundryTxType::Tempo,
        ] {
            assert_roundtrip(receipt_for(tx_type));
        }
        #[cfg(feature = "optimism")]
        for tx_type in [FoundryTxType::PostExec, FoundryTxType::Deposit] {
            assert_roundtrip(receipt_for(tx_type));
        }

        // Varied payload so encodings differ beyond the type byte.
        let logs = vec![Log {
            address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
            data: LogData::new_unchecked(
                vec![B256::repeat_byte(0x22)],
                Bytes::from_static(&[0x01, 0x02, 0x03]),
            ),
        }];
        let logs_bloom = logs_bloom(&logs);
        let receipt = Receipt { status: false.into(), cumulative_gas_used: 0x2a, logs };
        assert_roundtrip(FoundryReceiptEnvelope::Eip1559(ReceiptWithBloom {
            receipt: receipt.clone(),
            logs_bloom,
        }));
        // A deposit receipt with set deposit fields; op-alloy encodes them only when `Some`, so
        // this catches decode paths that drop them.
        #[cfg(feature = "optimism")]
        assert_roundtrip(FoundryReceiptEnvelope::Deposit(OpDepositReceiptWithBloom {
            receipt: OpDepositReceipt {
                inner: receipt,
                deposit_nonce: Some(7),
                deposit_receipt_version: Some(1),
            },
            logs_bloom,
        }));
    }

    #[test]
    fn encode_typed_receipt_uses_rlp_string() {
        let receipt = receipt_for(FoundryTxType::Eip2930);
        let mut encoded = Vec::new();
        receipt.encode(&mut encoded);

        // Long string containing a 266-byte EIP-2718 envelope, beginning with type 0x01.
        assert_eq!(&encoded[..4], &[0xb9, 0x01, 0x0a, EIP2930_TX_TYPE_ID]);
    }

    #[test]
    fn receipt_predicates() {
        assert!(receipt_for(FoundryTxType::Legacy).is_legacy());
        assert!(receipt_for(FoundryTxType::Eip2930).is_eip2930());
        assert!(receipt_for(FoundryTxType::Eip1559).is_eip1559());
        assert!(receipt_for(FoundryTxType::Eip4844).is_eip4844());
        assert!(receipt_for(FoundryTxType::Eip7702).is_eip7702());
        assert!(receipt_for(FoundryTxType::Tempo).is_tempo());
        assert!(!receipt_for(FoundryTxType::Tempo).is_legacy());

        #[cfg(feature = "optimism")]
        {
            assert!(receipt_for(FoundryTxType::Deposit).is_deposit());
            assert!(receipt_for(FoundryTxType::PostExec).is_post_exec());
        }
    }

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

        // The EIP-2718 encoding starts with the Tempo type byte.
        let mut encoded_2718 = Vec::new();
        receipt.encode_2718(&mut encoded_2718);
        assert_eq!(encoded_2718[0], TEMPO_TX_TYPE_ID);

        // `Decodable` expects the network format, which wraps the typed payload in an RLP string.
        let mut encoded = Vec::new();
        receipt.encode(&mut encoded);
        let decoded = FoundryReceiptEnvelope::decode(&mut &encoded[..]).unwrap();
        assert_eq!(receipt, decoded);
    }

    #[test]
    fn decode_tempo_receipt() {
        let receipt = FoundryReceiptEnvelope::Tempo(ReceiptWithBloom {
            receipt: Receipt { status: true.into(), cumulative_gas_used: 21000, logs: vec![] },
            logs_bloom: [0; 256].into(),
        });

        // Encode and decode via 2718.
        let mut encoded = Vec::new();
        receipt.encode_2718(&mut encoded);
        assert_eq!(encoded[0], TEMPO_TX_TYPE_ID);

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
        #[cfg(feature = "optimism")]
        {
            assert!(receipt.deposit_nonce().is_none());
            assert!(receipt.deposit_receipt_version().is_none());
        }
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
