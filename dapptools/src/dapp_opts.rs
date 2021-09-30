use structopt::StructOpt;

use ethers::types::Address;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, StructOpt)]
#[structopt(about = "Build, test, fuzz, formally verify, debug & deploy solidity contracts.")]
pub enum Subcommands {
    #[structopt(about = "test your smart contracts")]
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

        #[structopt(
            long,
            short,
            help = "the EVM type you want to use (e.g. sputnik, evmodin)",
            default_value = "sputnik"
        )]
        evm_type: EvmType,

        #[structopt(help = "skip re-compilation", long, short)]
        no_compile: bool,

        #[structopt(
            help = "fetch state over a remote instead of starting from empty state",
            long,
            short
        )]
        #[structopt(alias = "rpc-url")]
        fork_url: Option<String>,

        #[structopt(help = "pins the block number for the state fork", long)]
        fork_block_number: Option<u64>,
    },
    #[structopt(about = "build your smart contracts")]
    Build {
        #[structopt(flatten)]
        opts: BuildOpts,
    },
}

#[derive(Debug, StructOpt)]
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

    #[structopt(help = "choose the evm version", long, default_value = "berlin")]
    pub evm_version: EvmVersion,
}

#[derive(Clone, Debug)]
pub enum EvmType {
    #[cfg(feature = "sputnik-evm")]
    Sputnik,
    #[cfg(feature = "evmodin-evm")]
    EvmOdin,
}

impl FromStr for EvmType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            #[cfg(feature = "sputnik-evm")]
            "sputnik" => EvmType::Sputnik,
            #[cfg(feature = "evmodin-evm")]
            "evmodin" => EvmType::EvmOdin,
            other => eyre::bail!("unknown EVM type {}", other),
        })
    }
}

#[derive(Clone, Debug)]
pub enum EvmVersion {
    Frontier,
    Istanbul,
    Berlin,
    London,
}

#[cfg(feature = "sputnik-evm")]
use sputnik::Config;

#[cfg(feature = "evmodin-evm")]
use evmodin::Revision;

impl EvmVersion {
    #[cfg(feature = "sputnik-evm")]
    pub fn sputnik_cfg(self) -> Config {
        use EvmVersion::*;
        match self {
            Frontier => Config::frontier(),
            Istanbul => Config::istanbul(),
            Berlin => Config::berlin(),
            other => panic!("The {:?} hard fork is unsupported on Sputnik", other),
        }
    }

    #[cfg(feature = "evmodin-evm")]
    pub fn evmodin_cfg(self) -> Revision {
        use EvmVersion::*;
        match self {
            Frontier => Revision::Frontier,
            Istanbul => Revision::Istanbul,
            Berlin => Revision::Berlin,
            London => Revision::London,
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
            "london" => London,
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

    #[structopt(help = "the tx.gasprice value during EVM execution", long, default_value = "0")]
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

    #[structopt(help = "the block.number value during EVM execution", long, default_value = "0")]
    pub block_number: u64,

    #[structopt(
        help = "the block.difficulty value during EVM execution",
        long,
        default_value = "0"
    )]
    pub block_difficulty: u64,

    #[structopt(help = "the block.gaslimit value during EVM execution", long)]
    pub block_gas_limit: Option<u64>,
    // TODO: Add configuration option for base fee.
}

#[cfg(feature = "sputnik-evm")]
use sputnik::backend::MemoryVicinity;

#[cfg(feature = "evmodin-evm")]
use evmodin::util::mocked_host::MockedHost;

impl Env {
    #[cfg(feature = "sputnik-evm")]
    pub fn sputnik_state(&self) -> MemoryVicinity {
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

    #[cfg(feature = "evmodin-evm")]
    pub fn evmodin_state(&self) -> MockedHost {
        let mut host = MockedHost::default();

        host.tx_context.chain_id = self.chain_id.into();
        host.tx_context.tx_gas_price = self.gas_price.into();
        host.tx_context.tx_origin = self.tx_origin;
        host.tx_context.block_coinbase = self.block_coinbase;
        host.tx_context.block_number = self.block_number;
        host.tx_context.block_timestamp = self.block_timestamp;
        host.tx_context.block_difficulty = self.block_difficulty.into();
        host.tx_context.block_gas_limit = self.block_gas_limit.unwrap_or(self.gas_limit);

        host
    }
}
