use structopt::StructOpt;

use ethers::{solc::EvmVersion, types::Address};
use std::{path::PathBuf, str::FromStr};

use crate::cmd::{build::BuildArgs, create::CreateArgs, run::RunArgs, snapshot, test};

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
    Test(test::TestArgs),

    #[structopt(about = "build your smart contracts")]
    #[structopt(alias = "b")]
    Build(BuildArgs),

    #[structopt(about = "run a single smart contract as a script")]
    #[structopt(alias = "r")]
    Run(RunArgs),

    #[structopt(alias = "u", about = "fetches all upstream lib changes")]
    Update {
        #[structopt(
            help = "the submodule name of the library you want to update (will update all if none is provided)"
        )]
        lib: Option<PathBuf>,
    },

    #[structopt(alias = "i", about = "installs one or more dependencies as git submodules")]
    Install {
        #[structopt(help = "the submodule name of the library you want to install")]
        dependencies: Vec<Dependency>,
    },

    #[structopt(alias = "r", about = "removes one or more dependencies from git submodules")]
    Remove {
        #[structopt(help = "the submodule name of the library you want to remove")]
        dependencies: Vec<Dependency>,
    },

    #[structopt(about = "prints the automatically inferred remappings for this repository")]
    Remappings {
        #[structopt(
            help = "the project's root path, default being the current working directory",
            long
        )]
        root: Option<PathBuf>,
        #[structopt(help = "the paths where your libraries are installed", long)]
        lib_paths: Vec<PathBuf>,
    },

    #[structopt(
        about = "verify your smart contracts source code on Etherscan. Requires `ETHERSCAN_API_KEY` to be set."
    )]
    VerifyContract {
        #[structopt(help = "contract source info `<path>:<contractname>`")]
        contract: FullContractInfo,
        #[structopt(help = "the address of the contract to verify.")]
        address: Address,
        #[structopt(help = "constructor args calldata arguments.")]
        constructor_args: Vec<String>,
    },

    #[structopt(alias = "c", about = "deploy a compiled contract")]
    Create(CreateArgs),

    #[structopt(alias = "i", about = "initializes a new forge sample repository")]
    Init {
        #[structopt(help = "the project's root path, default being the current working directory")]
        root: Option<PathBuf>,
        #[structopt(help = "optional solidity template to start from", long, short)]
        template: Option<String>,
    },

    #[structopt(about = "generate shell completions script")]
    Completions {
        #[structopt(help = "the shell you are using")]
        shell: structopt::clap::Shell,
    },

    #[structopt(about = "removes the build artifacts and cache directories")]
    Clean {
        #[structopt(
            help = "the project's root path, default being the current working directory",
            long
        )]
        root: Option<PathBuf>,
    },

    #[structopt(about = "creates a snapshot of each test's gas usage")]
    Snapshot(snapshot::SnapshotArgs),
}

#[derive(Debug, Clone, StructOpt)]
pub struct CompilerArgs {
    #[structopt(help = "choose the evm version", long, default_value = "london")]
    pub evm_version: EvmVersion,

    #[structopt(help = "activate the solidity optimizer", long)]
    pub optimize: bool,

    #[structopt(help = "optimizer parameter runs", long, default_value = "200")]
    pub optimize_runs: u32,
}

use crate::cmd::build::{Env, EvmType};
use ethers::types::U256;

#[derive(Debug, Clone, StructOpt)]
pub struct EvmOpts {
    #[structopt(flatten)]
    pub env: Env,

    #[structopt(
        long,
        short,
        help = "the EVM type you want to use (e.g. sputnik, evmodin)",
        default_value = "sputnik"
    )]
    pub evm_type: EvmType,

    #[structopt(
        help = "fetch state over a remote instead of starting from empty state",
        long,
        short
    )]
    #[structopt(alias = "rpc-url")]
    pub fork_url: Option<String>,

    #[structopt(help = "pins the block number for the state fork", long)]
    #[structopt(env = "DAPP_FORK_BLOCK")]
    pub fork_block_number: Option<u64>,

    #[structopt(
        help = "the initial balance of each deployed test contract",
        long,
        default_value = "0xffffffffffffffffffffffff"
    )]
    pub initial_balance: U256,

    #[structopt(
        help = "the address which will be executing all tests",
        long,
        default_value = "0x0000000000000000000000000000000000000000",
        env = "DAPP_TEST_ADDRESS"
    )]
    pub sender: Address,

    #[structopt(help = "enables the FFI cheatcode", long)]
    pub ffi: bool,

    #[structopt(
        help = r#"Verbosity mode of EVM output as number of occurences of the `v` flag (-v, -vv, -vvv, etc.)
    3: print test trace for failing tests
    4: always print test trace, print setup for failing tests
    5: always print test trace and setup
"#,
        long,
        short,
        parse(from_occurrences)
    )]
    pub verbosity: u8,

    #[structopt(help = "enable debugger", long)]
    pub debug: bool,
}

impl EvmOpts {
    #[cfg(feature = "sputnik-evm")]
    pub fn vicinity(&self) -> eyre::Result<sputnik::backend::MemoryVicinity> {
        Ok(if let Some(ref url) = self.fork_url {
            let provider = ethers::providers::Provider::try_from(url.as_str())?;
            let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
            rt.block_on(evm_adapters::sputnik::vicinity(
                &provider,
                self.fork_block_number,
                Some(self.env.tx_origin),
            ))?
        } else {
            self.env.sputnik_state()
        })
    }
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
