use clap::{Parser, ValueHint};
use eyre::{bail, Result};
use forge_lint::{Linter, OutputFormat, Severity};
use foundry_cli::utils::LoadConfig;
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
        let root = if let Some(root) = &self.root { root } else { &config.root };

        // Expand ignore globs and canonicalize paths
        let mut ignored = expand_globs(&root, config.fmt.ignore.iter())?
            .iter()
            .flat_map(foundry_common::fs::canonicalize_path)
            .collect::<HashSet<_>>();

        // Add explicitly excluded paths to the ignored set
        if let Some(exclude_paths) = &self.exclude {
            ignored.extend(exclude_paths.iter().flat_map(foundry_common::fs::canonicalize_path));
        }

        let entries = fs::read_dir(root).unwrap();
        println!("Files in directory: {}", root.display());
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            println!("{}", path.display());
        }

        let mut input: Vec<PathBuf> = if let Some(include_paths) = &self.include {
            include_paths.iter().filter(|path| path.exists()).cloned().collect()
        } else {
            source_files_iter(&root, SOLC_EXTENSIONS)
                .filter(|p| !(ignored.contains(p) || ignored.contains(&root.join(p))))
                .collect()
        };

        input.retain(|path| !ignored.contains(path));

        if input.is_empty() {
            bail!("No source files found in path");
        }

        // TODO: maybe compile and lint on the aggreagted compiler output?
        Linter::new(input)
            .with_severity(self.severity)
            .with_description(self.with_description)
            .lint();

        Ok(())
    }
}

pub struct ProjectLinter {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    pub file: String,
    pub start: i32,
    pub end: i32,
}
