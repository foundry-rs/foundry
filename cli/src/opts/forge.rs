use crate::cmd::forge::{
    bind::BindArgs,
    build::BuildArgs,
    cache::CacheArgs,
    config, coverage,
    create::CreateArgs,
    debug::DebugArgs,
    doc::DocArgs,
    flatten,
    fmt::FmtArgs,
    fourbyte::UploadSelectorsArgs,
    geiger,
    init::InitArgs,
    inspect,
    install::InstallArgs,
    remappings::RemappingArgs,
    remove::RemoveArgs,
    script::ScriptArgs,
    snapshot, test, tree, update,
    verify::{VerifyArgs, VerifyCheckArgs},
};
use clap::{Parser, Subcommand, ValueHint};
use ethers::solc::{artifacts::output_selection::ContractOutputSelection, EvmVersion};
use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Parser)]
#[clap(name = "forge", version = crate::utils::VERSION_MESSAGE)]
pub struct Opts {
    #[clap(subcommand)]
    pub sub: Subcommands,
}

#[derive(Debug, Subcommand)]
#[clap(
    about = "Build, test, fuzz, debug and deploy Solidity contracts.",
    after_help = "Find more information in the book: http://book.getfoundry.sh/reference/forge/forge.html",
    next_display_order = None
)]
#[allow(clippy::large_enum_variant)]
pub enum Subcommands {
    #[clap(visible_alias = "t", about = "Run the project's tests.")]
    Test(test::TestArgs),

    #[clap(
        about = "Run a smart contract as a script, building transactions that can be sent onchain."
    )]
    Script(ScriptArgs),

    #[clap(about = "Generate coverage reports.")]
    Coverage(coverage::CoverageArgs),

    #[clap(alias = "bi", about = "Generate Rust bindings for smart contracts.")]
    Bind(BindArgs),

    #[clap(visible_aliases = ["b", "compile"], about = "Build the project's smart contracts.")]
    Build(BuildArgs),

    #[clap(visible_alias = "d", about = "Debugs a single smart contract as a script.")]
    Debug(DebugArgs),

    #[clap(
        visible_alias = "u",
        about = "Update one or multiple dependencies.",
        long_about = "Update one or multiple dependencies. If no arguments are provided, then all dependencies are updated."
    )]
    Update(update::UpdateArgs),

    #[clap(
        visible_alias = "i",
        about = "Install one or multiple dependencies.",
        long_about = "Install one or multiple dependencies. If no arguments are provided, then existing dependencies will be installed."
    )]
    Install(InstallArgs),

    #[clap(visible_alias = "rm", about = "Remove one or multiple dependencies.")]
    Remove(RemoveArgs),

    #[clap(
        visible_alias = "re",
        about = "Get the automatically inferred remappings for the project."
    )]
    Remappings(RemappingArgs),

    #[clap(visible_alias = "v", about = "Verify smart contracts on Etherscan.")]
    VerifyContract(VerifyArgs),

    #[clap(visible_alias = "vc", about = "Check verification status on Etherscan.")]
    VerifyCheck(VerifyCheckArgs),

    #[clap(visible_alias = "c", about = "Deploy a smart contract.")]
    Create(CreateArgs),

    #[clap(about = "Create a new Forge project.")]
    Init(InitArgs),

    #[clap(visible_alias = "com", about = "Generate shell completions script.")]
    Completions {
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },
    #[clap(visible_alias = "fig", about = "Generate Fig autocompletion spec.")]
    GenerateFigSpec,
    #[clap(visible_alias = "cl", about = "Remove the build artifacts and cache directories.")]
    Clean {
        /// The project's root path.
        ///
        /// By default root of the Git repository, if in one,
        /// or the current working directory.
        #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
        root: Option<PathBuf>,
    },

    #[clap(about = "Manage the Foundry cache.")]
    Cache(CacheArgs),

    #[clap(visible_alias = "s", about = "Create a snapshot of each test's gas usage.")]
    Snapshot(snapshot::SnapshotArgs),

    #[clap(visible_alias = "co", about = "Display the current config.")]
    Config(config::ConfigArgs),

    #[clap(
        visible_alias = "f",
        about = "Flatten a source file and all of its imports into one file."
    )]
    Flatten(flatten::FlattenArgs),

    #[clap(about = "Formats Solidity source files.")]
    Fmt(FmtArgs),

    #[clap(visible_alias = "in", about = "Get specialized information about a smart contract.")]
    Inspect(inspect::InspectArgs),

    #[clap(
        visible_alias = "up",
        about = "Uploads abi of given contract to https://sig.eth.samczsun.com function selector database."
    )]
    UploadSelectors(UploadSelectorsArgs),

    #[clap(
        visible_alias = "tr",
        about = "Display a tree visualization of the project's dependency graph."
    )]
    Tree(tree::TreeArgs),

    #[clap(
        about = "Detects usage of unsafe cheat codes in a foundry project and its dependencies."
    )]
    Geiger(geiger::GeigerArgs),

    #[clap(about = "Generate documentation for the project.")]
    Doc(DocArgs),
}

// A set of solc compiler settings that can be set via command line arguments, which are intended
// to be merged into an existing `foundry_config::Config`.
//
// See also [`BuildArgs`]
#[derive(Default, Debug, Clone, Parser, Serialize)]
#[clap(next_help_heading = "Compiler options")]
pub struct CompilerArgs {
    #[clap(help = "The target EVM version.", long, value_name = "VERSION")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<EvmVersion>,

    #[clap(help = "Activate the Solidity optimizer.", long)]
    #[serde(skip)]
    pub optimize: bool,

    #[clap(help = "The number of optimizer runs.", long, value_name = "RUNS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_runs: Option<usize>,

    /// Extra output to include in the contract's artifact.
    ///
    /// Example keys: evm.assembly, ewasm, ir, irOptimized, metadata
    ///
    /// For a full description, see https://docs.soliditylang.org/en/v0.8.13/using-the-compiler.html#input-description
    #[clap(long, num_args(1..), value_name = "SELECTOR")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_output: Vec<ContractOutputSelection>,

    /// Extra output to write to separate files.
    ///
    /// Valid values: metadata, ir, irOptimized, ewasm, evm.assembly
    #[clap(long, num_args(1..), value_name = "SELECTOR")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_output_files: Vec<ContractOutputSelection>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_evm_version() {
        let args: CompilerArgs =
            CompilerArgs::parse_from(["foundry-cli", "--evm-version", "london"]);
        assert_eq!(args.evm_version, Some(EvmVersion::London));
    }

    #[test]
    fn can_parse_extra_output() {
        let args: CompilerArgs =
            CompilerArgs::parse_from(["foundry-cli", "--extra-output", "metadata", "ir-optimized"]);
        assert_eq!(
            args.extra_output,
            vec![ContractOutputSelection::Metadata, ContractOutputSelection::IrOptimized]
        );
    }

    #[test]
    fn can_parse_extra_output_files() {
        let args: CompilerArgs = CompilerArgs::parse_from([
            "foundry-cli",
            "--extra-output-files",
            "metadata",
            "ir-optimized",
        ]);
        assert_eq!(
            args.extra_output_files,
            vec![ContractOutputSelection::Metadata, ContractOutputSelection::IrOptimized]
        );
    }
}
