use cache::Cache;
use clap::{
    builder::{PossibleValuesParser, TypedValueParser},
    Arg, Command, Parser, Subcommand,
};
use eyre::Result;
use foundry_config::{cache, Chain, Config, NamedChain};
use std::{ffi::OsStr, str::FromStr};
use strum::VariantNames;

/// CLI arguments for `forge cache`.
#[derive(Debug, Parser)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub sub: CacheSubcommands,
}

#[derive(Debug, Subcommand)]
pub enum CacheSubcommands {
    /// Cleans cached data from the global foundry directory.
    Clean(CleanArgs),

    /// Shows cached data from the global foundry directory.
    Ls(LsArgs),
}

/// CLI arguments for `forge clean`.
#[derive(Debug, Parser)]
#[command(group = clap::ArgGroup::new("etherscan-blocks").multiple(false))]
pub struct CleanArgs {
    /// The chains to clean the cache for.
    ///
    /// Can also be "all" to clean all chains.
    #[arg(
        env = "CHAIN",
        default_value = "all",
        value_parser = ChainOrAllValueParser::default(),
    )]
    chains: Vec<ChainOrAll>,

    /// The blocks to clean the cache for.
    #[arg(
        short,
        long,
        num_args(1..),
        value_delimiter(','),
        group = "etherscan-blocks"
    )]
    blocks: Vec<u64>,

    /// Whether to clean the Etherscan cache.
    #[arg(long, group = "etherscan-blocks")]
    etherscan: bool,
}

impl CleanArgs {
    pub fn run(self) -> Result<()> {
        let Self { chains, blocks, etherscan } = self;

        for chain_or_all in chains {
            match chain_or_all {
                ChainOrAll::NamedChain(chain) => {
                    clean_chain_cache(chain, blocks.to_vec(), etherscan)?
                }
                ChainOrAll::All => {
                    if etherscan {
                        Config::clean_foundry_etherscan_cache()?;
                    } else {
                        Config::clean_foundry_cache()?
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Parser)]
pub struct LsArgs {
    /// The chains to list the cache for.
    ///
    /// Can also be "all" to list all chains.
    #[arg(
        env = "CHAIN",
        default_value = "all",
        value_parser = ChainOrAllValueParser::default(),
    )]
    chains: Vec<ChainOrAll>,
}

impl LsArgs {
    pub fn run(self) -> Result<()> {
        let Self { chains } = self;
        let mut cache = Cache::default();
        for chain_or_all in chains {
            match chain_or_all {
                ChainOrAll::NamedChain(chain) => {
                    cache.chains.push(Config::list_foundry_chain_cache(chain.into())?)
                }
                ChainOrAll::All => cache = Config::list_foundry_cache()?,
            }
        }
        print!("{cache}");
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum ChainOrAll {
    NamedChain(NamedChain),
    All,
}

impl FromStr for ChainOrAll {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(chain) = NamedChain::from_str(s) {
            Ok(Self::NamedChain(chain))
        } else if s == "all" {
            Ok(Self::All)
        } else {
            Err(format!("Expected known chain or all, found: {s}"))
        }
    }
}

fn clean_chain_cache(chain: impl Into<Chain>, blocks: Vec<u64>, etherscan: bool) -> Result<()> {
    let chain = chain.into();
    if blocks.is_empty() {
        Config::clean_foundry_etherscan_chain_cache(chain)?;
        if etherscan {
            return Ok(())
        }
        Config::clean_foundry_chain_cache(chain)?;
    } else {
        for block in blocks {
            Config::clean_foundry_block_cache(chain, block)?;
        }
    }
    Ok(())
}

/// The value parser for `ChainOrAll`
#[derive(Clone, Debug)]
pub struct ChainOrAllValueParser {
    inner: PossibleValuesParser,
}

impl Default for ChainOrAllValueParser {
    fn default() -> Self {
        Self { inner: possible_chains() }
    }
}

impl TypedValueParser for ChainOrAllValueParser {
    type Value = ChainOrAll;

    fn parse_ref(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.inner.parse_ref(cmd, arg, value)?.parse::<ChainOrAll>().map_err(|_| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                "chain argument did not match any possible chain variant",
            )
        })
    }
}

fn possible_chains() -> PossibleValuesParser {
    Some(&"all").into_iter().chain(NamedChain::VARIANTS).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_cache_ls() {
        let args: CacheArgs = CacheArgs::parse_from(["cache", "ls"]);
        assert!(matches!(args.sub, CacheSubcommands::Ls(_)));
    }
}
