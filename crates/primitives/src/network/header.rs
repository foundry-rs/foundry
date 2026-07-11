use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::{Address, B64, B256, Bloom, Bytes, Sealable, U256};
use alloy_rlp::{BufMut, Decodable, Encodable, Result};
use std::ops::Deref;
use tempo_primitives::TempoHeader;

/// Consensus header used by Foundry's multi-network tooling.
///
/// The variant order is significant for untagged serde deserialization. [`Self::Tempo`] must stay
/// first because Ethereum headers ignore unknown fields and would otherwise silently deserialize
/// Tempo state dumps as [`Self::Ethereum`].
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum FoundryHeader {
    /// Tempo consensus header.
    Tempo(TempoHeader),
    /// Ethereum consensus header.
    Ethereum(Header),
}

impl Default for FoundryHeader {
    fn default() -> Self {
        Self::Ethereum(Header::default())
    }
}

impl FoundryHeader {
    /// Creates a header for the selected network.
    pub const fn new(inner: Header, is_tempo: bool) -> Self {
        if is_tempo { Self::tempo(inner) } else { Self::Ethereum(inner) }
    }

    /// Creates a Tempo header from its Ethereum-shaped fields.
    pub const fn tempo(inner: Header) -> Self {
        Self::Tempo(TempoHeader {
            general_gas_limit: inner.gas_limit,
            shared_gas_limit: 0,
            timestamp_millis_part: 0,
            inner,
            consensus_context: None,
        })
    }

    /// Returns the Tempo header when this is a Tempo block.
    pub const fn as_tempo(&self) -> Option<&TempoHeader> {
        match self {
            Self::Tempo(header) => Some(header),
            Self::Ethereum(_) => None,
        }
    }

    /// Returns the inner Ethereum-shaped header.
    pub const fn inner(&self) -> &Header {
        match self {
            Self::Tempo(header) => &header.inner,
            Self::Ethereum(header) => header,
        }
    }

    const fn inner_mut(&mut self) -> &mut Header {
        match self {
            Self::Tempo(header) => &mut header.inner,
            Self::Ethereum(header) => header,
        }
    }

    /// Sets the transactions root shared by Ethereum and Tempo headers.
    pub const fn set_transactions_root(&mut self, transactions_root: B256) {
        self.inner_mut().transactions_root = transactions_root;
    }

    /// Sets the ommers root shared by Ethereum and Tempo headers.
    pub const fn set_ommers_hash(&mut self, ommers_hash: B256) {
        self.inner_mut().ommers_hash = ommers_hash;
    }

    /// Consumes the wrapper and returns the inner Ethereum-shaped header.
    pub fn into_inner(self) -> Header {
        match self {
            Self::Tempo(header) => header.inner,
            Self::Ethereum(header) => header,
        }
    }

    /// Computes the canonical network header hash.
    pub fn hash_slow(&self) -> B256 {
        match self {
            Self::Tempo(header) => header.hash_slow(),
            Self::Ethereum(header) => header.hash_slow(),
        }
    }
}

impl From<Header> for FoundryHeader {
    fn from(value: Header) -> Self {
        Self::Ethereum(value)
    }
}

impl From<TempoHeader> for FoundryHeader {
    fn from(value: TempoHeader) -> Self {
        Self::Tempo(value)
    }
}

impl AsRef<Self> for FoundryHeader {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl Deref for FoundryHeader {
    type Target = Header;

    fn deref(&self) -> &Self::Target {
        self.inner()
    }
}

impl Encodable for FoundryHeader {
    fn encode(&self, out: &mut dyn BufMut) {
        match self {
            Self::Tempo(header) => header.encode(out),
            Self::Ethereum(header) => header.encode(out),
        }
    }

    fn length(&self) -> usize {
        match self {
            Self::Tempo(header) => header.length(),
            Self::Ethereum(header) => header.length(),
        }
    }
}

impl Decodable for FoundryHeader {
    fn decode(buf: &mut &[u8]) -> Result<Self> {
        // Tempo headers start with scalar gas-limit fields, while Ethereum headers start with a
        // 32-byte parent hash, so trying Tempo first cannot misclassify a valid Ethereum header.
        let mut tempo_buf = *buf;
        if let Ok(header) = TempoHeader::decode(&mut tempo_buf) {
            *buf = tempo_buf;
            return Ok(Self::Tempo(header));
        }

        Header::decode(buf).map(Self::Ethereum)
    }
}

impl Sealable for FoundryHeader {
    fn hash_slow(&self) -> B256 {
        Self::hash_slow(self)
    }
}

macro_rules! delegate_header_methods {
    ($($method:ident -> $return_type:ty),+ $(,)?) => {
        $(
            fn $method(&self) -> $return_type {
                self.inner().$method()
            }
        )+
    };
}

impl BlockHeader for FoundryHeader {
    delegate_header_methods! {
        parent_hash -> B256,
        ommers_hash -> B256,
        beneficiary -> Address,
        state_root -> B256,
        transactions_root -> B256,
        receipts_root -> B256,
        withdrawals_root -> Option<B256>,
        logs_bloom -> Bloom,
        difficulty -> U256,
        number -> u64,
        gas_limit -> u64,
        gas_used -> u64,
        timestamp -> u64,
        mix_hash -> Option<B256>,
        nonce -> Option<B64>,
        base_fee_per_gas -> Option<u64>,
        blob_gas_used -> Option<u64>,
        excess_blob_gas -> Option<u64>,
        parent_beacon_block_root -> Option<B256>,
        requests_hash -> Option<B256>,
        block_access_list_hash -> Option<B256>,
        slot_number -> Option<u64>,
    }

    fn extra_data(&self) -> &Bytes {
        self.inner().extra_data()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlp_roundtrip_preserves_network_header() {
        for header in [
            Header { number: 1, ..Default::default() }.into(),
            FoundryHeader::tempo(Header { number: 2, gas_limit: 30_000_000, ..Default::default() }),
        ] {
            let encoded = alloy_rlp::encode(&header);
            let decoded = FoundryHeader::decode(&mut encoded.as_ref()).unwrap();

            assert_eq!(decoded, header);
            assert_eq!(decoded.hash_slow(), header.hash_slow());
            if let Some(tempo) = header.as_tempo() {
                assert_eq!(header.hash_slow(), tempo.hash_slow());
            }
        }
    }

    #[test]
    fn serde_roundtrip_preserves_tempo_fields() {
        let header =
            FoundryHeader::tempo(Header { number: 1, gas_limit: 30_000_000, ..Default::default() });
        let value = serde_json::to_value(&header).unwrap();

        assert_eq!(value["mainBlockGeneralGasLimit"], "0x1c9c380");
        assert_eq!(serde_json::from_value::<FoundryHeader>(value).unwrap(), header);
    }
}
