use clap::{Parser, ValueHint};
use eyre::Result;
use forge_lint::{
    linter::{Linter, Severity},
    sol::{SolLint, SolLintError, SolidityLinter},
};
use foundry_cli::utils::LoadConfig;
use foundry_config::impl_figment_convert_basic;
use std::{collections::HashSet, path::PathBuf};

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

    /// Specifies which lints to run based on severity.
    ///
    /// Supported values: `high`, `med`, `low`, `info`, `gas`.
    #[arg(long, value_name = "SEVERITY", num_args(1..))]
    severity: Option<Vec<Severity>>,

    /// Specifies which lints to run based on their ID (e.g., "incorrect-shift").
    ///
    /// Cannot be used with --deny-lint.
    #[arg(long = "only-lint", value_name = "LINT_ID", num_args(1..), conflicts_with = "exclude_lint")]
    include_lint: Option<Vec<String>>,

    /// Deny specific lints based on their ID (e.g., "function-mixed-case").
    ///
    /// Cannot be used with --only-lint.
    #[arg(long = "deny-lint", value_name = "LINT_ID", num_args(1..), conflicts_with = "include_lint")]
    exclude_lint: Option<Vec<String>>,
}

impl_figment_convert_basic!(LintArgs);

impl LintArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let project = config.project()?;

        // Get all source files from the project
        let mut sources =
            project.paths.read_input_files()?.keys().cloned().collect::<Vec<PathBuf>>();

        // Add included paths to sources
        if let Some(include_paths) = &self.include {
            let included =
                include_paths.iter().filter(|path| path.exists()).cloned().collect::<Vec<_>>();
            sources.extend(included);
        }

        // Remove excluded files from sources
        if let Some(exclude_paths) = &self.exclude {
            let excluded = exclude_paths.iter().cloned().collect::<HashSet<_>>();
            sources.retain(|path| !excluded.contains(path));
        }

        if sources.is_empty() {
            sh_println!("Nothing to lint")?;
            std::process::exit(0);
        }

        if project.compiler.solc.is_some() {
            let mut linter = SolidityLinter::new().with_severity(self.severity);

            // Resolve and apply included lints if provided
            if let Some(ref include_ids) = self.include_lint {
                let included_lints = include_ids 
                    .iter()
                    .map(|id_str| SolLint::try_from(id_str.as_str()))
                    .collect::<Result<Vec<SolLint>, SolLintError>>()?;
                linter = linter.with_lints(Some(included_lints));
            }

            // Resolve and apply excluded lints if provided
            if let Some(ref exclude_ids) = self.exclude_lint {
                let excluded_lints = exclude_ids
                    .iter()
                    .map(|id_str| SolLint::try_from(id_str.as_str()))
                    .collect::<Result<Vec<SolLint>, SolLintError>>()?;
                linter = linter.without_lints(Some(excluded_lints));
            }

            linter.lint(&sources);
        } else {
            todo!("Linting not supported for this language");
        };

        Ok(())
    }
}
