use crate::U256;
use ethers_core::types::{Chain as NamedChain, ParseChainError, U64};
use fastrlp::{Decodable, Encodable};
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, str::FromStr, time::Duration};

/// Either a named or chain id or the actual id value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum Chain {
    /// Contains a known chain
    #[serde(serialize_with = "super::from_str_lowercase::serialize")]
    Named(NamedChain),
    /// Contains the id of a chain
    Id(u64),
}

impl Chain {
    /// The id of the chain
    pub fn id(&self) -> u64 {
        match self {
            Chain::Named(chain) => *chain as u64,
            Chain::Id(id) => *id,
        }
    }

    /// Helper function for checking if a chainid corresponds to a legacy chainid
    /// without eip1559
    pub fn is_legacy(&self) -> bool {
        match self {
            Chain::Named(c) => c.is_legacy(),
            Chain::Id(_) => false,
        }
    }

    /// The blocktime various from chain to chain
    ///
    /// It can be beneficial to know the average blocktime to adjust the polling of an Http provider
    /// for example.
    ///
    /// **Note:** this will not return the accurate average depending on the time but is rather a
    /// sensible default derived from blocktime charts like <https://etherscan.com/chart/blocktime>
    /// <https://polygonscan.com/chart/blocktime>
    pub fn average_blocktime_hint(&self) -> Option<Duration> {
        if let Chain::Named(chain) = self {
            let ms = match chain {
                NamedChain::Mainnet => 13_000,
                NamedChain::Polygon | NamedChain::PolygonMumbai => 2_100,
                NamedChain::Moonbeam | NamedChain::Moonriver => 12_500,
                NamedChain::BinanceSmartChain | NamedChain::BinanceSmartChainTestnet => 3_000,
                NamedChain::Dev | NamedChain::AnvilHardhat => 200,
                _ => return None,
            };

            return Some(Duration::from_millis(ms))
        }
        None
    }
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Chain::Named(chain) => chain.fmt(f),
            Chain::Id(id) => {
                if let Ok(chain) = NamedChain::try_from(*id) {
                    chain.fmt(f)
                } else {
                    id.fmt(f)
                }
            }
        }
    }
}

impl From<NamedChain> for Chain {
    fn from(id: NamedChain) -> Self {
        Chain::Named(id)
    }
}

impl From<u64> for Chain {
    fn from(id: u64) -> Self {
        NamedChain::try_from(id).map(Chain::Named).unwrap_or_else(|_| Chain::Id(id))
    }
}

impl From<U256> for Chain {
    fn from(id: U256) -> Self {
        id.as_u64().into()
    }
}

impl From<Chain> for u64 {
    fn from(c: Chain) -> Self {
        match c {
            Chain::Named(c) => c as u64,
            Chain::Id(id) => id,
        }
    }
}

impl From<Chain> for U64 {
    fn from(c: Chain) -> Self {
        u64::from(c).into()
    }
}

impl From<Chain> for U256 {
    fn from(c: Chain) -> Self {
        u64::from(c).into()
    }
}

impl TryFrom<Chain> for NamedChain {
    type Error = ParseChainError;

    fn try_from(chain: Chain) -> Result<Self, Self::Error> {
        match chain {
            Chain::Named(chain) => Ok(chain),
            Chain::Id(id) => id.try_into(),
        }
    }
}

impl FromStr for Chain {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(chain) = NamedChain::from_str(s) {
            Ok(Chain::Named(chain))
        } else {
            s.parse::<u64>()
                .map(Chain::Id)
                .map_err(|_| format!("Expected known chain or integer, found: {s}"))
        }
    }
}

impl<'de> Deserialize<'de> for Chain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ChainId {
            Named(String),
            Id(u64),
        }

        match ChainId::deserialize(deserializer)? {
            ChainId::Named(s) => {
                s.to_lowercase().parse().map(Chain::Named).map_err(serde::de::Error::custom)
            }
            ChainId::Id(id) => {
                Ok(NamedChain::try_from(id).map(Chain::Named).unwrap_or_else(|_| Chain::Id(id)))
            }
        }
    }
}

impl Encodable for Chain {
    fn length(&self) -> usize {
        match self {
            Self::Named(chain) => u64::from(*chain).length(),
            Self::Id(id) => id.length(),
        }
    }
    fn encode(&self, out: &mut dyn fastrlp::BufMut) {
        match self {
            Self::Named(chain) => u64::from(*chain).encode(out),
            Self::Id(id) => id.encode(out),
        }
    }
}

impl Decodable for Chain {
    fn decode(buf: &mut &[u8]) -> Result<Self, fastrlp::DecodeError> {
        Ok(u64::decode(buf)?.into())
    }
}
