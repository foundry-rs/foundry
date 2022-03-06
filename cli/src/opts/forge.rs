use clap::{Parser, Subcommand, ValueHint};

use ethers::solc::{artifacts::output_selection::ContractOutputSelection, EvmVersion};
use std::{path::PathBuf, str::FromStr};

use crate::cmd::{
    bind::BindArgs,
    build::BuildArgs,
    config,
    create::CreateArgs,
    flatten,
    init::InitArgs,
    install::InstallArgs,
    remappings::RemappingArgs,
    run::RunArgs,
    snapshot, test,
    verify::{VerifyArgs, VerifyCheckArgs},
};
use serde::Serialize;

use once_cell::sync::Lazy;
use regex::Regex;

static GH_REPO_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new("[A-Za-z\\d-]+/[A-Za-z\\d_.-]+").unwrap());

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
    #[clap(about = "Test your smart contracts")]
    #[clap(alias = "t")]
    Test(test::TestArgs),

    #[clap(about = "Generate rust bindings for your smart contracts")]
    Bind(BindArgs),

    #[clap(about = "Build your smart contracts")]
    #[clap(alias = "b")]
    Build(BuildArgs),

    #[clap(about = "Run a single smart contract as a script")]
    #[clap(alias = "r")]
    Run(RunArgs),

    #[clap(alias = "u", about = "Fetches all upstream lib changes")]
    Update {
        #[clap(
            help = "The submodule name of the library you want to update (will update all if none is provided)",
            value_hint = ValueHint::DirPath
        )]
        lib: Option<PathBuf>,
    },

    #[clap(
        alias = "i",
        about = "installs one or more dependencies as git submodules (will install existing dependencies if no arguments are provided)"
    )]
    Install(InstallArgs),

    #[clap(alias = "rm", about = "Removes one or more dependencies from git submodules")]
    Remove {
        #[clap(help = "The submodule name of the library you want to remove")]
        dependencies: Vec<Dependency>,
    },

    #[clap(about = "Prints the automatically inferred remappings for this repository")]
    Remappings(RemappingArgs),

    #[clap(
        about = "Verify your smart contracts source code on Etherscan. Requires `ETHERSCAN_API_KEY` to be set."
    )]
    VerifyContract(VerifyArgs),

    #[clap(
        about = "Check verification status on Etherscan. Requires `ETHERSCAN_API_KEY` to be set."
    )]
    VerifyCheck(VerifyCheckArgs),

    #[clap(alias = "c", about = "Deploy a compiled contract")]
    Create(CreateArgs),

    #[clap(alias = "i", about = "Initializes a new forge sample project")]
    Init(InitArgs),

    #[clap(about = "Generate shell completions script")]
    Completions {
        #[clap(arg_enum)]
        shell: clap_complete::Shell,
    },

    #[clap(about = "Removes the build artifacts and cache directories")]
    Clean {
        #[clap(
            help = "The project's root path, default being the current working directory",
            long,
            value_hint = ValueHint::DirPath
        )]
        root: Option<PathBuf>,
    },

    #[clap(about = "Creates a snapshot of each test's gas usage")]
    Snapshot(snapshot::SnapshotArgs),

    #[clap(about = "Shows the currently set config values")]
    Config(config::ConfigArgs),

    #[clap(about = "Concats a file with all of its imports")]
    Flatten(flatten::FlattenArgs),
    // #[clap(about = "formats Solidity source files")]
    // Fmt(FmtArgs),
}

// A set of solc compiler settings that can be set via command line arguments, which are intended
// to be merged into an existing `foundry_config::Config`.
//
// See also [`BuildArgs`]
#[derive(Default, Debug, Clone, Parser, Serialize)]
pub struct CompilerArgs {
    #[clap(help = "Choose the evm version", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<EvmVersion>,

    #[clap(help = "Activate the solidity optimizer", long)]
    // skipped because, optimize is opt-in
    #[serde(skip)]
    pub optimize: bool,

    #[clap(help = "Optimizer parameter runs", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_runs: Option<usize>,

    #[clap(
        help = "Extra output types to include in the contract's json artifact [evm.assembly, ewasm, ir, irOptimized, metadata] eg: `--extra-output evm.assembly`",
        long
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_output: Option<Vec<ContractOutputSelection>>,

    #[clap(
        help = "Extra output types to write to a separate file [metadata, ir, irOptimized, ewasm] eg: `--extra-output-files metadata`",
        long
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_output_files: Option<Vec<ContractOutputSelection>>,
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

        if name.ends_with(".sol") || name.contains('/') {
            return Err(eyre::eyre!(
                "contract source info format must be `<path>:<contractname>` or `<contractname>`"
            ))
        }

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
            if !GH_REPO_REGEX.is_match(dependency) {
                eyre::bail!("invalid github repository name `{}`", dependency);
            }
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

    #[test]
    #[should_panic]
    fn test_invalid_github_repo_dependency() {
        Dependency::from_str("solmate").unwrap();
    }

    #[test]
    fn parses_contract_info() {
        [
            (
                "src/contracts/Contracts.sol:Contract",
                Some("src/contracts/Contracts.sol"),
                "Contract",
            ),
            ("Contract", None, "Contract"),
        ]
        .iter()
        .for_each(|(input, expected_path, expected_name)| {
            let contract = ContractInfo::from_str(input).unwrap();
            assert_eq!(contract.path, expected_path.map(ToString::to_string));
            assert_eq!(contract.name, expected_name.to_string());
        });
    }

    #[test]
    fn contract_info_should_reject_without_name() {
        ["src/contracts/", "src/contracts/Contracts.sol"].iter().for_each(|input| {
            let contract = ContractInfo::from_str(input);
            assert!(contract.is_err())
        });
    }
}
