//! cache command

use clap::{Parser, Subcommand};
use std::str::FromStr;

use crate::cmd::Cmd;
use cache::{Cache, ChainCache};
use ethers::prelude::Chain;
use eyre::Result;
use foundry_config::{cache, Chain as FoundryConfigChain, Config};

#[derive(Debug, Parser)]
pub struct CacheArgs {
    #[clap(subcommand)]
    pub sub: CacheSubcommands,
}

#[derive(Debug)]
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

#[derive(Debug, Parser)]
pub struct CleanArgs {
    // TODO refactor to dedup shared logic with ClapChain in opts/mod
    #[clap(
        env = "CHAIN",
        default_value = "all",
        possible_values = [
            "all",
            "mainnet",
            "ropsten",
            "rinkeby",
            "goerli",
            "kovan",
            "xdai",
            "polygon",
            "polygon_mumbai",
            "avalanche",
            "avalanche_fuji",
            "sepolia",
            "moonbeam",
            "moonbeam_dev",
            "moonriver",
            "optimism",
            "optimism-kovan"
    ])]
    chains: Vec<ChainOrAll>,

    #[clap(
        short,
        long,
        multiple_values(true),
        use_value_delimiter(true),
        require_value_delimiter(true)
    )]
    blocks: Vec<u64>,
}

#[derive(Debug, Parser)]
pub struct LsArgs {
    // TODO refactor to dedup shared logic with ClapChain in opts/mod
    #[clap(
        env = "CHAIN",
        default_value = "all",
        possible_values = [
            "all",
            "mainnet",
            "ropsten",
            "rinkeby",
            "goerli",
            "kovan",
            "xdai",
            "polygon",
            "polygon_mumbai",
            "avalanche",
            "avalanche_fuji",
            "sepolia",
            "moonbeam",
            "moonbeam_dev",
            "moonriver",
            "optimism",
            "optimism-kovan"
    ])]
    chains: Vec<ChainOrAll>,
}

#[derive(Debug, Subcommand)]
pub enum CacheSubcommands {
    #[clap(about = "Cleans cached data from ~/.foundry.")]
    Clean(CleanArgs),
    #[clap(about = "Shows cached data from ~/.foundry.")]
    Ls(LsArgs),
}

impl Cmd for CleanArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        let CleanArgs { chains, blocks } = self;

        for chain_or_all in chains {
            match chain_or_all {
                ChainOrAll::Chain(chain) => clean_chain_cache(chain, blocks.to_vec())?,
                ChainOrAll::All => Config::clean_foundry_cache()?,
            }
        }

        Ok(())
    }
}

impl Cmd for LsArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        let LsArgs { chains } = self;
        let mut cache = Cache { chains: vec![] };
        for chain_or_all in chains {
            match chain_or_all {
                ChainOrAll::Chain(chain) => cache.chains.push(list_chain_cache(chain)?),
                ChainOrAll::All => cache = Config::list_foundry_cache()?,
            }
        }
        print!("{}", cache);
        Ok(())
    }
}

fn clean_chain_cache(chain: Chain, blocks: Vec<u64>) -> Result<()> {
    if let Ok(foundry_chain) = FoundryConfigChain::try_from(chain) {
        if blocks.is_empty() {
            Config::clean_foundry_chain_cache(foundry_chain)?;
        } else {
            for block in blocks {
                Config::clean_foundry_block_cache(foundry_chain, block)?;
            }
        }
    } else {
        eyre::bail!("failed to map chain");
    }

    Ok(())
}

fn list_chain_cache(chain: Chain) -> Result<ChainCache> {
    if let Ok(foundry_chain) = FoundryConfigChain::try_from(chain) {
        Config::list_foundry_chain_cache(foundry_chain)
    } else {
        eyre::bail!("failed to map chain");
    }
}
