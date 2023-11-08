//! Support types for configuring storage caching

use crate::chain::Chain;
use number_prefix::NumberPrefix;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, fmt::Formatter, str::FromStr};

/// Settings to configure caching of remote
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StorageCachingConfig {
    /// chains to cache
    pub chains: CachedChains,
    /// endpoints to cache
    pub endpoints: CachedEndpoints,
}

impl StorageCachingConfig {
    /// Whether caching should be enabled for the endpoint
    pub fn enable_for_endpoint(&self, endpoint: impl AsRef<str>) -> bool {
        self.endpoints.is_match(endpoint)
    }

    /// Whether caching should be enabled for the chain id
    pub fn enable_for_chain_id(&self, chain_id: u64) -> bool {
        // ignore dev chains
        if [99, 1337, 31337].contains(&chain_id) {
            return false
        }
        self.chains.is_match(chain_id)
    }
}

/// What chains to cache
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CachedChains {
    /// Cache all chains
    #[default]
    All,
    /// Don't cache anything
    None,
    /// Only cache these chains
    Chains(Vec<Chain>),
}
impl CachedChains {
    /// Whether the `endpoint` matches
    pub fn is_match(&self, chain: u64) -> bool {
        match self {
            CachedChains::All => true,
            CachedChains::None => false,
            CachedChains::Chains(chains) => chains.iter().any(|c| c.id() == chain),
        }
    }
}

impl Serialize for CachedChains {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CachedChains::All => serializer.serialize_str("all"),
            CachedChains::None => serializer.serialize_str("none"),
            CachedChains::Chains(chains) => chains.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for CachedChains {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Chains {
            All(String),
            Chains(Vec<Chain>),
        }

        match Chains::deserialize(deserializer)? {
            Chains::All(s) => match s.as_str() {
                "all" => Ok(CachedChains::All),
                "none" => Ok(CachedChains::None),
                s => Err(serde::de::Error::unknown_variant(s, &["all", "none"])),
            },
            Chains::Chains(chains) => Ok(CachedChains::Chains(chains)),
        }
    }
}

/// What endpoints to enable caching for
#[derive(Debug, Clone, Default)]
pub enum CachedEndpoints {
    /// Cache all endpoints
    #[default]
    All,
    /// Only cache non-local host endpoints
    Remote,
    /// Only cache these chains
    Pattern(regex::Regex),
}

impl CachedEndpoints {
    /// Whether the `endpoint` matches
    pub fn is_match(&self, endpoint: impl AsRef<str>) -> bool {
        let endpoint = endpoint.as_ref();
        match self {
            CachedEndpoints::All => true,
            CachedEndpoints::Remote => {
                !endpoint.contains("localhost:") && !endpoint.contains("127.0.0.1:")
            }
            CachedEndpoints::Pattern(re) => re.is_match(endpoint),
        }
    }
}

impl PartialEq for CachedEndpoints {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CachedEndpoints::Pattern(a), CachedEndpoints::Pattern(b)) => a.as_str() == b.as_str(),
            (&CachedEndpoints::All, &CachedEndpoints::All) => true,
            (&CachedEndpoints::Remote, &CachedEndpoints::Remote) => true,
            _ => false,
        }
    }
}

impl Eq for CachedEndpoints {}

impl fmt::Display for CachedEndpoints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CachedEndpoints::All => f.write_str("all"),
            CachedEndpoints::Remote => f.write_str("remote"),
            CachedEndpoints::Pattern(s) => s.fmt(f),
        }
    }
}

impl FromStr for CachedEndpoints {
    type Err = regex::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(CachedEndpoints::All),
            "remote" => Ok(CachedEndpoints::Remote),
            _ => Ok(CachedEndpoints::Pattern(s.parse()?)),
        }
    }
}

impl<'de> Deserialize<'de> for CachedEndpoints {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for CachedEndpoints {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CachedEndpoints::All => serializer.serialize_str("all"),
            CachedEndpoints::Remote => serializer.serialize_str("remote"),
            CachedEndpoints::Pattern(pattern) => serializer.serialize_str(pattern.as_str()),
        }
    }
}

/// Content of the foundry cache folder
#[derive(Debug, Default)]
pub struct Cache {
    /// The list of chains in the cache
    pub chains: Vec<ChainCache>,
}

impl fmt::Display for Cache {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for chain in &self.chains {
            match NumberPrefix::decimal(
                chain.block_explorer as f32 + chain.blocks.iter().map(|x| x.1).sum::<u64>() as f32,
            ) {
                NumberPrefix::Standalone(size) => {
                    writeln!(f, "-️ {} ({size:.1} B)", chain.name)?;
                }
                NumberPrefix::Prefixed(prefix, size) => {
                    writeln!(f, "-️ {} ({size:.1} {prefix}B)", chain.name)?;
                }
            }
            match NumberPrefix::decimal(chain.block_explorer as f32) {
                NumberPrefix::Standalone(size) => {
                    writeln!(f, "\t-️ Block Explorer ({size:.1} B)\n")?;
                }
                NumberPrefix::Prefixed(prefix, size) => {
                    writeln!(f, "\t-️ Block Explorer ({size:.1} {prefix}B)\n")?;
                }
            }
            for block in &chain.blocks {
                match NumberPrefix::decimal(block.1 as f32) {
                    NumberPrefix::Standalone(size) => {
                        writeln!(f, "\t-️ Block {} ({size:.1} B)", block.0)?;
                    }
                    NumberPrefix::Prefixed(prefix, size) => {
                        writeln!(f, "\t-️ Block {} ({size:.1} {prefix}B)", block.0)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// A representation of data for a given chain in the foundry cache
#[derive(Debug)]
pub struct ChainCache {
    /// The name of the chain
    pub name: String,

    /// A tuple containing block number and the block directory size in bytes
    pub blocks: Vec<(String, u64)>,

    /// The size of the block explorer directory in bytes
    pub block_explorer: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::NamedChain;
    use pretty_assertions::assert_str_eq;

    #[test]
    fn can_parse_storage_config() {
        #[derive(Serialize, Deserialize)]
        pub struct Wrapper {
            pub rpc_storage_caching: StorageCachingConfig,
        }

        let s = r#"rpc_storage_caching = { chains = "all", endpoints = "remote"}"#;
        let w: Wrapper = toml::from_str(s).unwrap();

        assert_eq!(
            w.rpc_storage_caching,
            StorageCachingConfig { chains: CachedChains::All, endpoints: CachedEndpoints::Remote }
        );

        let s = r#"rpc_storage_caching = { chains = [1, "optimism", 999999], endpoints = "all"}"#;
        let w: Wrapper = toml::from_str(s).unwrap();

        assert_eq!(
            w.rpc_storage_caching,
            StorageCachingConfig {
                chains: CachedChains::Chains(vec![
                    Chain::Named(NamedChain::Mainnet),
                    Chain::Named(NamedChain::Optimism),
                    Chain::Id(999999)
                ]),
                endpoints: CachedEndpoints::All
            }
        )
    }

    #[test]
    fn cache_to_string() {
        let cache = Cache {
            chains: vec![
                ChainCache {
                    name: "mainnet".to_string(),
                    blocks: vec![("1".to_string(), 1), ("2".to_string(), 2)],
                    block_explorer: 500,
                },
                ChainCache {
                    name: "ropsten".to_string(),
                    blocks: vec![("1".to_string(), 1), ("2".to_string(), 2)],
                    block_explorer: 4567,
                },
                ChainCache {
                    name: "rinkeby".to_string(),
                    blocks: vec![("1".to_string(), 1032), ("2".to_string(), 2000000)],
                    block_explorer: 4230000,
                },
                ChainCache {
                    name: "mumbai".to_string(),
                    blocks: vec![("1".to_string(), 1), ("2".to_string(), 2)],
                    block_explorer: 0,
                },
            ],
        };

        let expected = "\
            -️ mainnet (503.0 B)\n\t\
                -️ Block Explorer (500.0 B)\n\n\t\
                -️ Block 1 (1.0 B)\n\t\
                -️ Block 2 (2.0 B)\n\
            -️ ropsten (4.6 kB)\n\t\
                -️ Block Explorer (4.6 kB)\n\n\t\
                -️ Block 1 (1.0 B)\n\t\
                -️ Block 2 (2.0 B)\n\
            -️ rinkeby (6.2 MB)\n\t\
                -️ Block Explorer (4.2 MB)\n\n\t\
                -️ Block 1 (1.0 kB)\n\t\
                -️ Block 2 (2.0 MB)\n\
            -️ mumbai (3.0 B)\n\t\
                -️ Block Explorer (0.0 B)\n\n\t\
                -️ Block 1 (1.0 B)\n\t\
                -️ Block 2 (2.0 B)\n";
        assert_str_eq!(format!("{cache}"), expected);
    }
}
