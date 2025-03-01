//! Ethereum log object.

use alloy_primitives::{Address, Bloom, Bytes, B256};
use alloy_rlp::{RlpDecodable, RlpEncodable};

use alloc::vec::Vec;

/// Ethereum Log
#[derive(Clone, Debug, Default, PartialEq, Eq, RlpDecodable, RlpEncodable)]
pub struct Log {
    /// Contract that emitted this log.
    pub address: Address,
    /// Topics of the log. The number of logs depend on what `LOG` opcode is used.
    pub topics: Vec<B256>,
    /// Arbitrary length data.
    pub data: Bytes,
}

/// Calculate receipt logs bloom.
pub fn logs_bloom<'a, It>(logs: It) -> Bloom
where
    It: IntoIterator<Item = &'a Log>,
{
    let mut bloom = Bloom::ZERO;
    for log in logs {
        bloom.m3_2048(log.address.as_slice());
        for topic in &log.topics {
            bloom.m3_2048(topic.as_slice());
        }
    }
    bloom
}
