use structopt::StructOpt;

use ethers::types::{Address, U256};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, StructOpt)]
#[structopt(about = "Build, test, fuzz, formally verify, debug & deploy solidity contracts.")]
#[allow(clippy::large_enum_variant)]
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

        #[structopt(
            help = "the initial balance of each deployed test contract",
            long,
            default_value = "0xffffffffffffffffffffffff"
        )]
        initial_balance: U256,

        #[structopt(
            help = "the address which will be executing all tests",
            long,
            default_value = "0x0000000000000000000000000000000000000000"
        )]
        deployer: Address,

        #[structopt(help = "enables the FFI cheatcode", long)]
        ffi: bool,
    },
    #[structopt(about = "build your smart contracts")]
    Build {
        #[structopt(flatten)]
        opts: BuildOpts,
    },
    #[structopt(about = "fetches all upstream lib changes")]
    Update {
        #[structopt(
            help = "the submodule name of the library you want to update (will update all if none is provided)"
        )]
        lib: Option<PathBuf>,
    },
    #[structopt(about = "installs 1 or more dependencies as git submodules")]
    Install {
        #[structopt(
            help = "the submodule name of the library you want to update (will update all if none is provided)"
        )]
        dependencies: Vec<Dependency>,
    },
    #[structopt(about = "prints the automatically inferred remappings for this repository")]
    Remappings,
    #[structopt(about = "build your smart contracts. Requires `ETHERSCAN_API_KEY` to be set.")]
    VerifyContract {
        #[structopt(help = "contract source info `<path>:<contractname>`")]
        contract: FullContractInfo,
        #[structopt(help = "the address of the contract to verify.")]
        address: Address,
        #[structopt(help = "constructor args calldata arguments.")]
        constructor_args: Vec<String>,
    },
    #[structopt(about = "deploy a compiled contract")]
    Create {
        #[structopt(help = "contract source info `<path>:<contractname>` or `<contractname>`")]
        contract: ContractInfo,
        #[structopt(long, help = "verify on Etherscan")]
        verify: bool,
    },
}

/// Represents the common dapp argument pattern for `<path>:<contractname>` where `<path>:` is
/// optional.
#[derive(Clone, Debug)]
pub struct ContractInfo {
    /// Location of the contract
    pub path: Option<String>,
    /// Name of the contract
    pub name: String,
}

impl FromStr for ContractInfo {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut iter = s.rsplit(':');
        let name = iter.next().unwrap().to_string();
        let path = iter.next().map(str::to_string);
        Ok(Self { path, name })
    }
}

/// Represents the common dapp argument pattern `<path>:<contractname>`
#[derive(Clone, Debug)]
pub struct FullContractInfo {
    /// Location of the contract
    pub path: String,
    /// Name of the contract
    pub name: String,
}

impl FromStr for FullContractInfo {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (path, name) = s
            .split_once(':')
            .ok_or_else(|| eyre::eyre!("Expected `<path>:<contractname>`, got `{}`", s))?;
        Ok(Self { path: path.to_string(), name: name.to_string() })
    }
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
        default_value = crate::utils::DAPP_JSON
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
    // structopt does not let use `u64::MAX`:
    // https://doc.rust-lang.org/std/primitive.u64.html#associatedconstant.MAX
    #[structopt(help = "the block gas limit", long, default_value = "18446744073709551615")]
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

/// A git dependency which will be installed as a submodule
///
/// A dependency can be provided as a raw URL, or as a path to a Github repository
/// e.g. `org-name/repo-name`
///
/// Providing a ref can be done in the following 3 ways:
/// * branch: master
/// * tag: v0.1.1
/// * commit: 8e8128
///
/// Non Github URLs must be provided with an https:// prefix.
/// Adding dependencies as local paths is not supported yet.
#[derive(Clone, Debug)]
pub struct Dependency {
    /// The name of the dependency
    pub name: String,
    /// The url to the git repository corresponding to the dependency
    pub url: String,
    /// Optional tag corresponding to a Git SHA, tag, or branch.
    pub tag: Option<String>,
}

const GITHUB: &str = "github.com";
const VERSION_SEPARATOR: char = '@';

impl FromStr for Dependency {
    type Err = eyre::Error;
    fn from_str(dependency: &str) -> Result<Self, Self::Err> {
        // TODO: Is there a better way to normalize these paths to having a
        // `https://github.com/` prefix?
        let path = if dependency.starts_with("https://") {
            dependency.to_string()
        } else if dependency.starts_with(GITHUB) {
            format!("https://{}", dependency)
        } else {
            format!("https://{}/{}", GITHUB, dependency)
        };

        // everything after the "@" should be considered the version
        let mut split = path.split(VERSION_SEPARATOR);
        let url =
            split.next().ok_or_else(|| eyre::eyre!("no dependency path was provided"))?.to_string();
        let name = url
            .split('/')
            .last()
            .ok_or_else(|| eyre::eyre!("no dependency name found"))?
            .to_string();
        let tag = split.next().map(ToString::to_string);

        Ok(Dependency { name, url, tag })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dependencies() {
        [
            ("gakonst/lootloose", "https://github.com/gakonst/lootloose", None),
            ("github.com/gakonst/lootloose", "https://github.com/gakonst/lootloose", None),
            ("https://github.com/gakonst/lootloose", "https://github.com/gakonst/lootloose", None),
            ("https://gitlab.com/gakonst/lootloose", "https://gitlab.com/gakonst/lootloose", None),
            ("gakonst/lootloose@0.1.0", "https://github.com/gakonst/lootloose", Some("0.1.0")),
            ("gakonst/lootloose@develop", "https://github.com/gakonst/lootloose", Some("develop")),
            (
                "gakonst/lootloose@98369d0edc900c71d0ec33a01dfba1d92111deed",
                "https://github.com/gakonst/lootloose",
                Some("98369d0edc900c71d0ec33a01dfba1d92111deed"),
            ),
        ]
        .iter()
        .for_each(|(input, expected_path, expected_tag)| {
            let dep = Dependency::from_str(input).unwrap();
            assert_eq!(dep.url, expected_path.to_string());
            assert_eq!(dep.tag, expected_tag.map(ToString::to_string));
            assert_eq!(dep.name, "lootloose");
        });
    }
}
