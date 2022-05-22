use clap::{Parser, Subcommand, ValueHint};

use ethers::solc::{artifacts::output_selection::ContractOutputSelection, EvmVersion};
use std::{path::PathBuf, str::FromStr};

use crate::cmd::forge::{
    bind::BindArgs,
    build::BuildArgs,
    cache::CacheArgs,
    config,
    create::CreateArgs,
    flatten,
    fmt::FmtArgs,
    init::InitArgs,
    inspect,
    install::InstallArgs,
    remappings::RemappingArgs,
    run::RunArgs,
    snapshot, test, tree,
    verify::{VerifyArgs, VerifyCheckArgs},
};
use serde::Serialize;

use once_cell::sync::Lazy;
use regex::Regex;

static GH_REPO_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new("[A-Za-z\\d-]+/[A-Za-z\\d_.-]+").unwrap());

static GH_REPO_PREFIX_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"((git@)|(git\+https://)|(https://))?([A-Za-z0-9-]+)\.([A-Za-z0-9-]+)(/|:)")
        .unwrap()
});

#[derive(Debug, Parser)]
#[clap(name = "forge", version = crate::utils::VERSION_MESSAGE)]
pub struct Opts {
    #[clap(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, Subcommand)]
#[clap(
    about = "Build, test, fuzz, debug and deploy Solidity contracts.",
    after_help = "Find more information in the book: http://book.getfoundry.sh/reference/forge/forge.html"
)]
#[allow(clippy::large_enum_variant)]
pub enum Subcommands {
    #[clap(alias = "t")]
    #[clap(about = "Run the project's tests.")]
    Test(test::TestArgs),

    #[clap(alias = "bi")]
    #[clap(about = "Generate Rust bindings for smart contracts.")]
    Bind(BindArgs),

    #[clap(alias = "b")]
    #[clap(about = "Build the project's smart contracts.")]
    Build(BuildArgs),

    #[clap(alias = "r")]
    #[clap(about = "Run a single smart contract as a script.")]
    Run(RunArgs),

    #[clap(
        alias = "u",
        about = "Update one or multiple dependencies.",
        long_about = "Update one or multiple dependencies. If no arguments are provided, then all dependencies are updated."
    )]
    Update {
        #[clap(
            help = "The path to the dependency you want to update.",
            value_hint = ValueHint::DirPath
        )]
        lib: Option<PathBuf>,
    },

    #[clap(
        alias = "i",
        about = "Install one or multiple dependencies.",
        long_about = "Install one or multiple dependencies. If no arguments are provided, then existing dependencies will be installed."
    )]
    Install(InstallArgs),

    #[clap(alias = "rm", about = "Remove one or multiple dependencies.")]
    Remove {
        #[clap(help = "The path to the dependency you want to remove.")]
        dependencies: Vec<Dependency>,
    },

    #[clap(alias = "re", about = "Get the automatically inferred remappings for the project.")]
    Remappings(RemappingArgs),

    #[clap(
        alias = "v",
        about = "Verify smart contracts on Etherscan.",
        long_about = "Verify smart contracts on Etherscan."
    )]
    VerifyContract(VerifyArgs),

    #[clap(
        alias = "vc",
        about = "Check verification status on Etherscan.",
        long_about = "Check verification status on Etherscan."
    )]
    VerifyCheck(VerifyCheckArgs),

    #[clap(alias = "c", about = "Deploy a smart contract.")]
    Create(CreateArgs),

    #[clap(about = "Create a new Forge project.")]
    Init(InitArgs),

    #[clap(alias = "com", about = "Generate shell completions script")]
    Completions {
        #[clap(arg_enum)]
        shell: clap_complete::Shell,
    },

    #[clap(alias = "cl", about = "Remove the build artifacts and cache directories.")]
    Clean {
        #[clap(
            help = "The project's root path. Defaults to the current working directory.",
            long,
            value_hint = ValueHint::DirPath
        )]
        root: Option<PathBuf>,
    },

    #[clap(about = "Manage the Foundry cache.")]
    Cache(CacheArgs),

    #[clap(alias = "s", about = "Create a snapshot of each test's gas usage.")]
    Snapshot(snapshot::SnapshotArgs),

    #[clap(alias = "co", about = "Display the current config.")]
    Config(config::ConfigArgs),

    #[clap(alias = "f", about = "Flatten a source file and all of its imports into one file.")]
    Flatten(flatten::FlattenArgs),

    #[clap(about = "formats Solidity source files")]
    Fmt(FmtArgs),

    #[clap(alias = "in", about = "Get specialized information about a smart contract")]
    Inspect(inspect::InspectArgs),

    #[clap(
        alias = "tr",
        about = "Display a tree visualization of the project's dependency graph."
    )]
    Tree(tree::TreeArgs),
}

// A set of solc compiler settings that can be set via command line arguments, which are intended
// to be merged into an existing `foundry_config::Config`.
//
// See also [`BuildArgs`]
#[derive(Default, Debug, Clone, Parser, Serialize)]
pub struct CompilerArgs {
    #[clap(help = "The target EVM version.", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<EvmVersion>,

    #[clap(help = "Activate the Solidity optimizer.", long)]
    #[serde(skip)]
    pub optimize: bool,

    #[clap(help = "The number of optimizer runs.", long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_runs: Option<usize>,

    /// Extra output to include in the contract's artifact.
    ///
    /// Example keys: evm.assembly, ewasm, ir, irOptimized, metadata
    ///
    /// For a full description, see https://docs.soliditylang.org/en/v0.8.13/using-the-compiler.html#input-description
    #[clap(long, min_values = 1)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_output: Vec<ContractOutputSelection>,

    /// Extra output to write to separate files.
    ///
    /// Valid values: metadata, ir, irOptimized, ewasm, evm.assembly
    #[clap(long, min_values = 1)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_output_files: Vec<ContractOutputSelection>,
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
        let err = "contract source info format must be `<path>:<contractname>` or `<contractname>`";
        let mut iter = s.rsplit(':');
        let name = iter.next().ok_or_else(|| eyre::eyre!(err))?.trim().to_string();
        let path = iter.next().map(str::to_string);

        if name.ends_with(".sol") || name.contains('/') {
            eyre::bail!(err)
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
            .ok_or_else(|| eyre::eyre!("Expected `<path>:<contractname>`, got `{s}`"))?;
        Ok(Self { path: path.to_string(), name: name.trim().to_string() })
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
    pub url: Option<String>,
    /// Optional tag corresponding to a Git SHA, tag, or branch.
    pub tag: Option<String>,
    /// Optional alias of the dependency
    pub alias: Option<String>,
}

const GITHUB: &str = "github.com";
const VERSION_SEPARATOR: char = '@';
const ALIAS_SEPARATOR: char = '=';

impl FromStr for Dependency {
    type Err = eyre::Error;
    fn from_str(dependency: &str) -> Result<Self, Self::Err> {
        // everything before "=" should be considered the alias
        let (mut alias, dependency) = if let Some(split) = dependency.split_once(ALIAS_SEPARATOR) {
            (Some(String::from(split.0)), split.1)
        } else {
            (None, dependency)
        };

        let url_with_version = if let Some(captures) = GH_REPO_PREFIX_REGEX.captures(dependency) {
            let brand = captures.get(5).unwrap().as_str();
            let tld = captures.get(6).unwrap().as_str();
            let project = GH_REPO_PREFIX_REGEX.replace(dependency, "");
            Some(format!("https://{}.{}/{}", brand, tld, project))
        } else {
            // If we don't have a URL and we don't have a valid
            // GitHub repository name, then we assume this is the alias.
            //
            // This is to allow for conveniently removing aliased dependencies
            // using `forge remove <alias>`
            if !GH_REPO_REGEX.is_match(dependency) {
                alias = Some(dependency.to_string());
                None
            } else {
                Some(format!("https://{GITHUB}/{dependency}"))
            }
        };

        // everything after the "@" should be considered the version
        let (url, name, tag) = if let Some(url_with_version) = url_with_version {
            let mut split = url_with_version.split(VERSION_SEPARATOR);
            let url = split
                .next()
                .ok_or_else(|| eyre::eyre!("no dependency path was provided"))?
                .to_string();
            let name = url
                .split('/')
                .last()
                .ok_or_else(|| eyre::eyre!("no dependency name found"))?
                .to_string();
            let tag = split.next().map(ToString::to_string);

            (Some(url), Some(name), tag)
        } else {
            (None, None, None)
        };

        Ok(Dependency { name: name.or_else(|| alias.clone()).unwrap(), url, tag, alias })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dependencies() {
        [
            ("gakonst/lootloose", "https://github.com/gakonst/lootloose", None, None),
            ("github.com/gakonst/lootloose", "https://github.com/gakonst/lootloose", None, None),
            (
                "https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "git+https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "git@github.com:gakonst/lootloose@v1",
                "https://github.com/gakonst/lootloose",
                Some("v1"),
                None,
            ),
            (
                "git@github.com:gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "https://gitlab.com/gakonst/lootloose",
                "https://gitlab.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "https://github.xyz/gakonst/lootloose",
                "https://github.xyz/gakonst/lootloose",
                None,
                None,
            ),
            (
                "gakonst/lootloose@0.1.0",
                "https://github.com/gakonst/lootloose",
                Some("0.1.0"),
                None,
            ),
            (
                "gakonst/lootloose@develop",
                "https://github.com/gakonst/lootloose",
                Some("develop"),
                None,
            ),
            (
                "gakonst/lootloose@98369d0edc900c71d0ec33a01dfba1d92111deed",
                "https://github.com/gakonst/lootloose",
                Some("98369d0edc900c71d0ec33a01dfba1d92111deed"),
                None,
            ),
            ("loot=gakonst/lootloose", "https://github.com/gakonst/lootloose", None, Some("loot")),
            (
                "loot=github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                Some("loot"),
            ),
            (
                "loot=https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                Some("loot"),
            ),
            (
                "loot=git+https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                Some("loot"),
            ),
            (
                "loot=git@github.com:gakonst/lootloose@v1",
                "https://github.com/gakonst/lootloose",
                Some("v1"),
                Some("loot"),
            ),
        ]
        .iter()
        .for_each(|(input, expected_path, expected_tag, expected_alias)| {
            let dep = Dependency::from_str(input).unwrap();
            assert_eq!(dep.url, Some(expected_path.to_string()));
            assert_eq!(dep.tag, expected_tag.map(ToString::to_string));
            assert_eq!(dep.name, "lootloose");
            assert_eq!(dep.alias, expected_alias.map(ToString::to_string));
        });
    }

    #[test]
    fn can_parse_alias_only() {
        let dep = Dependency::from_str("foo").unwrap();
        assert_eq!(dep.name, "foo");
        assert_eq!(dep.url, None);
        assert_eq!(dep.tag, None);
        assert_eq!(dep.alias, Some("foo".to_string()));
    }

    #[test]
    fn test_invalid_github_repo_dependency() {
        let dep = Dependency::from_str("solmate").unwrap();
        assert_eq!(dep.url, None);
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
