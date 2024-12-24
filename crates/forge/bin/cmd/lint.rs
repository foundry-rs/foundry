use clap::ValueEnum;
use clap::{Parser, ValueHint};
use eyre::Result;
use forge_lint::Linter;
use foundry_cli::utils::{FoundryPathExt, LoadConfig};
use foundry_compilers::{compilers::solc::SolcLanguage, solc::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, impl_figment_convert_basic};
use std::collections::HashSet;
use std::{
    io,
    io::Read,
    path::{Path, PathBuf},
};

/// CLI arguments for `forge lint`.
#[derive(Clone, Debug, Parser)]
pub struct LintArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Include only the specified files.
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "FILES", num_args(1..))]
    include: Option<Vec<PathBuf>>,

    /// Exclude the specified files.
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "FILES", num_args(1..))]
    exclude: Option<Vec<PathBuf>>,

    /// Format of the output.
    ///
    /// Supported values: `json` or `markdown`.
    #[arg(long, value_name = "FORMAT", default_value = "json")]
    format: OutputFormat,

    // TODO: output file
    /// Use only selected severities for output.
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
        let root = self.root.unwrap_or_else(|| std::env::current_dir().unwrap());

        let mut paths: Vec<PathBuf> = if let Some(include_paths) = &self.include {
            include_paths.iter().filter(|path| path.exists()).cloned().collect()
        } else {
            foundry_compilers::utils::source_files_iter(&root, &[".sol"]).collect()
        };

        if let Some(exclude_paths) = &self.exclude {
            let exclude_set = exclude_paths.iter().collect::<HashSet<_>>();
            paths.retain(|path| !exclude_set.contains(path));
        }

        Linter::new(paths)
            .with_severity(self.severity)
            .with_description(self.with_description)
            .lint();

        Ok(())
    }
}
