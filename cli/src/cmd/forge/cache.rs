//! cache command

use crate::cmd::Cmd;
use cache::Cache;
use clap::{
    builder::{PossibleValuesParser, TypedValueParser},
    Arg, Command, Parser, Subcommand,
};
use ethers::prelude::Chain;
use eyre::Result;
use foundry_config::{cache, Chain as FoundryConfigChain, Config};
use std::{ffi::OsStr, str::FromStr};
use strum::VariantNames;

#[derive(Debug, Parser)]
pub struct CacheArgs {
    #[clap(subcommand)]
    pub sub: CacheSubcommands,
}

#[derive(Debug, Subcommand)]
pub enum CacheSubcommands {
    #[clap(about = "Cleans cached data from ~/.foundry.")]
    Clean(CleanArgs),
    #[clap(about = "Shows cached data from ~/.foundry.")]
    Ls(LsArgs),
}

#[derive(Debug, Parser)]
#[clap(group = clap::ArgGroup::new("etherscan-blocks").multiple(false))]
pub struct CleanArgs {
    #[clap(
        env = "CHAIN",
        default_value = "all",
        value_parser = ChainOrAllValueParser::default(),
        value_name = "CHAINS"
    )]
    chains: Vec<ChainOrAll>,

    #[clap(
        short,
        long,
        num_args(1..),
        use_value_delimiter(true),
        value_delimiter(','),
        value_name = "BLOCKS",
        group = "etherscan-blocks"
    )]
    blocks: Vec<u64>,

    #[clap(long, group = "etherscan-blocks")]
    etherscan: bool,
}

impl Cmd for CleanArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        let CleanArgs { chains, blocks, etherscan } = self;

        for chain_or_all in chains {
            match chain_or_all {
                ChainOrAll::Chain(chain) => clean_chain_cache(chain, blocks.to_vec(), etherscan)?,
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
    #[clap(
        env = "CHAIN",
        default_value = "all",
        value_parser = ChainOrAllValueParser::default(),
        value_name = "CHAINS"
    )]
    chains: Vec<ChainOrAll>,
}

impl Cmd for LsArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        let LsArgs { chains } = self;
        let mut cache = Cache::default();
        for chain_or_all in chains {
            match chain_or_all {
                ChainOrAll::Chain(chain) => {
                    cache.chains.push(Config::list_foundry_chain_cache(chain.into())?)
                }
                ChainOrAll::All => cache = Config::list_foundry_cache()?,
            }
        }
        print!("{}", cache);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum ChainOrAll {
    Chain(Chain),
    All,
}

impl FromStr for ChainOrAll {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(chain) = ethers::prelude::Chain::from_str(s) {
            Ok(ChainOrAll::Chain(chain))
        } else if s == "all" {
            Ok(ChainOrAll::All)
        } else {
            Err(format!("Expected known chain or all, found: {s}"))
        }
    }
}

fn clean_chain_cache(
    chain: impl Into<FoundryConfigChain>,
    blocks: Vec<u64>,
    etherscan: bool,
) -> Result<()> {
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
        ChainOrAllValueParser { inner: possible_chains() }
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
    Some(&"all").into_iter().chain(Chain::VARIANTS).into()
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
