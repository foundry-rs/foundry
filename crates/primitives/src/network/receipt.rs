use alloy_consensus::{Receipt, TxReceipt};
use alloy_network::{AnyReceiptEnvelope, AnyTransactionReceipt, ReceiptResponse};
use alloy_primitives::{Address, B256, BlockHash, TxHash, U64};
use alloy_rpc_types::{ConversionError, Log, TransactionReceipt};
use alloy_serde::WithOtherFields;
use derive_more::AsRef;
use foundry_common_fmt::UIfmt;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom};
use serde::{Deserialize, Serialize};

use crate::FoundryReceiptEnvelope;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, AsRef)]
pub struct FoundryTxReceipt(WithOtherFields<TransactionReceipt<FoundryReceiptEnvelope<Log>>>);

impl FoundryTxReceipt {
    pub fn new(inner: TransactionReceipt<FoundryReceiptEnvelope<Log>>) -> Self {
        Self(WithOtherFields::new(inner))
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

impl UIfmt for FoundryTxReceipt {
    fn pretty(&self) -> String {
        let receipt = &self.0.inner;
        let other = &self.0.other;

        let mut pretty = format!(
            "
blockHash            {}
blockNumber          {}
contractAddress      {}
cumulativeGasUsed    {}
effectiveGasPrice    {}
from                 {}
gasUsed              {}
logs                 {}
logsBloom            {}
root                 {}
status               {}
transactionHash      {}
transactionIndex     {}
type                 {}
blobGasPrice         {}
blobGasUsed          {}",
            receipt.block_hash.pretty(),
            receipt.block_number.pretty(),
            receipt.contract_address.pretty(),
            receipt.inner.cumulative_gas_used().pretty(),
            receipt.effective_gas_price.pretty(),
            receipt.from.pretty(),
            receipt.gas_used.pretty(),
            serde_json::to_string(receipt.inner.logs()).unwrap(),
            receipt.inner.logs_bloom().pretty(),
            self.state_root().pretty(),
            receipt.inner.status().pretty(),
            receipt.transaction_hash.pretty(),
            receipt.transaction_index.pretty(),
            receipt.inner.tx_type() as u8,
            receipt.blob_gas_price.pretty(),
            receipt.blob_gas_used.pretty()
        );

        if let Some(to) = receipt.to {
            pretty.push_str(&format!("\nto                   {}", to.pretty()));
        }

        // additional captured fields
        pretty.push_str(&other.pretty());

        pretty
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
        } = receipt;

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
                    0x7E => {
                        // Construct the deposit receipt, extracting optional deposit fields
                        // These fields may not be present in all receipts, so missing/invalid
                        // values are None
                        let deposit_nonce = other
                            .get_deserialized::<U64>("depositNonce")
                            .transpose()
                            .ok()
                            .flatten()
                            .map(|v| v.to::<u64>());
                        let deposit_receipt_version = other
                            .get_deserialized::<U64>("depositReceiptVersion")
                            .transpose()
                            .ok()
                            .flatten()
                            .map(|v| v.to::<u64>());

                        FoundryReceiptEnvelope::Deposit(OpDepositReceiptWithBloom {
                            receipt: OpDepositReceipt {
                                inner: Receipt {
                                    status: alloy_consensus::Eip658Value::Eip658(
                                        receipt_with_bloom.status(),
                                    ),
                                    cumulative_gas_used: receipt_with_bloom.cumulative_gas_used(),
                                    logs: receipt_with_bloom.receipt.logs,
                                },
                                deposit_nonce,
                                deposit_receipt_version,
                            },
                            logs_bloom: receipt_with_bloom.logs_bloom,
                        })
                    }
                    _ => {
                        let tx_type = r#type;
                        return Err(ConversionError::Custom(format!(
                            "Unknown transaction receipt type: 0x{tx_type:02X}"
                        )));
                    }
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

    #[test]
    fn test_foundry_tx_receipt_uifmt() {
        // Test UIfmt implementation for FoundryTxReceipt
        let s = r#"{"type":"0x2","status":"0x1","cumulativeGasUsed":"0x5208","logs":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionHash":"0x1234567890123456789012345678901234567890123456789012345678901234","transactionIndex":"0x0","blockHash":"0xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd","blockNumber":"0x1","gasUsed":"0x5208","effectiveGasPrice":"0x3b9aca00","from":"0x1234567890123456789012345678901234567890","to":"0x0987654321098765432109876543210987654321","contractAddress":null}"#;
        let any_receipt: AnyTransactionReceipt = serde_json::from_str(s).unwrap();
        let foundry_receipt = FoundryTxReceipt::try_from(any_receipt).unwrap();

        let pretty_output = foundry_receipt.pretty();

        // Check that essential fields are present in the output
        assert!(pretty_output.contains("blockHash"));
        assert!(pretty_output.contains("blockNumber"));
        assert!(pretty_output.contains("status"));
        assert!(pretty_output.contains("gasUsed"));
        assert!(pretty_output.contains("transactionHash"));
        assert!(pretty_output.contains("type"));

        // Verify the transaction hash appears in the output
        assert!(
            pretty_output
                .contains("0x1234567890123456789012345678901234567890123456789012345678901234")
        );

        // Verify status is pretty printed correctly (boolean true for successful transaction)
        assert!(pretty_output.contains("true"));
    }
}
