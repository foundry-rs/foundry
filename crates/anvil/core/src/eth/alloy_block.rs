use alloy_consensus::Header;
use alloy_primitives::{Address, Bloom, Bytes, B256, U256};
use alloy_rlp::{RlpEncodable, RlpDecodable};
use foundry_common::types::ToAlloy;
use super::trie;


// Type alias to optionally support impersonated transactions
#[cfg(not(feature = "impersonated-tx"))]
type Transaction = crate::eth::transaction::alloy::TypedTransaction;
#[cfg(feature = "impersonated-tx")]
type Transaction = crate::eth::transaction::alloy::MaybeImpersonatedTransaction;

/// An Ethereum Block
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(RlpEncodable, RlpDecodable)]
pub struct Block {
    pub header: Header,
    pub transactions: Vec<Transaction>,
    pub ommers: Vec<Header>,
}

impl Block {
    /// Creates a new block
    ///
    /// Note: if the `impersonate-tx` feature is enabled this  will also accept
    /// [MaybeImpersonatedTransaction]
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
        let ommers_hash = B256::from_slice(alloy_primitives::utils::keccak256(encoded_ommers).as_slice());
        let transactions_root =
            trie::ordered_trie_root(transactions.iter().map(|r| Bytes::from(alloy_rlp::encode(r)))).to_alloy();

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
                withdrawals_root: Some(partial_header.mix_hash),
                blob_gas_used: None,
                excess_blob_gas: None,
                parent_beacon_block_root: None,
                nonce: partial_header.nonce,
                base_fee_per_gas: partial_header.base_fee,
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
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: Bytes,
    pub mix_hash: B256,
    pub nonce: u64,
    pub base_fee: Option<u64>,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::hex::{FromHex, self};
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
            number: 124u64.into(),
            gas_limit: Default::default(),
            gas_used: 1337u64.into(),
            timestamp: 0,
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: 99u64,
            withdrawals_root: Default::default(),
            blob_gas_used: Default::default(),
            excess_blob_gas: Default::default(),
            parent_beacon_block_root: Default::default(),
            base_fee_per_gas: None,
        };

        let encoded = alloy_rlp::encode(&header);
        let decoded: Header = <Header as Decodable>::decode(&mut encoded.as_ref()).unwrap();
        assert_eq!(header, decoded);

        header.base_fee_per_gas = Some(12345u64.into());

        let encoded = alloy_rlp::encode(&header);
        let decoded: Header = <Header as Decodable>::decode(&mut encoded.as_ref()).unwrap();
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
            logs_bloom: Bloom::from_hex("00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap().into(),
            difficulty: U256::from(2222),
            number: 0xd05u64.into(),
            gas_limit: 0x115cu64.into(),
            gas_used: 0x15b3u64.into(),
            timestamp: 0x1a0au64,
            extra_data: hex::decode("7788").unwrap().into(),
            mix_hash: B256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            nonce: 0,
            base_fee_per_gas: None,
        };

        header.encode(&mut data);
        assert_eq!(hex::encode(&data), hex::encode(expected));
        assert_eq!(header.length(), data.len());
    }
}