use structopt::StructOpt;

use dapp::evm::{backend::MemoryVicinity, Config};

use ethers::types::Address;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, StructOpt)]
#[structopt(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
pub enum Subcommands {
    Test {
        #[structopt(help = "print the test results in json format", long, short)]
        json: bool,

        #[structopt(flatten)]
        env: Env,

        #[structopt(
            long = "--match",
            short = "-m",
            help = "only run test methods matching regex",
            default_value = ".*"
        )]
        pattern: regex::Regex,

        #[structopt(flatten)]
        opts: BuildOpts,
    },
    Build {
        #[structopt(flatten)]
        opts: BuildOpts,
    },
}

#[derive(Debug, StructOpt)]
#[structopt(about = "build your smart contracts")]
pub struct BuildOpts {
    #[structopt(
        help = "glob path to your smart contracts",
        long,
        short,
        default_value = "./src/**/*.sol"
    )]
    pub contracts: String,

    #[structopt(help = "the remappings", long, short)]
    pub remappings: Vec<String>,
    #[structopt(env = "DAPP_REMAPPINGS")]
    pub remappings_env: Option<String>,

    #[structopt(help = "the paths where your libraries are installed", long)]
    pub lib_paths: Vec<String>,

    #[structopt(
        help = "path to where the contract artifacts are stored",
        long = "out",
        short,
        default_value = "./out/dapp.sol.json"
    )]
    pub out_path: PathBuf,

    #[structopt(help = "skip re-compilation", long, short)]
    pub no_compile: bool,

    #[structopt(help = "choose the evm version", long, default_value = "berlin")]
    pub evm_version: EvmVersion,
}

#[derive(Clone, Debug)]
pub enum EvmVersion {
    Frontier,
    Istanbul,
    Berlin,
}

impl EvmVersion {
    pub fn cfg(self) -> Config {
        use EvmVersion::*;
        match self {
            Frontier => Config::frontier(),
            Istanbul => Config::istanbul(),
            Berlin => Config::berlin(),
        }
    }
}

impl FromStr for EvmVersion {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use EvmVersion::*;
        Ok(match s.to_lowercase().as_str() {
            "frontier" => Frontier,
            "istanbul" => Istanbul,
            "berlin" => Berlin,
            // TODO: Add London.
            _ => eyre::bail!("unsupported evm version: {}", s),
        })
    }
}

#[derive(Debug, StructOpt)]
pub struct Env {
    #[structopt(help = "the block gas limit", long, default_value = "25000000")]
    pub gas_limit: u64,

    #[structopt(help = "the chainid opcode value", long, default_value = "1")]
    pub chain_id: u64,

    #[structopt(
        help = "the tx.gasprice value during EVM execution",
        long,
        default_value = "0"
    )]
    pub gas_price: u64,

    #[structopt(
        help = "the tx.origin value during EVM execution",
        long,
        default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub tx_origin: Address,

    #[structopt(
        help = "the block.coinbase value during EVM execution",
        long,
        // TODO: It'd be nice if we could use Address::zero() here.
        default_value = "0x0000000000000000000000000000000000000000"
    )]
    pub block_coinbase: Address,
    #[structopt(
        help = "the block.timestamp value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_timestamp: u64,

    #[structopt(
        help = "the block.number value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_number: u64,

    #[structopt(
        help = "the block.difficulty value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_difficulty: u64,

    #[structopt(help = "the block.gaslimit value during EVM execution", long)]
    pub block_gas_limit: Option<u64>,
}

impl Env {
    // TODO: Maybe we should allow a way to specify multiple vicinities for use
    // across tests? Probably not, better to do with HEVM cheat codes.
    pub fn vicinity(&self) -> MemoryVicinity {
        MemoryVicinity {
            chain_id: self.chain_id.into(),

            gas_price: self.gas_price.into(),
            origin: self.tx_origin,

            block_coinbase: self.block_coinbase,
            block_number: self.block_number.into(),
            block_timestamp: self.block_timestamp.into(),
            block_difficulty: self.block_difficulty.into(),
            block_gas_limit: self.block_gas_limit.unwrap_or(self.gas_limit).into(),
            block_hashes: Vec::new(),
        }
    }
}
