use alloy_consensus::{BlockHeader, Header, Sealable};
use alloy_primitives::{Address, B64, B256, Bloom, Bytes, U256, keccak256};
use alloy_rlp::{Decodable, Encodable};
use serde::{Deserialize, Serialize};

/// Foundry block header - superset supporting both Ethereum and Tempo.
/// Uses Ethereum Header as base and adds optional Tempo-specific fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundryHeader {
    /// Inner Ethereum header (flattened in JSON)
    #[serde(flatten)]
    pub inner: Header,

    /// Tempo: Non-payment gas limit for the block
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "mainBlockGeneralGasLimit",
        with = "alloy_serde::quantity::opt"
    )]
    pub general_gas_limit: Option<u64>,

    /// Tempo: Shared gas limit for subblocks
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub shared_gas_limit: Option<u64>,

    /// Tempo: Sub-second milliseconds portion of timestamp
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub timestamp_millis_part: Option<u64>,
}

impl FoundryHeader {
    /// Returns true if this is a Tempo header (has Tempo-specific fields)
    pub fn is_tempo(&self) -> bool {
        self.general_gas_limit.is_some()
            || self.shared_gas_limit.is_some()
            || self.timestamp_millis_part.is_some()
    }

    /// Returns the timestamp in milliseconds (for Tempo) or seconds*1000 (for Eth)
    pub fn timestamp_millis(&self) -> u64 {
        let base = self.inner.timestamp().saturating_mul(1000);
        base.saturating_add(self.timestamp_millis_part.unwrap_or(0))
    }
}

impl From<Header> for FoundryHeader {
    fn from(header: Header) -> Self {
        Self { inner: header, ..Default::default() }
    }
}

impl BlockHeader for FoundryHeader {
    fn parent_hash(&self) -> B256 {
        self.inner.parent_hash()
    }

    fn ommers_hash(&self) -> B256 {
        self.inner.ommers_hash()
    }

    fn beneficiary(&self) -> Address {
        self.inner.beneficiary()
    }

    fn state_root(&self) -> B256 {
        self.inner.state_root()
    }

    fn transactions_root(&self) -> B256 {
        self.inner.transactions_root()
    }

    fn receipts_root(&self) -> B256 {
        self.inner.receipts_root()
    }

    fn withdrawals_root(&self) -> Option<B256> {
        self.inner.withdrawals_root()
    }

    fn logs_bloom(&self) -> Bloom {
        self.inner.logs_bloom()
    }

    fn difficulty(&self) -> U256 {
        self.inner.difficulty()
    }

    fn number(&self) -> u64 {
        self.inner.number()
    }

    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }

    fn gas_used(&self) -> u64 {
        self.inner.gas_used()
    }

    fn timestamp(&self) -> u64 {
        self.inner.timestamp()
    }

    fn mix_hash(&self) -> Option<B256> {
        self.inner.mix_hash()
    }

    fn nonce(&self) -> Option<B64> {
        self.inner.nonce()
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.inner.base_fee_per_gas()
    }

    fn blob_gas_used(&self) -> Option<u64> {
        self.inner.blob_gas_used()
    }

    fn excess_blob_gas(&self) -> Option<u64> {
        self.inner.excess_blob_gas()
    }

    fn parent_beacon_block_root(&self) -> Option<B256> {
        self.inner.parent_beacon_block_root()
    }

    fn requests_hash(&self) -> Option<B256> {
        self.inner.requests_hash()
    }

    fn extra_data(&self) -> &Bytes {
        self.inner.extra_data()
    }
}

impl Encodable for FoundryHeader {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        if self.is_tempo() {
            let general_gas_limit = self.general_gas_limit.unwrap_or(0);
            let shared_gas_limit = self.shared_gas_limit.unwrap_or(0);
            let timestamp_millis_part = self.timestamp_millis_part.unwrap_or(0);

            alloy_rlp::Header {
                list: true,
                payload_length: self.inner.length()
                    + general_gas_limit.length()
                    + shared_gas_limit.length()
                    + timestamp_millis_part.length(),
            }
            .encode(out);
            self.inner.encode(out);
            general_gas_limit.encode(out);
            shared_gas_limit.encode(out);
            timestamp_millis_part.encode(out);
        } else {
            self.inner.encode(out);
        }
    }

    fn length(&self) -> usize {
        if self.is_tempo() {
            let general_gas_limit = self.general_gas_limit.unwrap_or(0);
            let shared_gas_limit = self.shared_gas_limit.unwrap_or(0);
            let timestamp_millis_part = self.timestamp_millis_part.unwrap_or(0);

            let payload_length = self.inner.length()
                + general_gas_limit.length()
                + shared_gas_limit.length()
                + timestamp_millis_part.length();
            alloy_rlp::Header { list: true, payload_length }.length() + payload_length
        } else {
            self.inner.length()
        }
    }
}

impl Decodable for FoundryHeader {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let original = *buf;
        if let Ok(header) = Header::decode(buf) {
            return Ok(Self::from(header));
        }

        *buf = original;
        let header = alloy_rlp::Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }

        let inner = Header::decode(buf)?;
        let general_gas_limit = u64::decode(buf)?;
        let shared_gas_limit = u64::decode(buf)?;
        let timestamp_millis_part = u64::decode(buf)?;

        Ok(Self {
            inner,
            general_gas_limit: Some(general_gas_limit),
            shared_gas_limit: Some(shared_gas_limit),
            timestamp_millis_part: Some(timestamp_millis_part),
        })
    }
}

impl Sealable for FoundryHeader {
    fn hash_slow(&self) -> B256 {
        keccak256(alloy_rlp::encode(self))
    }
}

impl AsRef<Self> for FoundryHeader {
    fn as_ref(&self) -> &Self {
        self
    }
}
