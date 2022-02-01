use clap::{Parser, Subcommand, ValueHint};

use ethers::{solc::EvmVersion, types::Address};
use std::{path::PathBuf, str::FromStr};

use crate::cmd::{
    build::BuildArgs, config, create::CreateArgs, flatten, init::InitArgs, install::InstallArgs,
    remappings::RemappingArgs, run::RunArgs, snapshot, test,
};
use serde::Serialize;

#[derive(Debug, Parser)]
#[clap(name = "forge", version = crate::utils::VERSION_MESSAGE)]
pub struct Opts {
    #[clap(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, Subcommand)]
#[clap(about = "Build, test, fuzz, formally verify, debug & deploy solidity contracts.")]
#[allow(clippy::large_enum_variant)]
pub enum Subcommands {
    #[clap(about = "test your smart contracts")]
    #[clap(alias = "t")]
    Test(test::TestArgs),

    #[clap(about = "build your smart contracts")]
    #[clap(alias = "b")]
    Build(BuildArgs),

    #[clap(about = "run a single smart contract as a script")]
    #[clap(alias = "r")]
    Run(RunArgs),

    #[clap(alias = "u", about = "fetches all upstream lib changes")]
    Update {
        #[clap(
            help = "the submodule name of the library you want to update (will update all if none is provided)",
            value_hint = ValueHint::DirPath
        )]
        lib: Option<PathBuf>,
    },

    #[clap(
        alias = "i",
        about = "installs one or more dependencies as git submodules (will install existing dependencies if no arguments are provided"
    )]
    Install(InstallArgs),

    #[clap(alias = "rm", about = "removes one or more dependencies from git submodules")]
    Remove {
        #[clap(help = "the submodule name of the library you want to remove")]
        dependencies: Vec<Dependency>,
    },

    #[clap(about = "prints the automatically inferred remappings for this repository")]
    Remappings(RemappingArgs),

    #[clap(
        about = "verify your smart contracts source code on Etherscan. Requires `ETHERSCAN_API_KEY` to be set."
    )]
    VerifyContract {
        #[clap(help = "contract source info `<path>:<contractname>`")]
        contract: FullContractInfo,
        #[clap(help = "the address of the contract to verify.")]
        address: Address,
        #[clap(help = "constructor args calldata arguments.")]
        constructor_args: Vec<String>,
    },

    #[clap(alias = "c", about = "deploy a compiled contract")]
    Create(CreateArgs),

    #[clap(alias = "i", about = "initializes a new forge sample project")]
    Init(InitArgs),

    #[clap(about = "generate shell completions script")]
    Completions {
        #[clap(arg_enum)]
        shell: clap_complete::Shell,
    },

    #[clap(about = "removes the build artifacts and cache directories")]
    Clean {
        #[clap(
            help = "the project's root path, default being the current working directory",
            long,
            value_hint = ValueHint::DirPath
        )]
        root: Option<PathBuf>,
    },

    #[clap(about = "creates a snapshot of each test's gas usage")]
    Snapshot(snapshot::SnapshotArgs),

    #[clap(about = "shows the currently set config values")]
    Config(config::ConfigArgs),

    #[clap(about = "concats a file with all of its imports")]
    Flatten(flatten::FlattenArgs),
}

/// A set of solc compiler settings that can be set via command line arguments, which are intended
/// to be merged into an existing `foundry_config::Config`.
///
/// See also [`BuildArgs`]
#[derive(Default, Debug, Clone, Parser, Serialize)]
pub struct CompilerArgs {
    #[clap(help = "choose the evm version", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<EvmVersion>,

    #[clap(help = "activate the solidity optimizer", long)]
    // skipped because, optimize is opt-in
    #[serde(skip)]
    pub optimize: bool,

    #[clap(help = "optimizer parameter runs", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_runs: Option<usize>,

    #[clap(
        help = "extra output types [evm.assembly, ewasm, ir, irOptimized] eg: `--extra-output evm.assembly`",
        long
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_output: Option<Vec<String>>,
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
