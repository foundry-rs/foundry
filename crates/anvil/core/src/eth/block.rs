use super::{
    transaction::{TransactionInfo, TypedReceipt},
    trie,
};
use alloy_consensus::Header;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bloom, Bytes, B256, B64, U256};
use alloy_rlp::{RlpDecodable, RlpEncodable};

// Type alias to optionally support impersonated transactions
#[cfg(not(feature = "impersonated-tx"))]
type Transaction = crate::eth::transaction::TypedTransaction;
#[cfg(feature = "impersonated-tx")]
type Transaction = crate::eth::transaction::MaybeImpersonatedTransaction;

/// Container type that gathers all block data
#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub block: Block,
    pub transactions: Vec<TransactionInfo>,
    pub receipts: Vec<TypedReceipt>,
}

/// An Ethereum Block
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct Block {
    pub header: Header,
    pub transactions: Vec<Transaction>,
    pub ommers: Vec<Header>,
}

impl Block {
    /// Creates a new block.
    ///
    /// Note: if the `impersonate-tx` feature is enabled this will also accept
    /// `MaybeImpersonatedTransaction`.
    pub fn new<T>(
        partial_header: PartialHeader,
        transactions: impl IntoIterator<Item = T>,
        ommers: Vec<Header>,
    ) -> Self
    where
        T: Into<Transaction>,
    {
        let transactions: Vec<_> = transactions.into_iter().map(Into::into).collect();
        let mut encoded_ommers: Vec<u8> = Vec::new();
        alloy_rlp::encode_list(&ommers, &mut encoded_ommers);
        let ommers_hash =
            B256::from_slice(alloy_primitives::utils::keccak256(encoded_ommers).as_slice());
        let transactions_root =
            trie::ordered_trie_root(transactions.iter().map(|r| r.encoded_2718()));

        Self {
            header: Header {
                parent_hash: partial_header.parent_hash,
                beneficiary: partial_header.beneficiary,
                ommers_hash,
                state_root: partial_header.state_root,
                transactions_root,
                receipts_root: partial_header.receipts_root,
                logs_bloom: partial_header.logs_bloom,
                difficulty: partial_header.difficulty,
                number: partial_header.number,
                gas_limit: partial_header.gas_limit,
                gas_used: partial_header.gas_used,
                timestamp: partial_header.timestamp,
                extra_data: partial_header.extra_data,
                mix_hash: partial_header.mix_hash,
                withdrawals_root: None,
                blob_gas_used: partial_header.blob_gas_used,
                excess_blob_gas: partial_header.excess_blob_gas,
                parent_beacon_block_root: partial_header.parent_beacon_block_root,
                nonce: partial_header.nonce,
                base_fee_per_gas: partial_header.base_fee,
                requests_root: None,
            },
            transactions,
            ommers,
        }
    }
}

/// Partial header definition without ommers hash and transactions root
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartialHeader {
    pub parent_hash: B256,
    pub beneficiary: Address,
    pub state_root: B256,
    pub receipts_root: B256,
    pub logs_bloom: Bloom,
    pub difficulty: U256,
    pub number: u64,
    pub gas_limit: u128,
    pub gas_used: u128,
    pub timestamp: u64,
    pub extra_data: Bytes,
    pub mix_hash: B256,
    pub blob_gas_used: Option<u128>,
    pub excess_blob_gas: Option<u128>,
    pub parent_beacon_block_root: Option<B256>,
    pub nonce: B64,
    pub base_fee: Option<u128>,
}

impl From<Header> for PartialHeader {
    fn from(value: Header) -> Self {
        Self {
            parent_hash: value.parent_hash,
            beneficiary: value.beneficiary,
            state_root: value.state_root,
            receipts_root: value.receipts_root,
            logs_bloom: value.logs_bloom,
            difficulty: value.difficulty,
            number: value.number,
            gas_limit: value.gas_limit,
            gas_used: value.gas_used,
            timestamp: value.timestamp,
            extra_data: value.extra_data,
            mix_hash: value.mix_hash,
            nonce: value.nonce,
            base_fee: value.base_fee_per_gas,
            blob_gas_used: value.blob_gas_used,
            excess_blob_gas: value.excess_blob_gas,
            parent_beacon_block_root: value.parent_beacon_block_root,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{
        b256,
        hex::{self, FromHex},
    };
    use alloy_rlp::Decodable;

    use super::*;
    use std::str::FromStr;

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
            gas_used: 1337u128,
            timestamp: 0,
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: B64::with_last_byte(99),
            withdrawals_root: Default::default(),
            blob_gas_used: Default::default(),
            excess_blob_gas: Default::default(),
            parent_beacon_block_root: Default::default(),
            base_fee_per_gas: None,
            requests_root: None,
        };

        let encoded = alloy_rlp::encode(&header);
        let decoded: Header = Header::decode(&mut encoded.as_ref()).unwrap();
        assert_eq!(header, decoded);

        header.base_fee_per_gas = Some(12345u128);

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
            gas_limit: 0x115cu128,
            gas_used: 0x15b3u128,
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            nonce: B64::ZERO,
            base_fee_per_gas: None,
            requests_root: None,
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
            gas_limit: 0x115cu128,
            gas_used: 0x15b3u128,
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            nonce: B64::ZERO,
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            base_fee_per_gas: None,
            requests_root: None,
        };
        let header = Header::decode(&mut data.as_slice()).unwrap();
        assert_eq!(header, expected);
    }

    #[test]
    // Test vector from: https://github.com/ethereum/tests/blob/f47bbef4da376a49c8fc3166f09ab8a6d182f765/BlockchainTests/ValidBlocks/bcEIP1559/baseFee.json#L15-L36
    fn test_eip1559_block_header_hash() {
        let expected_hash =
            b256!("6a251c7c3c5dca7b42407a3752ff48f3bbca1fab7f9868371d9918daf1988d1f");
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
            gas_limit: U256::from(0x016345785d8a0000u128).to::<u128>(),
            gas_used: U256::from(0x015534).to::<u128>(),
            timestamp: 0x079e,
            extra_data: hex::decode("42").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            nonce: B64::ZERO,
            base_fee_per_gas: Some(875),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_root: None,
        };
        assert_eq!(header.hash_slow(), expected_hash);
    }

    #[test]
    // Test vector from network
    fn block_network_roundtrip() {
        use alloy_rlp::Encodable;

        let data = hex::decode("f9034df90348a0fbdbd8d2d0ac5f14bd5fa90e547fe6f1d15019c724f8e7b60972d381cd5d9cf8a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794c9577e7945db22e38fc060909f2278c7746b0f9ba05017cfa3b0247e35197215ae8d610265ffebc8edca8ea66d6567eb0adecda867a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000018355bb7b871fffffffffffff808462bd0e1ab9014bf90148a00000000000000000000000000000000000000000000000000000000000000000f85494319fa8f1bc4e53410e92d10d918659b16540e60a945a573efb304d04c1224cd012313e827eca5dce5d94a9c831c5a268031176ebf5f3de5051e8cba0dbfe94c9577e7945db22e38fc060909f2278c7746b0f9b808400000000f8c9b841a6946f2d16f68338cbcbd8b117374ab421128ce422467088456bceba9d70c34106128e6d4564659cf6776c08a4186063c0a05f7cffd695c10cf26a6f301b67f800b8412b782100c18c35102dc0a37ece1a152544f04ad7dc1868d18a9570f744ace60870f822f53d35e89a2ea9709ccbf1f4a25ee5003944faa845d02dde0a41d5704601b841d53caebd6c8a82456e85c2806a9e08381f959a31fb94a77e58f00e38ad97b2e0355b8519ab2122662cbe022f2a4ef7ff16adc0b2d5dcd123181ec79705116db300a063746963616c2062797a616e74696e65206661756c7420746f6c6572616e6365880000000000000000c0c0").unwrap();

        let block = Block::decode(&mut data.as_slice()).unwrap();

        // encode and check that it matches the original data
        let mut encoded = Vec::new();
        block.encode(&mut encoded);
        assert_eq!(data, encoded);

        // check that length of encoding is the same as the output of `length`
        assert_eq!(block.length(), encoded.len());
    }
}
