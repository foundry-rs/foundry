use alloy_consensus::{
    Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom, TxReceipt, Typed2718,
};
use alloy_network::{
    AnyReceiptEnvelope,
    eip2718::{
        Decodable2718, EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID, EIP4844_TX_TYPE_ID,
        EIP7702_TX_TYPE_ID, Eip2718Error, Encodable2718, LEGACY_TX_TYPE_ID,
    },
};
use alloy_primitives::{Bloom, Log, TxHash, logs_bloom};
use alloy_rlp::{BufMut, Decodable, Encodable, bytes};
use alloy_rpc_types::{BlockNumHash, trace::otterscan::OtsReceipt};
#[cfg(feature = "base")]
use base_common_consensus::Eip8130Receipt;
#[cfg(feature = "base")]
use base_common_evm::EIP8130_TRANSACTION_TYPE;
#[cfg(all(feature = "base", not(feature = "optimism")))]
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, OpDepositReceipt, OpDepositReceiptWithBloom};
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
    #[cfg(any(feature = "base", feature = "optimism"))]
    #[serde(rename = "0x7E", alias = "0x7e")]
    Deposit(OpDepositReceiptWithBloom<T>),
    #[cfg(feature = "base")]
    #[serde(rename = "0x79")]
    Eip8130(ReceiptWithBloom<Eip8130Receipt<T>>),
    #[serde(rename = "0x76")]
    Tempo(ReceiptWithBloom<Receipt<T>>),
    /// A receipt with a transaction type Foundry does not model.
    ///
    /// Anvil cannot execute these transactions, but it must still relay receipts fetched from a
    /// forked chain that mints them, for example Arbitrum Nitro's `0x64`-`0x6a` types. The type
    /// byte is preserved so the receipt round-trips through RPC and RLP unchanged.
    #[serde(untagged)]
    Unknown(AnyReceiptEnvelope<T>),
}

impl FoundryReceiptEnvelope<alloy_rpc_types::Log> {
    /// Creates a new [`FoundryReceiptEnvelope`] from the given parts.
    pub fn from_parts(
        status: bool,
        cumulative_gas_used: u64,
        logs: impl IntoIterator<Item = alloy_rpc_types::Log>,
        tx_type: FoundryTxType,
        #[cfg(feature = "base")] eip8130_phase_statuses: Vec<u8>,
        #[cfg_attr(not(any(feature = "base", feature = "optimism")), allow(unused_variables))]
        deposit_nonce: Option<u64>,
        #[cfg_attr(not(any(feature = "base", feature = "optimism")), allow(unused_variables))]
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
            #[cfg(any(feature = "base", feature = "optimism"))]
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
            #[cfg(feature = "base")]
            FoundryTxType::Eip8130 => Self::Eip8130(ReceiptWithBloom {
                receipt: Eip8130Receipt::new(inner_receipt, eip8130_phase_statuses),
                logs_bloom,
            }),
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
    /// Returns `true` if this is an OP stack deposit receipt.
    #[cfg(any(feature = "base", feature = "optimism"))]
    pub const fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit(_))
    }

    /// Returns `true` if this is an OP stack post-execution synthetic receipt.
    #[cfg(feature = "optimism")]
    pub const fn is_post_exec(&self) -> bool {
        matches!(self, Self::PostExec(_))
    }

    /// Returns `true` if this is a Base EIP-8130 receipt.
    #[cfg(feature = "base")]
    pub const fn is_eip8130(&self) -> bool {
        matches!(self, Self::Eip8130(_))
    }

    /// Returns EIP-8130 per-phase statuses, or an empty slice for other receipt types.
    #[cfg(feature = "base")]
    pub fn eip8130_phase_statuses(&self) -> &[u8] {
        match self {
            Self::Eip8130(receipt) => &receipt.receipt.phase_statuses,
            _ => &[],
        }
    }

    /// Returns `true` if this is a Tempo receipt.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo(_))
    }

    /// Returns `true` if the receipt's transaction type is not modelled by Foundry.
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// Returns the EIP-2718 type byte of the inner receipt.
    pub const fn ty(&self) -> u8 {
        match self {
            Self::Legacy(_) => LEGACY_TX_TYPE_ID,
            Self::Eip2930(_) => EIP2930_TX_TYPE_ID,
            Self::Eip1559(_) => EIP1559_TX_TYPE_ID,
            Self::Eip4844(_) => EIP4844_TX_TYPE_ID,
            Self::Eip7702(_) => EIP7702_TX_TYPE_ID,
            #[cfg(feature = "optimism")]
            Self::PostExec(_) => POST_EXEC_TX_TYPE_ID,
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(_) => DEPOSIT_TX_TYPE_ID,
            #[cfg(feature = "base")]
            Self::Eip8130(_) => EIP8130_TRANSACTION_TYPE,
            Self::Tempo(_) => TEMPO_TX_TYPE_ID,
            Self::Unknown(r) => r.r#type,
        }
    }

    /// Return the [`FoundryTxType`] of the inner receipt, or `None` for an unknown type.
    pub const fn tx_type(&self) -> Option<FoundryTxType> {
        Some(match self {
            Self::Legacy(_) => FoundryTxType::Legacy,
            Self::Eip2930(_) => FoundryTxType::Eip2930,
            Self::Eip1559(_) => FoundryTxType::Eip1559,
            Self::Eip4844(_) => FoundryTxType::Eip4844,
            Self::Eip7702(_) => FoundryTxType::Eip7702,
            #[cfg(feature = "optimism")]
            Self::PostExec(_) => FoundryTxType::PostExec,
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(_) => FoundryTxType::Deposit,
            #[cfg(feature = "base")]
            Self::Eip8130(_) => FoundryTxType::Eip8130,
            Self::Tempo(_) => FoundryTxType::Tempo,
            Self::Unknown(_) => return None,
        })
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
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(r) => FoundryReceiptEnvelope::Deposit(
                r.map_receipt(|r: OpDepositReceipt<T>| r.map_logs(f)),
            ),
            #[cfg(feature = "base")]
            Self::Eip8130(r) => {
                FoundryReceiptEnvelope::Eip8130(r.map_receipt(|r: Eip8130Receipt<T>| r.map_logs(f)))
            }
            Self::Tempo(r) => FoundryReceiptEnvelope::Tempo(r.map_logs(f)),
            Self::Unknown(r) => FoundryReceiptEnvelope::Unknown(AnyReceiptEnvelope {
                inner: r.inner.map_logs(f),
                r#type: r.r#type,
            }),
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
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(t) => &t.logs_bloom,
            #[cfg(feature = "base")]
            Self::Eip8130(t) => &t.logs_bloom,
            Self::Tempo(t) => &t.logs_bloom,
            Self::Unknown(t) => &t.inner.logs_bloom,
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
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(t) => t.receipt.into_inner(),
            #[cfg(feature = "base")]
            Self::Eip8130(t) => t.receipt.into_inner(),
            Self::Unknown(t) => t.inner.receipt,
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
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(t) => &t.receipt.inner,
            #[cfg(feature = "base")]
            Self::Eip8130(t) => &t.receipt.inner,
            Self::Unknown(t) => &t.inner.receipt,
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

impl<T> Typed2718 for FoundryReceiptEnvelope<T> {
    fn ty(&self) -> u8 {
        self.ty()
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
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(r) => 1 + r.length(),
            #[cfg(feature = "base")]
            Self::Eip8130(r) => 1 + r.length(),
            Self::Tempo(r) => 1 + r.length(),
            Self::Unknown(r) => r.rlp_payload_length(),
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
            #[cfg(any(feature = "base", feature = "optimism"))]
            Self::Deposit(r) => r.encode(out),
            #[cfg(feature = "base")]
            Self::Eip8130(r) => r.encode(out),
            Self::Unknown(r) => r.inner.encode(out),
        }
    }
}

impl Decodable2718 for FoundryReceiptEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        #[cfg(feature = "base")]
        if ty == EIP8130_TRANSACTION_TYPE {
            return Ok(Self::Eip8130(ReceiptWithBloom::decode(buf)?));
        }
        #[cfg(all(feature = "base", not(feature = "optimism")))]
        if ty == DEPOSIT_TX_TYPE_ID {
            return Ok(Self::Deposit(OpDepositReceiptWithBloom::decode(buf)?));
        }
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
        match ty {
            LEGACY_TX_TYPE_ID => Err(Eip2718Error::UnexpectedType(LEGACY_TX_TYPE_ID)),
            EIP2930_TX_TYPE_ID | EIP1559_TX_TYPE_ID | EIP4844_TX_TYPE_ID | EIP7702_TX_TYPE_ID => {
                match ReceiptEnvelope::typed_decode(ty, buf)? {
                    ReceiptEnvelope::Eip2930(tx) => Ok(Self::Eip2930(tx)),
                    ReceiptEnvelope::Eip1559(tx) => Ok(Self::Eip1559(tx)),
                    ReceiptEnvelope::Eip4844(tx) => Ok(Self::Eip4844(tx)),
                    ReceiptEnvelope::Eip7702(tx) => Ok(Self::Eip7702(tx)),
                    _ => {
                        Err(Eip2718Error::RlpError(alloy_rlp::Error::Custom("unexpected tx type")))
                    }
                }
            }
            // Receipts for transaction types Foundry does not model, such as Arbitrum's, are kept
            // verbatim so they survive a decode/encode round-trip.
            _ => Ok(Self::Unknown(AnyReceiptEnvelope {
                inner: ReceiptWithBloom::decode(buf)?,
                r#type: ty,
            })),
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
            r#type: receipt.ty(),
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
            #[cfg(feature = "base")]
            Vec::new(),
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

    /// Arbitrum's `ArbitrumInternalTx`; anvil forks these chains but cannot execute their
    /// transactions, so their receipts must survive as-is.
    const ARBITRUM_INTERNAL_TX_TYPE: u8 = 0x6a;

    fn unknown_receipt(ty: u8) -> FoundryReceiptEnvelope {
        let logs = vec![Log {
            address: Address::from_str("0000000000000000000000000000000000000064").unwrap(),
            data: LogData::new_unchecked(
                vec![B256::repeat_byte(0x33)],
                Bytes::from_static(&[0xaa, 0xbb]),
            ),
        }];
        let logs_bloom = logs_bloom(&logs);
        FoundryReceiptEnvelope::Unknown(AnyReceiptEnvelope {
            inner: ReceiptWithBloom {
                receipt: Receipt { status: true.into(), cumulative_gas_used: 0x1234, logs },
                logs_bloom,
            },
            r#type: ty,
        })
    }

    #[test]
    fn unknown_receipt_roundtrips_and_keeps_type() {
        let receipt = unknown_receipt(ARBITRUM_INTERNAL_TX_TYPE);

        assert!(receipt.is_unknown());
        assert_eq!(receipt.tx_type(), None);
        assert_eq!(receipt.ty(), ARBITRUM_INTERNAL_TX_TYPE);
        assert!(receipt.status());
        assert_eq!(receipt.cumulative_gas_used(), 0x1234);
        assert_eq!(receipt.logs().len(), 1);

        let mut encoded_2718 = Vec::new();
        receipt.encode_2718(&mut encoded_2718);
        assert_eq!(encoded_2718[0], ARBITRUM_INTERNAL_TX_TYPE);
        assert_eq!(encoded_2718.len(), receipt.encode_2718_len());
        assert_eq!(FoundryReceiptEnvelope::decode_2718(&mut &encoded_2718[..]).unwrap(), receipt);

        let mut encoded = Vec::new();
        receipt.encode(&mut encoded);
        assert_eq!(encoded.len(), receipt.length());
        assert_eq!(FoundryReceiptEnvelope::decode(&mut &encoded[..]).unwrap(), receipt);
    }

    #[test]
    fn unknown_receipt_serde_roundtrip() {
        let receipt = unknown_receipt(ARBITRUM_INTERNAL_TX_TYPE);
        let json = serde_json::to_value(&receipt).unwrap();
        assert_eq!(json["type"], "0x6a");
        assert_eq!(serde_json::from_value::<FoundryReceiptEnvelope>(json).unwrap(), receipt);

        // The fallback must not swallow types that have a dedicated variant.
        let known = receipt_for(FoundryTxType::Eip1559);
        let json = serde_json::to_value(&known).unwrap();
        let decoded = serde_json::from_value::<FoundryReceiptEnvelope>(json).unwrap();
        assert_eq!(decoded, known);
        assert!(!decoded.is_unknown());
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

    #[cfg(feature = "base")]
    #[test]
    fn eip8130_receipt_preserves_phase_statuses_outside_consensus_encoding() {
        use alloy_network::eip2718::{Decodable2718, Encodable2718};

        let receipt = FoundryReceiptEnvelope::<alloy_rpc_types::Log>::from_parts(
            false,
            42_000,
            Vec::new(),
            FoundryTxType::Eip8130,
            vec![0x01, 0x00],
            None,
            None,
        )
        .map_logs(|log| log.inner);
        assert_eq!(receipt.eip8130_phase_statuses(), &[0x01, 0x00]);

        let encoded = receipt.encoded_2718();
        let decoded = FoundryReceiptEnvelope::decode_2718(&mut encoded.as_slice()).unwrap();
        assert!(decoded.eip8130_phase_statuses().is_empty());
        assert_eq!(decoded.encoded_2718(), encoded);
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

        assert_eq!(receipt.tx_type(), Some(FoundryTxType::Tempo));
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
            #[cfg(feature = "base")]
            Vec::new(),
            None,
            None,
        );

        assert_eq!(receipt.tx_type(), Some(FoundryTxType::Tempo));
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
        assert_eq!(mapped.tx_type(), Some(FoundryTxType::Tempo));
    }
}
