use clap::{Parser, ValueHint};
use eyre::Result;
use forge_lint::{
    linter::Linter,
    sol::{SolLint, SolLintError, SolidityLinter},
};
use foundry_cli::utils::{FoundryPathExt, LoadConfig};
use foundry_compilers::{solc::SolcLanguage, utils::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, impl_figment_convert_basic, lint::Severity};
use std::{collections::HashSet, path::PathBuf};

/// CLI arguments for `forge lint`.
#[derive(Clone, Debug, Parser)]
pub struct LintArgs {
    /// Path to the file.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Specifies which lints to run based on severity. Overrides the project config.
    ///
    /// Supported values: `high`, `med`, `low`, `info`, `gas`.
    #[arg(long, value_name = "SEVERITY", num_args(1..))]
    severity: Option<Vec<Severity>>,

    /// Specifies which lints to run based on their ID (e.g., "incorrect-shift"). Overrides the
    /// project config.
    #[arg(long = "only-lint", value_name = "LINT_ID", num_args(1..))]
    lint: Option<Vec<String>>,
}

impl_figment_convert_basic!(LintArgs);

impl LintArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let project = config.project()?;

        // Expand ignore globs and canonicalize from the get go
        let ignored = expand_globs(&config.root, config.lint.ignore.iter())?
            .iter()
            .flat_map(foundry_common::fs::canonicalize_path)
            .collect::<Vec<_>>();

        let cwd = std::env::current_dir()?;
        let input = match &self.paths[..] {
            [] => {
                // Retrieve the project paths, and filter out the ignored ones.
                let project_paths = config
                    .project_paths::<SolcLanguage>()
                    .input_files_iter()
                    .filter(|p| !(ignored.contains(p) || ignored.contains(&cwd.join(p))))
                    .collect();
                project_paths
            }
            paths => {
                let mut inputs = Vec::with_capacity(paths.len());
                for path in paths {
                    if !ignored.is_empty() &&
                        ((path.is_absolute() && ignored.contains(path)) ||
                            ignored.contains(&cwd.join(path)))
                    {
                        continue
                    }

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
            std::process::exit(0);
        }

        // Helper to convert strings to `SolLint` objects
        let convert_lints = |lints: &[String]| -> Result<Vec<SolLint>, SolLintError> {
            lints.iter().map(|s| SolLint::try_from(s.as_str())).collect()
        };

        // Override default lint config with user-defined lints
        let (include, exclude) = if let Some(cli_lints) = &self.lint {
            let include_lints = convert_lints(cli_lints)?;
            let target_ids: HashSet<&str> = cli_lints.iter().map(String::as_str).collect();
            let filtered_excludes = config
                .lint
                .exclude_lints
                .iter()
                .filter(|l| !target_ids.contains(l.as_str()))
                .cloned()
                .collect::<Vec<_>>();

            (include_lints, convert_lints(&filtered_excludes)?)
        } else {
            (convert_lints(&config.lint.include_lints)?, convert_lints(&config.lint.exclude_lints)?)
        };

        // Override default severity config with user-defined severity
        let severity = match self.severity {
            Some(target) => target,
            None => config.lint.severity,
        };

        if project.compiler.solc.is_some() {
            let linter = SolidityLinter::new()
                .with_lints(if include.is_empty() { None } else { Some(include) })
                .without_lints(if exclude.is_empty() { None } else { Some(exclude) })
                .with_severity(if severity.is_empty() { None } else { Some(severity) });

            linter.lint(&input);
        } else {
            todo!("Linting not supported for this language");
        };

        Ok(())
    }
}
