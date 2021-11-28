use structopt::StructOpt;

use ethers::{
    solc::{remappings::Remapping, Project, ProjectPathsConfig},
    types::{Address, U256},
};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, StructOpt)]
#[structopt(name = "forge")]
#[structopt(about = "Build, test, fuzz, formally verify, debug & deploy solidity contracts.")]
#[allow(clippy::large_enum_variant)]
pub enum Subcommands {
    #[structopt(about = "test your smart contracts")]
    #[structopt(alias = "t")]
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

        #[structopt(
            help = "fetch state over a remote instead of starting from empty state",
            long,
            short
        )]
        #[structopt(alias = "rpc-url")]
        #[structopt(env = "ETH_RPC_URL")]
        fork_url: Option<String>,

        #[structopt(help = "pins the block number for the state fork", long)]
        #[structopt(env = "DAPP_FORK_BLOCK")]
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
            default_value = "0x0000000000000000000000000000000000000000",
            env = "DAPP_TEST_ADDRESS"
        )]
        sender: Address,

        #[structopt(help = "enables the FFI cheatcode", long)]
        ffi: bool,

        #[structopt(help = "verbosity of 'forge test' output (0-3)", long, default_value = "0")]
        verbosity: u8,

        #[structopt(
            help = "if set to true, the process will exit with an exit code = 0, even if the tests fail",
            long,
            env = "FORGE_ALLOW_FAILURE"
        )]
        allow_failure: bool,
    },
    #[structopt(about = "build your smart contracts")]
    #[structopt(alias = "b")]
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
    #[structopt(about = "installs one or more dependencies as git submodules")]
    Install {
        #[structopt(
            help = "the submodule name of the library you want to update (will update all if none is provided)"
        )]
        dependencies: Vec<Dependency>,
    },
    #[structopt(about = "prints the automatically inferred remappings for this repository")]
    Remappings {
        #[structopt(help = "the project's root path, default being the current directory", long)]
        root: Option<PathBuf>,
        #[structopt(help = "the paths where your libraries are installed", long)]
        lib_paths: Vec<PathBuf>,
    },
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
    #[structopt(alias = "i", about = "initializes a new forge repository")]
    Init {
        #[structopt(help = "the project's root path, default being the current directory")]
        root: Option<PathBuf>,
        #[structopt(help = "optional solidity template to start from", long, short)]
        template: Option<String>,
    },
    Completions {
        #[structopt(help = "the shell you are using")]
        shell: structopt::clap::Shell,
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

impl std::convert::TryFrom<&BuildOpts> for Project {
    type Error = eyre::Error;

    /// Defaults to converting to DAppTools-style repo layout, but can be customized.
    fn try_from(opts: &BuildOpts) -> eyre::Result<Project> {
        // 1. Set the root dir
        let root = opts.root.clone().unwrap_or_else(|| std::env::current_dir().unwrap());
        let root = std::fs::canonicalize(&root)?;

        // 2. Set the contracts dir
        let contracts = if let Some(ref contracts) = opts.contracts {
            root.join(contracts)
        } else {
            root.join("src")
        };

        // 3. Set the output dir
        let artifacts = if let Some(ref artifacts) = opts.out_path {
            root.join(artifacts)
        } else {
            root.join("out")
        };

        // 4. Set where the libraries are going to be read from
        // default to the lib path being the `lib/` dir
        let lib_paths =
            if opts.lib_paths.is_empty() { vec![root.join("lib")] } else { opts.lib_paths.clone() };

        // get all the remappings corresponding to the lib paths
        let mut remappings: Vec<_> =
            lib_paths.iter().map(|path| Remapping::find_many(&path).unwrap()).flatten().collect();

        // extend them with the once manually provided in the opts
        remappings.extend_from_slice(&opts.remappings);

        // extend them with the one via the env vars
        if let Some(ref env) = opts.remappings_env {
            remappings.extend(remappings_from_newline(env))
        }

        // extend them with the one via the requirements.txt
        if let Ok(ref remap) = std::fs::read_to_string(root.join("remappings.txt")) {
            remappings.extend(remappings_from_newline(remap))
        }

        // helper function for parsing newline-separated remappings
        fn remappings_from_newline(remappings: &str) -> impl Iterator<Item = Remapping> + '_ {
            remappings.split('\n').filter(|x| !x.is_empty()).map(|x| {
                Remapping::from_str(x)
                    .unwrap_or_else(|_| panic!("could not parse remapping: {}", x))
            })
        }

        // remove any potential duplicates
        remappings.sort_unstable();
        remappings.dedup();

        // build the path
        let mut paths_builder =
            ProjectPathsConfig::builder().root(&root).sources(contracts).artifacts(artifacts);

        if !remappings.is_empty() {
            paths_builder = paths_builder.remappings(remappings);
        }

        let paths = paths_builder.build()?;

        // build the project w/ allowed paths = root and all the libs
        let mut builder =
            Project::builder().paths(paths).allowed_path(root).allowed_paths(lib_paths);

        if opts.no_auto_detect {
            builder = builder.no_auto_detect();
        }

        let project = builder.build()?;

        Ok(project)
    }
}

#[derive(Debug, StructOpt)]
pub struct BuildOpts {
    #[structopt(help = "the project's root path, default being the current directory", long)]
    pub root: Option<PathBuf>,

    #[structopt(
        help = "the directory relative to the root under which the smart contrats are",
        long,
        short
    )]
    #[structopt(env = "DAPP_SRC")]
    pub contracts: Option<PathBuf>,

    #[structopt(help = "the remappings", long, short)]
    pub remappings: Vec<ethers::solc::remappings::Remapping>,
    #[structopt(long = "remappings-env", env = "DAPP_REMAPPINGS")]
    pub remappings_env: Option<String>,

    #[structopt(help = "the paths where your libraries are installed", long)]
    pub lib_paths: Vec<PathBuf>,

    #[structopt(help = "path to where the contract artifacts are stored", long = "out", short)]
    pub out_path: Option<PathBuf>,

    #[structopt(help = "choose the evm version", long, default_value = "london")]
    pub evm_version: EvmVersion,

    #[structopt(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    pub no_auto_detect: bool,
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
            London => Config::london(),
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

    #[structopt(help = "the base fee in a block", long, default_value = "0")]
    pub block_base_fee_per_gas: u64,

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
        default_value = "0",
        env = "DAPP_TEST_TIMESTAMP"
    )]
    pub block_timestamp: u64,

    #[structopt(help = "the block.number value during EVM execution", long, default_value = "0")]
    #[structopt(env = "DAPP_TEST_NUMBER")]
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
            block_base_fee_per_gas: self.block_base_fee_per_gas.into(),
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
