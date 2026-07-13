use super::transaction::TransactionInfo;
#[cfg(test)]
use alloy_consensus::Header;
use alloy_consensus::{
    BlockBody, EMPTY_OMMER_ROOT_HASH, TxEip4844Variant, Typed2718,
    proofs::ordered_trie_root_with_encoder, transaction::RlpEcdsaEncodableTx,
};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::Network;
use foundry_primitives::{FoundryHeader, FoundryTxEnvelope};

use crate::eth::transaction::MaybeImpersonatedTransaction;

/// Type alias for a block containing potentially impersonated transactions.
pub type Block<T = FoundryTxEnvelope, H = FoundryHeader> =
    alloy_consensus::Block<MaybeImpersonatedTransaction<T>, H>;

/// Container type that gathers all block data, generic over a [`Network`].
#[derive(Clone, Debug)]
pub struct BlockInfo<N: Network> {
    pub block: Block<N::TxEnvelope>,
    pub transactions: Vec<TransactionInfo>,
    pub receipts: Vec<N::ReceiptEnvelope>,
}

/// A transaction that can be encoded for inclusion in a block body.
pub trait EncodableBlockTransaction: Encodable2718 {
    /// Encodes the transaction in its canonical block-body form.
    fn encode_2718_for_block(&self, out: &mut dyn bytes::BufMut);
}

impl EncodableBlockTransaction for FoundryTxEnvelope {
    fn encode_2718_for_block(&self, out: &mut dyn bytes::BufMut) {
        if let Self::Eip4844(tx) = self {
            out.put_u8(self.ty());
            tx.tx().tx().rlp_encode_signed(tx.signature(), out);
        } else {
            self.encode_2718(out);
        }
    }
}

/// Drops pooled sidecars so a transaction uses its canonical block-body representation.
pub fn canonical_block_transaction(tx: FoundryTxEnvelope) -> FoundryTxEnvelope {
    match tx {
        FoundryTxEnvelope::Eip4844(tx) => {
            FoundryTxEnvelope::Eip4844(tx.map(TxEip4844Variant::drop_sidecar))
        }
        tx => tx,
    }
}

/// Returns a block whose transactions use canonical block-body representations.
pub fn canonical_block(mut block: Block) -> Block {
    block.body.transactions =
        block.body.transactions.into_iter().map(|tx| tx.map(canonical_block_transaction)).collect();
    block
}

/// Helper function to create a new block with Header and Anvil transactions, generic over the
/// transaction envelope with a default of [`FoundryTxEnvelope`].
///
/// Note: if the `impersonate-tx` feature is enabled this will also accept
/// `MaybeImpersonatedTransaction`.
pub fn create_block<T, Tx>(
    mut header: FoundryHeader,
    transactions: impl IntoIterator<Item = T>,
) -> Block<Tx>
where
    Tx: EncodableBlockTransaction,
    T: Into<MaybeImpersonatedTransaction<Tx>>,
{
    let transactions: Vec<_> = transactions.into_iter().map(Into::into).collect();
    let transactions_root = ordered_trie_root_with_encoder(&transactions, |tx, out| {
        tx.as_ref().encode_2718_for_block(out)
    });

    header.set_transactions_root(transactions_root);
    header.set_ommers_hash(EMPTY_OMMER_ROOT_HASH);

    let body = BlockBody { transactions, ommers: Vec::new(), withdrawals: None };
    Block::new(header, body)
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{
        BlobTransactionSidecar, BlobTransactionSidecarVariant, BlockHeader, SignableTransaction,
        TxEip4844, proofs::calculate_transaction_root,
    };
    use alloy_primitives::{
        Address, B64, B256, Bloom, Signature, U256, b256,
        hex::{self, FromHex},
    };
    use alloy_rlp::Decodable;

    use super::*;
    use std::str::FromStr;

    fn assert_blob_transaction_root(sidecar: BlobTransactionSidecarVariant) {
        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 1,
            to: Address::ZERO,
            value: U256::ZERO,
            access_list: Default::default(),
            blob_versioned_hashes: vec![B256::ZERO],
            max_fee_per_blob_gas: 1,
            input: Default::default(),
        };
        let signature = Signature::new(U256::from(1), U256::from(1), false);
        let canonical = FoundryTxEnvelope::Eip4844(
            TxEip4844Variant::TxEip4844(tx.clone()).into_signed(signature),
        );
        let pooled = FoundryTxEnvelope::Eip4844(
            TxEip4844Variant::TxEip4844WithSidecar(tx.with_sidecar(sidecar)).into_signed(signature),
        );

        let canonical_root = calculate_transaction_root(&[canonical]);
        let pooled_root = calculate_transaction_root(std::slice::from_ref(&pooled));
        let block = create_block(FoundryHeader::default(), [pooled]);

        assert_ne!(canonical_root, pooled_root);
        assert_eq!(block.header.transactions_root(), canonical_root);
    }

    #[test]
    fn blob_transaction_root_uses_canonical_encoding() {
        assert_blob_transaction_root(BlobTransactionSidecarVariant::Eip4844(
            BlobTransactionSidecar::new(
                vec![Default::default()],
                vec![Default::default()],
                vec![Default::default()],
            ),
        ));
        assert_blob_transaction_root(BlobTransactionSidecarVariant::Eip7594(Default::default()));
    }

    #[test]
    fn header_rlp_roundtrip() {
        let mut header = Header {
            parent_hash: Default::default(),
            ommers_hash: Default::default(),
            beneficiary: Default::default(),
            state_root: Default::default(),
            transactions_root: Default::default(),
            receipts_root: Default::default(),
            logs_bloom: Default::default(),
            difficulty: Default::default(),
            number: 124u64,
            gas_limit: Default::default(),
            gas_used: 1337u64,
            timestamp: 0,
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: B64::with_last_byte(99),
            withdrawals_root: Default::default(),
            blob_gas_used: Default::default(),
            excess_blob_gas: Default::default(),
            parent_beacon_block_root: Default::default(),
            base_fee_per_gas: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
        };

        let encoded = alloy_rlp::encode(&header);
        let decoded: Header = Header::decode(&mut encoded.as_ref()).unwrap();
        assert_eq!(header, decoded);

        header.base_fee_per_gas = Some(12345u64);

        let encoded = alloy_rlp::encode(&header);
        let decoded: Header = Header::decode(&mut encoded.as_ref()).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn test_encode_block_header() {
        use alloy_rlp::Encodable;

        let expected = hex::decode("f901f9a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000940000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008208ae820d0582115c8215b3821a0a827788a00000000000000000000000000000000000000000000000000000000000000000880000000000000000").unwrap();
        let mut data = vec![];
        let header = Header {
            parent_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            ommers_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            beneficiary: Address::from_str("0000000000000000000000000000000000000000").unwrap(),
            state_root: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            transactions_root: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            receipts_root: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            logs_bloom: Bloom::from_hex("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            difficulty: U256::from(2222),
            number: 0xd05u64,
            gas_limit: 0x115cu64,
            gas_used: 0x15b3u64,
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            nonce: B64::ZERO,
            base_fee_per_gas: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
        };

        header.encode(&mut data);
        assert_eq!(hex::encode(&data), hex::encode(expected));
        assert_eq!(header.length(), data.len());
    }

    #[test]
    // Test vector from: https://eips.ethereum.org/EIPS/eip-2481
    fn test_decode_block_header() {
        let data = hex::decode("f901f9a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000940000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008208ae820d0582115c8215b3821a0a827788a00000000000000000000000000000000000000000000000000000000000000000880000000000000000").unwrap();
        let expected = Header {
            parent_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            ommers_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            beneficiary: Address::from_str("0000000000000000000000000000000000000000").unwrap(),
            state_root: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            transactions_root: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            receipts_root: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            logs_bloom: <[u8; 256]>::from_hex("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap().into(),
            difficulty: U256::from(2222),
            number: 0xd05u64,
            gas_limit: 0x115cu64,
            gas_used: 0x15b3u64,
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            nonce: B64::ZERO,
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            base_fee_per_gas: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
        };
        let header = Header::decode(&mut data.as_slice()).unwrap();
        assert_eq!(header, expected);
    }

    #[test]
    // Test vector from: https://github.com/ethereum/tests/blob/f47bbef4da376a49c8fc3166f09ab8a6d182f765/BlockchainTests/ValidBlocks/bcEIP1559/baseFee.json#L15-L36
    fn test_eip1559_block_header_hash() {
        let expected_hash =
            b256!("0x6a251c7c3c5dca7b42407a3752ff48f3bbca1fab7f9868371d9918daf1988d1f");
        let header = Header {
            parent_hash: B256::from_str("e0a94a7a3c9617401586b1a27025d2d9671332d22d540e0af72b069170380f2a").unwrap(),
            ommers_hash: B256::from_str("1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347").unwrap(),
            beneficiary: Address::from_str("ba5e000000000000000000000000000000000000").unwrap(),
            state_root: B256::from_str("ec3c94b18b8a1cff7d60f8d258ec723312932928626b4c9355eb4ab3568ec7f7").unwrap(),
            transactions_root: B256::from_str("50f738580ed699f0469702c7ccc63ed2e51bc034be9479b7bff4e68dee84accf").unwrap(),
            receipts_root: B256::from_str("29b0562f7140574dd0d50dee8a271b22e1a0a7b78fca58f7c60370d8317ba2a9").unwrap(),
            logs_bloom: Bloom::from_hex("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            difficulty: U256::from(0x020000),
            number: 1u64,
            gas_limit: U256::from(0x016345785d8a0000u128).to::<u64>(),
            gas_used: U256::from(0x015534).to::<u64>(),
            timestamp: 0x079e,
            extra_data: hex::decode("42").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            nonce: B64::ZERO,
            base_fee_per_gas: Some(875),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            block_access_list_hash: None,
            slot_number: None,
        };
        assert_eq!(header.hash_slow(), expected_hash);
    }

    #[test]
    // Test vector from network
    fn block_network_roundtrip() {
        use alloy_rlp::Encodable;

        let data = hex::decode("f9034df90348a0fbdbd8d2d0ac5f14bd5fa90e547fe6f1d15019c724f8e7b60972d381cd5d9cf8a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794c9577e7945db22e38fc060909f2278c7746b0f9ba05017cfa3b0247e35197215ae8d610265ffebc8edca8ea66d6567eb0adecda867a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000018355bb7b871fffffffffffff808462bd0e1ab9014bf90148a00000000000000000000000000000000000000000000000000000000000000000f85494319fa8f1bc4e53410e92d10d918659b16540e60a945a573efb304d04c1224cd012313e827eca5dce5d94a9c831c5a268031176ebf5f3de5051e8cba0dbfe94c9577e7945db22e38fc060909f2278c7746b0f9b808400000000f8c9b841a6946f2d16f68338cbcbd8b117374ab421128ce422467088456bceba9d70c34106128e6d4564659cf6776c08a4186063c0a05f7cffd695c10cf26a6f301b67f800b8412b782100c18c35102dc0a37ece1a152544f04ad7dc1868d18a9570f744ace60870f822f53d35e89a2ea9709ccbf1f4a25ee5003944faa845d02dde0a41d5704601b841d53caebd6c8a82456e85c2806a9e08381f959a31fb94a77e58f00e38ad97b2e0355b8519ab2122662cbe022f2a4ef7ff16adc0b2d5dcd123181ec79705116db300a063746963616c2062797a616e74696e65206661756c7420746f6c6572616e6365880000000000000000c0c0").unwrap();

        let block = <Block>::decode(&mut data.as_slice()).unwrap();

        // encode and check that it matches the original data
        let mut encoded = Vec::new();
        block.encode(&mut encoded);
        assert_eq!(data, encoded);

        // check that length of encoding is the same as the output of `length`
        assert_eq!(block.length(), encoded.len());
    }
}
