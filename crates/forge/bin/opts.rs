use crate::cmd::{
    bind::BindArgs,
    build::BuildArgs,
    cache::CacheArgs,
    config, coverage,
    create::CreateArgs,
    debug::DebugArgs,
    doc::DocArgs,
    flatten,
    fmt::FmtArgs,
    geiger, generate,
    init::InitArgs,
    inspect,
    install::InstallArgs,
    remappings::RemappingArgs,
    remove::RemoveArgs,
    script::ScriptArgs,
    selectors::SelectorsSubcommands,
    snapshot, test, tree, update,
    verify::{VerifyArgs, VerifyCheckArgs},
};
use clap::{Parser, Subcommand, ValueHint};
use foundry_cli::opts::ShellOptions;
use std::path::PathBuf;

const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

/// Build, test, fuzz, debug and deploy Solidity contracts.
#[derive(Parser)]
#[clap(
    name = "forge",
    version = VERSION_MESSAGE,
    after_help = "Find more information in the book: http://book.getfoundry.sh/reference/forge/forge.html",
    next_display_order = None,
)]
pub struct Opts {
    #[clap(subcommand)]
    pub sub: Subcommands,
    #[clap(flatten)]
    pub shell: ShellOptions,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Subcommands {
    /// Run the project's tests.
    #[clap(visible_alias = "t")]
    Test(test::TestArgs),

    /// Run a smart contract as a script, building transactions that can be sent onchain.
    Script(ScriptArgs),

    /// Generate coverage reports.
    Coverage(coverage::CoverageArgs),

    /// Generate Rust bindings for smart contracts.
    #[clap(alias = "bi")]
    Bind(BindArgs),

    /// Build the project's smart contracts.
    #[clap(visible_aliases = ["b", "compile"])]
    Build(BuildArgs),

    /// Debugs a single smart contract as a script.
    #[clap(visible_alias = "d")]
    Debug(DebugArgs),

    /// Update one or multiple dependencies.
    ///
    /// If no arguments are provided, then all dependencies are updated.
    #[clap(visible_alias = "u")]
    Update(update::UpdateArgs),

    /// Install one or multiple dependencies.
    ///
    /// If no arguments are provided, then existing dependencies will be installed.
    #[clap(visible_alias = "i")]
    Install(InstallArgs),

    /// Remove one or multiple dependencies.
    #[clap(visible_alias = "rm")]
    Remove(RemoveArgs),

    /// Get the automatically inferred remappings for the project.
    #[clap(visible_alias = "re")]
    Remappings(RemappingArgs),

    /// Verify smart contracts on Etherscan.
    #[clap(visible_alias = "v")]
    VerifyContract(VerifyArgs),

    /// Check verification status on Etherscan.
    #[clap(visible_alias = "vc")]
    VerifyCheck(VerifyCheckArgs),

    /// Deploy a smart contract.
    #[clap(visible_alias = "c")]
    Create(CreateArgs),

    /// Create a new Forge project.
    Init(InitArgs),

    /// Generate shell completions script.
    #[clap(visible_alias = "com")]
    Completions {
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate Fig autocompletion spec.
    #[clap(visible_alias = "fig")]
    GenerateFigSpec,

    /// Remove the build artifacts and cache directories.
    #[clap(visible_alias = "cl")]
    Clean {
        /// The project's root path.
        ///
        /// By default root of the Git repository, if in one,
        /// or the current working directory.
        #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
        root: Option<PathBuf>,
    },

    /// Manage the Foundry cache.
    Cache(CacheArgs),

    /// Create a snapshot of each test's gas usage.
    #[clap(visible_alias = "s")]
    Snapshot(snapshot::SnapshotArgs),

    /// Display the current config.
    #[clap(visible_alias = "co")]
    Config(config::ConfigArgs),

    /// Flatten a source file and all of its imports into one file.
    #[clap(visible_alias = "f")]
    Flatten(flatten::FlattenArgs),

    /// Format Solidity source files.
    Fmt(FmtArgs),

    /// Get specialized information about a smart contract.
    #[clap(visible_alias = "in")]
    Inspect(inspect::InspectArgs),

    /// Display a tree visualization of the project's dependency graph.
    #[clap(visible_alias = "tr")]
    Tree(tree::TreeArgs),

    /// Detects usage of unsafe cheat codes in a project and its dependencies.
    Geiger(geiger::GeigerArgs),

    /// Generate documentation for the project.
    Doc(DocArgs),

    /// Function selector utilities
    #[clap(visible_alias = "se")]
    Selectors {
        #[clap(subcommand)]
        command: SelectorsSubcommands,
    },

    /// Generate scaffold files.
    Generate(generate::GenerateArgs),
}
