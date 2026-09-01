use alloy_network::{AnyReceiptEnvelope, AnyTransactionReceipt, ReceiptResponse};
use alloy_primitives::{Address, B256, BlockHash, TxHash};
use alloy_rpc_types::{ConversionError, Log, TransactionReceipt};
use alloy_serde::WithOtherFields;
#[cfg(feature = "base")]
use base_common_consensus::Eip8130Receipt;
use derive_more::AsRef;
use serde::{Deserialize, Serialize};
use tempo_primitives::TEMPO_TX_TYPE_ID;

#[cfg(any(feature = "base", feature = "optimism"))]
use super::optimism::build_deposit_receipt_envelope;
use crate::FoundryReceiptEnvelope;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, AsRef)]
pub struct FoundryTxReceipt(pub WithOtherFields<TransactionReceipt<FoundryReceiptEnvelope<Log>>>);

impl FoundryTxReceipt {
    pub fn new(inner: TransactionReceipt<FoundryReceiptEnvelope<Log>>) -> Self {
        Self(WithOtherFields::new(inner))
    }

    /// Creates a new receipt with a timestamp in the other fields.
    /// This avoids extra block lookups when timestamp is needed later.
    pub fn with_timestamp(
        inner: TransactionReceipt<FoundryReceiptEnvelope<Log>>,
        timestamp: u64,
    ) -> Self {
        let mut receipt = WithOtherFields::new(inner);
        receipt
            .other
            .insert("blockTimestamp".to_string(), serde_json::to_value(timestamp).unwrap());
        Self(receipt)
    }

    /// Adds a `feePayer` field to the receipt.
    pub fn with_fee_payer(mut self, fee_payer: Address) -> Self {
        self.0.other.insert("feePayer".to_string(), serde_json::to_value(fee_payer).unwrap());
        self
    }

    /// Adds a `feeToken` field to the receipt.
    pub fn with_fee_token(mut self, fee_token: Address) -> Self {
        self.0.other.insert("feeToken".to_string(), serde_json::to_value(fee_token).unwrap());
        self
    }

    /// Get block timestamp from other fields if present.
    pub fn block_timestamp(&self) -> Option<u64> {
        self.0.other.get_deserialized::<u64>("blockTimestamp").transpose().ok().flatten()
    }
}

impl ReceiptResponse for FoundryTxReceipt {
    fn contract_address(&self) -> Option<Address> {
        self.0.contract_address
    }

    fn status(&self) -> bool {
        self.0.inner.status()
    }

    fn block_hash(&self) -> Option<BlockHash> {
        self.0.block_hash
    }

    fn block_number(&self) -> Option<u64> {
        self.0.block_number
    }

    fn transaction_hash(&self) -> TxHash {
        self.0.transaction_hash
    }

    fn transaction_index(&self) -> Option<u64> {
        self.0.transaction_index()
    }

    fn gas_used(&self) -> u64 {
        self.0.gas_used()
    }

    fn effective_gas_price(&self) -> u128 {
        self.0.effective_gas_price()
    }

    fn blob_gas_used(&self) -> Option<u64> {
        self.0.blob_gas_used()
    }

    fn blob_gas_price(&self) -> Option<u128> {
        self.0.blob_gas_price()
    }

    fn from(&self) -> Address {
        self.0.from()
    }

    fn to(&self) -> Option<Address> {
        self.0.to()
    }

    fn cumulative_gas_used(&self) -> u64 {
        self.0.cumulative_gas_used()
    }

    fn state_root(&self) -> Option<B256> {
        self.0.state_root()
    }
}

impl TryFrom<AnyTransactionReceipt> for FoundryTxReceipt {
    type Error = ConversionError;

    fn try_from(receipt: AnyTransactionReceipt) -> Result<Self, Self::Error> {
        let WithOtherFields {
            inner:
                TransactionReceipt {
                    transaction_hash,
                    transaction_index,
                    block_hash,
                    block_number,
                    gas_used,
                    contract_address,
                    effective_gas_price,
                    from,
                    to,
                    blob_gas_price,
                    blob_gas_used,
                    inner: AnyReceiptEnvelope { inner: receipt_with_bloom, r#type },
                },
            other,
        } = receipt.0;

        Ok(Self(WithOtherFields {
            inner: TransactionReceipt {
                transaction_hash,
                transaction_index,
                block_hash,
                block_number,
                gas_used,
                contract_address,
                effective_gas_price,
                from,
                to,
                blob_gas_price,
                blob_gas_used,
                inner: match r#type {
                    0x00 => FoundryReceiptEnvelope::Legacy(receipt_with_bloom),
                    0x01 => FoundryReceiptEnvelope::Eip2930(receipt_with_bloom),
                    0x02 => FoundryReceiptEnvelope::Eip1559(receipt_with_bloom),
                    0x03 => FoundryReceiptEnvelope::Eip4844(receipt_with_bloom),
                    0x04 => FoundryReceiptEnvelope::Eip7702(receipt_with_bloom),
                    #[cfg(feature = "base")]
                    0x79 => FoundryReceiptEnvelope::Eip8130(alloy_consensus::ReceiptWithBloom {
                        receipt: Eip8130Receipt::new(receipt_with_bloom.receipt, Vec::new()),
                        logs_bloom: receipt_with_bloom.logs_bloom,
                    }),
                    TEMPO_TX_TYPE_ID => FoundryReceiptEnvelope::Tempo(receipt_with_bloom),
                    #[cfg(any(feature = "base", feature = "optimism"))]
                    0x7E => build_deposit_receipt_envelope(receipt_with_bloom, &other),
                    // Chains anvil can fork but not execute, such as Arbitrum and its Orbit
                    // rollups, mint their own transaction types. Keep those receipts verbatim
                    // instead of failing the whole request.
                    ty => FoundryReceiptEnvelope::Unknown(AnyReceiptEnvelope {
                        inner: receipt_with_bloom,
                        r#type: ty,
                    }),
                },
            },
            other,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // <https://github.com/foundry-rs/foundry/issues/10852>
    #[test]
    fn test_receipt_convert() {
        let s = r#"{"type":"0x4","status":"0x1","cumulativeGasUsed":"0x903fd1","logs":[{"address":"0x0000d9fcd47bf761e7287d8ee09917d7e2100000","topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef","0x0000000000000000000000000000000000000000000000000000000000000000","0x000000000000000000000000234ce51365b9c417171b6dad280f49143e1b0547"],"data":"0x00000000000000000000000000000000000000000000032139b42c3431700000","blockHash":"0xd26b59c1d8b5bfa9362d19eb0da3819dfe0b367987a71f6d30908dd45e0d7a60","blockNumber":"0x159663e","blockTimestamp":"0x68411f7b","transactionHash":"0x17a6af73d1317e69cfc3cac9221bd98261d40f24815850a44dbfbf96652ae52a","transactionIndex":"0x22","logIndex":"0x158","removed":false}],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000000000000000008100000000000000000000000000000000000000000000000020000200000000000000800000000800000000000000010000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000","transactionHash":"0x17a6af73d1317e69cfc3cac9221bd98261d40f24815850a44dbfbf96652ae52a","transactionIndex":"0x22","blockHash":"0xd26b59c1d8b5bfa9362d19eb0da3819dfe0b367987a71f6d30908dd45e0d7a60","blockNumber":"0x159663e","gasUsed":"0x28ee7","effectiveGasPrice":"0x4bf02090","from":"0x234ce51365b9c417171b6dad280f49143e1b0547","to":"0x234ce51365b9c417171b6dad280f49143e1b0547","contractAddress":null}"#;
        let receipt: AnyTransactionReceipt = serde_json::from_str(s).unwrap();
        let _converted = FoundryTxReceipt::try_from(receipt).unwrap();
    }

    // Arbitrum and its Orbit rollups mint transaction types anvil cannot execute; forked receipts
    // for them must still convert, keeping the original type byte.
    #[test]
    fn test_arbitrum_internal_receipt_convert() {
        let s = r#"{"type":"0x6a","status":"0x1","cumulativeGasUsed":"0x0","logs":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionHash":"0x7c9e0e2b0f2ffbd0a1ee3e2e2b6ff5a2ff8b6a0f1c0b4a5c1d5a8b6c9d0e1f22","transactionIndex":"0x0","blockHash":"0x3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b","blockNumber":"0x159663e","gasUsed":"0x0","effectiveGasPrice":"0x0","from":"0x00000000000000000000000000000000000a4b05","to":"0x00000000000000000000000000000000000a4b05","contractAddress":null,"gasUsedForL1":"0x0","l1BlockNumber":"0x1499e2c"}"#;
        let receipt: AnyTransactionReceipt = serde_json::from_str(s).unwrap();
        let converted = FoundryTxReceipt::try_from(receipt).unwrap();

        assert!(converted.0.inner.inner.is_unknown());
        assert_eq!(converted.0.inner.inner.ty(), 0x6a);
        assert_eq!(serde_json::to_value(&converted).unwrap()["type"], "0x6a");
    }
}
