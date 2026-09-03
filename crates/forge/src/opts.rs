use crate::cmd::{
    bind::BindArgs, bind_json, build::BuildArgs, cache::CacheArgs, clone::CloneArgs,
    compiler::CompilerArgs, config, coverage, create::CreateArgs, doc::DocArgs, eip712, flatten,
    fmt::FmtArgs, fuzz::FuzzArgs, geiger, init::InitArgs, inspect, install::InstallArgs,
    lint::LintArgs, lsp::LspArgs, reinit::ReinitArgs, remappings::RemappingArgs,
    remove::RemoveArgs, selectors::SelectorsSubcommands, snapshot, soldeer, test, tree, update,
};
use clap::{Parser, Subcommand, ValueHint};
use forge_script::ScriptArgs;
use forge_verify::{VerifyArgs, VerifyBytecodeArgs, VerifyCheckArgs};
use foundry_cli::opts::GlobalArgs;
use foundry_common::version::{LONG_VERSION, SHORT_VERSION};
use std::path::PathBuf;

/// Build, test, fuzz, debug and deploy Solidity contracts.
#[derive(Parser)]
#[command(
    name = "forge",
    version = SHORT_VERSION,
    long_version = LONG_VERSION,
    after_help = "Find more information in the book: https://getfoundry.sh/forge/overview",
    next_display_order = None,
)]
pub struct Forge {
    /// Include the global arguments.
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub cmd: ForgeSubcommand,
}

#[derive(Subcommand)]
pub enum ForgeSubcommand {
    /// Run the project's tests
    ///
    /// Examples:
    /// - forge test
    /// - forge test --match-test test_Increment -vvvv (show traces for a matching test)
    /// - forge test --match-contract CounterTest --fuzz-runs 1000
    #[command(verbatim_doc_comment, visible_alias = "t")]
    Test(test::TestArgs),

    /// Run and manage Forge fuzzing corpora.
    Fuzz(FuzzArgs),

    /// Run a smart contract as a script, building transactions that can be sent onchain
    ///
    /// Examples:
    /// - forge script script/Counter.s.sol (simulate the script locally)
    /// - forge script script/Counter.s.sol --rpc-url $RPC_URL --broadcast --account dev
    /// - forge script script/Counter.s.sol --rpc-url $RPC_URL --resume
    #[command(verbatim_doc_comment)]
    Script(ScriptArgs),

    /// Generate coverage reports
    ///
    /// Examples:
    /// - forge coverage
    /// - forge coverage --report lcov --report-file lcov.info
    /// - forge coverage --match-contract CounterTest
    #[command(verbatim_doc_comment)]
    Coverage(coverage::CoverageArgs),

    /// Generate Rust bindings for smart contracts.
    #[command(alias = "bi")]
    Bind(BindArgs),

    /// Build the project's smart contracts
    ///
    /// Examples:
    /// - forge build
    /// - forge build --sizes (print a contract size report)
    /// - forge build --watch (rebuild on file changes)
    #[command(verbatim_doc_comment, visible_aliases = ["b", "compile"])]
    Build {
        /// Require foundry.lock to match direct Git dependency submodules.
        #[arg(long)]
        locked: bool,
        #[command(flatten)]
        args: BuildArgs,
    },

    /// Clone a contract from Etherscan
    ///
    /// Examples:
    /// - forge clone $WETH weth --etherscan-api-key $KEY (clone WETH into ./weth)
    /// - forge clone --chain sepolia --etherscan-api-key $KEY $ADDRESS my-contract
    #[command(verbatim_doc_comment)]
    Clone(CloneArgs),

    /// Update one or multiple dependencies.
    ///
    /// If no arguments are provided, then all dependencies are updated.
    #[command(visible_alias = "u")]
    Update(update::UpdateArgs),

    /// Install one or multiple dependencies
    ///
    /// If no arguments are provided, then existing dependencies will be installed.
    ///
    /// Examples:
    /// - forge install (install all dependencies of the project)
    /// - forge install openzeppelin/openzeppelin-contracts
    /// - forge install openzeppelin/openzeppelin-contracts@v5.0.2 (pin a version)
    #[command(verbatim_doc_comment, visible_aliases = ["i", "add"])]
    Install(InstallArgs),

    /// Reinitialize the project's Git submodules, discarding local changes.
    Reinit(ReinitArgs),

    /// Remove one or multiple dependencies.
    #[command(visible_alias = "rm")]
    Remove(RemoveArgs),

    /// Get the automatically inferred remappings for the project.
    #[command(visible_alias = "re")]
    Remappings(RemappingArgs),

    /// Verify smart contracts on Etherscan and Sourcify
    ///
    /// Examples:
    /// - forge verify-contract --chain sepolia $ADDRESS src/Counter.sol:Counter --watch
    /// - forge verify-contract $ADDRESS src/Counter.sol:Counter --verifier sourcify
    #[command(verbatim_doc_comment, visible_alias = "v")]
    VerifyContract(VerifyArgs),

    /// Check verification status on the selected verifier.
    #[command(visible_alias = "vc")]
    VerifyCheck(VerifyCheckArgs),

    /// Verify the deployed bytecode against its source on Etherscan.
    #[command(visible_alias = "vb")]
    VerifyBytecode(VerifyBytecodeArgs),

    /// Deploy a smart contract
    ///
    /// Examples:
    /// - forge create Counter --rpc-url $RPC_URL --account dev --broadcast
    /// - forge create Token --private-key $PK --broadcast --constructor-args Token TKN
    #[command(verbatim_doc_comment, visible_alias = "c")]
    Create(CreateArgs),

    /// Create a new Forge project.
    Init(InitArgs),

    /// Generate shell completions script.
    #[command(visible_alias = "com")]
    Completions {
        #[arg(value_enum)]
        shell: foundry_cli::clap::Shell,
    },

    /// Remove the build artifacts and cache directories.
    #[command(visible_alias = "cl")]
    Clean {
        /// The project's root path.
        ///
        /// By default root of the Git repository, if in one,
        /// or the current working directory.
        #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
        root: Option<PathBuf>,
    },

    /// Manage the Foundry cache.
    Cache(CacheArgs),

    /// Create a gas snapshot of each test's gas usage
    ///
    /// Examples:
    /// - forge snapshot
    /// - forge snapshot --diff (compare against the existing .gas-snapshot file)
    /// - forge snapshot --check (fail if gas usage does not match .gas-snapshot)
    #[command(verbatim_doc_comment, visible_alias = "s")]
    Snapshot(snapshot::GasSnapshotArgs),

    /// Display the current config.
    #[command(visible_alias = "co")]
    Config(config::ConfigArgs),

    /// Flatten a source file and all of its imports into one file.
    #[command(visible_alias = "f")]
    Flatten(flatten::FlattenArgs),

    /// Format Solidity source files
    ///
    /// Examples:
    /// - forge fmt
    /// - forge fmt --check (report formatting issues without writing changes)
    /// - forge fmt src/Counter.sol
    #[command(verbatim_doc_comment)]
    Fmt(FmtArgs),

    /// Lint Solidity source files
    #[command(visible_alias = "l")]
    Lint(LintArgs),

    /// Start the Solar language server.
    Lsp(LspArgs),

    /// Get specialized information about a smart contract
    ///
    /// Examples:
    /// - forge inspect Counter abi
    /// - forge inspect Counter bytecode
    /// - forge inspect src/Counter.sol:Counter storageLayout
    #[command(verbatim_doc_comment, visible_alias = "in")]
    Inspect(inspect::InspectArgs),

    /// Display a tree visualization of the project's dependency graph.
    #[command(visible_alias = "tr")]
    Tree(tree::TreeArgs),

    /// DEPRECATED: Detects usage of unsafe cheat codes in a project and its dependencies.
    ///
    /// This is an alias for `forge lint --only-lint unsafe-cheatcode`.
    Geiger(geiger::GeigerArgs),

    /// Generate documentation for the project.
    Doc(DocArgs),

    /// Function selector utilities.
    #[command(visible_alias = "se")]
    Selectors {
        #[command(subcommand)]
        command: SelectorsSubcommands,
    },

    /// Compiler utilities.
    Compiler(CompilerArgs),

    /// Soldeer dependency manager.
    Soldeer(soldeer::SoldeerArgs),

    /// Generate EIP-712 struct encodings for structs from a given file.
    Eip712(eip712::Eip712Args),

    /// Generate bindings for serialization/deserialization of project structs via JSON cheatcodes.
    BindJson(bind_json::BindJsonArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Forge::command().debug_assert();
    }

    #[test]
    fn parse_lsp_args() {
        let args = Forge::try_parse_from(["forge", "lsp", "--stdio"]).unwrap();
        let ForgeSubcommand::Lsp(args) = args.cmd else {
            panic!("expected lsp subcommand");
        };
        assert!(args.stdio);
    }
}
