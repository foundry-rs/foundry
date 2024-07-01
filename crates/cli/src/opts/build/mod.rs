use clap::Parser;
use foundry_compilers::artifacts::{output_selection::ContractOutputSelection, EvmVersion};
use serde::Serialize;

mod core;
pub use self::core::CoreBuildArgs;

mod paths;
pub use self::paths::ProjectPathsArgs;

// A set of solc compiler settings that can be set via command line arguments, which are intended
// to be merged into an existing `foundry_config::Config`.
//
// See also `BuildArgs`.
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(next_help_heading = "Compiler options")]
pub struct CompilerArgs {
    /// Includes the AST as JSON in the compiler output.
    #[arg(long, help_heading = "Compiler options")]
    #[serde(skip)]
    pub ast: bool,

    /// The target EVM version.
    #[arg(long, value_name = "VERSION")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<EvmVersion>,

    /// Activate the Solidity optimizer.
    #[arg(long)]
    #[serde(skip)]
    pub optimize: bool,

    /// The number of optimizer runs.
    #[arg(long, value_name = "RUNS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_runs: Option<usize>,

    /// Extra output to include in the contract's artifact.
    ///
    /// Example keys: evm.assembly, ewasm, ir, irOptimized, metadata
    ///
    /// For a full description, see <https://docs.soliditylang.org/en/v0.8.13/using-the-compiler.html#input-description>
    #[arg(long, num_args(1..), value_name = "SELECTOR")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_output: Vec<ContractOutputSelection>,

    /// Extra output to write to separate files.
    ///
    /// Valid values: metadata, ir, irOptimized, ewasm, evm.assembly
    #[arg(long, num_args(1..), value_name = "SELECTOR")]
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
