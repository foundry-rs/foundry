use clap::{Parser, ValueHint};
use eyre::{bail, Result};
use forge_lint::sol::SolidityLinter;
use forge_lint::{Linter, OutputFormat, ProjectLinter, Severity};
use foundry_cli::utils::LoadConfig;
use foundry_common::shell;
use foundry_compilers::utils::{source_files_iter, SOLC_EXTENSIONS};
use foundry_config::filter::expand_globs;
use foundry_config::impl_figment_convert_basic;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// CLI arguments for `forge lint`.
#[derive(Clone, Debug, Parser)]
pub struct LintArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Include only the specified files when linting.
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "FILES", num_args(1..))]
    include: Option<Vec<PathBuf>>,

    /// Exclude the specified files when linting.
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "FILES", num_args(1..))]
    exclude: Option<Vec<PathBuf>>,

    // TODO: support writing to output file
    /// Format of the output.
    ///
    /// Supported values: `json` or `markdown`.
    #[arg(long, value_name = "FORMAT", default_value = "json")]
    format: OutputFormat,

    /// Specifies which lints to run based on severity.
    ///
    /// Supported values: `high`, `med`, `low`, `info`, `gas`.
    #[arg(long, value_name = "SEVERITY", num_args(1..))]
    severity: Option<Vec<Severity>>,

    /// Show descriptions in the output.
    ///
    /// Disabled by default to avoid long console output.
    #[arg(long)]
    with_description: bool,
}

impl_figment_convert_basic!(LintArgs);

impl LintArgs {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;
        let project = config.project()?;

        // TODO: Update this to infer the linter from the project, just hard coding to solidity for now
        let linter = SolidityLinter::new();
        let output = ProjectLinter::new(linter).lint(&project)?;

        // let format_json = shell::is_json();

        // if format_json && !self.names && !self.sizes {
        //     sh_println!("{}", serde_json::to_string_pretty(&output.output())?)?;
        // }

        Ok(())
    }
}
