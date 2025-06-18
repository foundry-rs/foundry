use clap::{Parser, ValueHint};
use eyre::{Result, eyre};
use forge_lint::{
    linter::Linter,
    sol::{SolLint, SolLintError, SolidityLinter},
};
use foundry_cli::utils::{FoundryPathExt, LoadConfig};
use foundry_compilers::{solc::SolcLanguage, utils::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, impl_figment_convert_basic, lint::Severity};
use std::path::PathBuf;

/// CLI arguments for `forge lint`.
#[derive(Clone, Debug, Parser)]
pub struct LintArgs {
    /// Path to the file to be checked. Overrides the `ignore` project config.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Specifies which lints to run based on severity. Overrides the `severity` project config.
    ///
    /// Supported values: `high`, `med`, `low`, `info`, `gas`.
    #[arg(long, value_name = "SEVERITY", num_args(1..))]
    severity: Option<Vec<Severity>>,

    /// Specifies which lints to run based on their ID (e.g., "incorrect-shift"). Overrides the
    /// `exclude_lints` project config.
    #[arg(long = "only-lint", value_name = "LINT_ID", num_args(1..))]
    lint: Option<Vec<String>>,

    /// Activates the linter's JSON formatter (rustc-compatible).
    #[arg(long)]
    json: bool,
}

impl_figment_convert_basic!(LintArgs);

impl LintArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let project = config.project()?;
        let path_config = config.project_paths();

        // Expand ignore globs and canonicalize from the get go
        let ignored = expand_globs(&config.root, config.lint.ignore.iter())?
            .iter()
            .flat_map(foundry_common::fs::canonicalize_path)
            .collect::<Vec<_>>();

        let cwd = std::env::current_dir()?;
        let input = match &self.paths[..] {
            [] => {
                // Retrieve the project paths, and filter out the ignored ones.
                config
                    .project_paths::<SolcLanguage>()
                    .input_files_iter()
                    .filter(|p| !(ignored.contains(p) || ignored.contains(&cwd.join(p))))
                    .collect()
            }
            paths => {
                // Override default excluded paths and only lint the input files.
                let mut inputs = Vec::with_capacity(paths.len());
                for path in paths {
                    if path.is_dir() {
                        inputs
                            .extend(foundry_compilers::utils::source_files(path, SOLC_EXTENSIONS));
                    } else if path.is_sol() {
                        inputs.push(path.to_path_buf());
                    } else {
                        warn!("Cannot process path {}", path.display());
                    }
                }
                inputs
            }
        };

        if input.is_empty() {
            sh_println!("Nothing to lint")?;
            return Ok(());
        }

        let parse_lints = |lints: &[String]| -> Result<Vec<SolLint>, SolLintError> {
            lints.iter().map(|s| SolLint::try_from(s.as_str())).collect()
        };

        // Override default lint config with user-defined lints
        let (include, exclude) = match &self.lint {
            Some(cli_lints) => (Some(parse_lints(cli_lints)?), None),
            None => (None, Some(parse_lints(&config.lint.exclude_lints)?)),
        };

        // Override default severity config with user-defined severity
        let severity = match self.severity {
            Some(target) => target,
            None => config.lint.severity,
        };

        if project.compiler.solc.is_none() {
            return Err(eyre!("Linting not supported for this language"));
        }

        let linter = SolidityLinter::new(path_config)
            .with_json_emitter(self.json)
            .with_description(true)
            .with_lints(include)
            .without_lints(exclude)
            .with_severity(if severity.is_empty() { None } else { Some(severity) });

        linter.lint(&input);

        Ok(())
    }
}
