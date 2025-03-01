use alloy_consensus_any::AnyReceiptEnvelope;
use alloy_rpc_types_eth::{Log, TransactionReceipt};
use alloy_serde::WithOtherFields;

/// Alias for a catch-all receipt type.
#[doc(alias = "AnyTxReceipt")]
pub type AnyTransactionReceipt = WithOtherFields<TransactionReceipt<AnyReceiptEnvelope<Log>>>;

#[cfg(test)]
mod test {
    use super::*;
    use alloy_primitives::b256;

    #[test]
    fn deserialize_tx_receipt_op() {
        // OtherFields for Optimism
        #[derive(Debug, serde::Deserialize)]
        struct OpOtherFields {
            #[serde(rename = "l1BaseFeeScalar")]
            l1_base_fee_scalar: String,
            #[serde(rename = "l1BlobBaseFee")]
            l1_blob_base_fee: String,
            #[serde(rename = "l1BlobBaseFeeScalar")]
            l1_blob_base_fee_scalar: String,
            #[serde(rename = "l1Fee")]
            l1_fee: String,
            #[serde(rename = "l1GasPrice")]
            l1_gas_price: String,
            #[serde(rename = "l1GasUsed")]
            l1_gas_used: String,
        }

        let receipt_json = r#"
        {
            "status": "0x1",
            "cumulativeGasUsed": "0xf1740",
            "logs": [
                {
                "address": "0x4200000000000000000000000000000000000006",
                "topics": [
                    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    "0x0000000000000000000000005112996d3ae99f0b5360cea1a620ffcd78e8ff83",
                    "0x00000000000000000000000077e7c5cbeaad915cf5462064b02984e16a902e67"
                ],
                "data": "0x000000000000000000000000000000000000000000000000001c66f6e8b40c00",
                "blockHash": "0x88e07a0d797b84bd122d6993a6faf5a59ada7f40c181c553c191dd400d3d1583",
                "blockNumber": "0x73a43e1",
                "transactionHash": "0x2bc7cb4648e847712e39abd42178e35214a70bb15c568d604687661b9539b4c2",
                "transactionIndex": "0x9",
                "logIndex": "0x16",
                "removed": false
                }
            ],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000040000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000008000000100000000000000000100000000000000000000010000020000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000002000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "type": "0x0",
            "transactionHash": "0x2bc7cb4648e847712e39abd42178e35214a70bb15c568d604687661b9539b4c2",
            "transactionIndex": "0x9",
            "blockHash": "0x88e07a0d797b84bd122d6993a6faf5a59ada7f40c181c553c191dd400d3d1583",
            "blockNumber": "0x73a43e1",
            "gasUsed": "0x85b9",
            "effectiveGasPrice": "0x3ac9e84",
            "from": "0x5112996d3ae99f0b5360cea1a620ffcd78e8ff83",
            "to": "0x4200000000000000000000000000000000000006",
            "contractAddress": null,
            "l1BaseFeeScalar": "0x558",
            "l1BlobBaseFee": "0x1",
            "l1BlobBaseFeeScalar": "0xc5fc5",
            "l1Fee": "0x105d4b2024",
            "l1GasPrice": "0x5d749a07e",
            "l1GasUsed": "0x800"
        }
        "#;
        let receipt = serde_json::from_str::<AnyTransactionReceipt>(receipt_json).unwrap();

        assert_eq!(
            receipt.transaction_hash,
            b256!("2bc7cb4648e847712e39abd42178e35214a70bb15c568d604687661b9539b4c2")
        );

        let other: OpOtherFields = receipt.other.deserialize_into().unwrap();
        assert_eq!(other.l1_base_fee_scalar, "0x558");
        assert_eq!(other.l1_blob_base_fee, "0x1");
        assert_eq!(other.l1_blob_base_fee_scalar, "0xc5fc5");
        assert_eq!(other.l1_fee, "0x105d4b2024");
        assert_eq!(other.l1_gas_price, "0x5d749a07e");
        assert_eq!(other.l1_gas_used, "0x800");
    }

    #[test]
    fn deserialize_tx_receipt_arb() {
        // OtherFields for Arbitrum
        #[derive(Debug, serde::Deserialize)]
        struct ArbOtherFields {
            #[serde(rename = "gasUsedForL1")]
            gas_used_for_l1: String,
            #[serde(rename = "l1BlockNumber")]
            l1_block_number: String,
        }

        let receipt_json = r#"
        {
            "status": "0x1",
            "cumulativeGasUsed": "0x27ebb8",
            "logs": [
                {
                "address": "0x912ce59144191c1204e64559fe8253a0e49e6548",
                "topics": [
                    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    "0x000000000000000000000000e487d95426e55a29f2266e6788ab55608ebb829b",
                    "0x0000000000000000000000009855134ed0c8b71266d9f3e15c0a518c07be5baf"
                ],
                "data": "0x00000000000000000000000000000000000000000000000009d40825d5ee8000",
                "blockHash": "0x83ddb8850803238bd58615680bc3718686ec1e3deaea0bc5f67c07c8577547f5",
                "blockNumber": "0xd288ac5",
                "transactionHash": "0x5aeca744e0c1f6d7f68641aedd394ac4b6e18cbeac3f8b3c81056c0e51a61cf3",
                "transactionIndex": "0x7",
                "logIndex": "0x7",
                "removed": false
                }
            ],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000005000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000008000100000000000001000000000000000000000000000000000000000000020000000000000000000004400000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "type": "0x0",
            "transactionHash": "0x5aeca744e0c1f6d7f68641aedd394ac4b6e18cbeac3f8b3c81056c0e51a61cf3",
            "transactionIndex": "0x7",
            "blockHash": "0x83ddb8850803238bd58615680bc3718686ec1e3deaea0bc5f67c07c8577547f5",
            "blockNumber": "0xd288ac5",
            "gasUsed": "0x3ad89",
            "effectiveGasPrice": "0x989680",
            "from": "0xe487d95426e55a29f2266e6788ab55608ebb829b",
            "to": "0x912ce59144191c1204e64559fe8253a0e49e6548",
            "contractAddress": null,
            "gasUsedForL1": "0x2c906",
            "l1BlockNumber": "0x1323b96"
        }
        "#;
        let receipt = serde_json::from_str::<AnyTransactionReceipt>(receipt_json).unwrap();

        assert_eq!(
            receipt.transaction_hash,
            b256!("5aeca744e0c1f6d7f68641aedd394ac4b6e18cbeac3f8b3c81056c0e51a61cf3")
        );

        let other: ArbOtherFields = receipt.other.deserialize_into().unwrap();
        assert_eq!(other.gas_used_for_l1, "0x2c906");
        assert_eq!(other.l1_block_number, "0x1323b96");
    }
}
