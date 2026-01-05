use clap::{Parser, ValueHint};
use eyre::{Result, eyre};
use forge_lint::{
    linter::Linter,
    sol::{SolLint, SolLintError, SolidityLinter},
};
use foundry_cli::{
    opts::{BuildOpts, configure_pcx_from_solc, get_solar_sources_from_compile_output},
    utils::{FoundryPathExt, LoadConfig},
};
use foundry_common::{compile::ProjectCompiler, shell};
use foundry_compilers::{solc::SolcLanguage, utils::SOLC_EXTENSIONS};
use foundry_config::{filter::expand_globs, lint::Severity};
use std::path::PathBuf;

/// CLI arguments for `forge lint`.
#[derive(Clone, Debug, Parser)]
pub struct LintArgs {
    /// Path to the file to be checked. Overrides the `ignore` project config.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH", num_args(1..))]
    pub(crate) paths: Vec<PathBuf>,

    /// Specifies which lints to run based on severity. Overrides the `severity` project config.
    ///
    /// Supported values: `high`, `med`, `low`, `info`, `gas`.
    #[arg(long, value_name = "SEVERITY", num_args(1..))]
    pub(crate) severity: Option<Vec<Severity>>,

    /// Specifies which lints to run based on their ID (e.g., "incorrect-shift"). Overrides the
    /// `exclude_lints` project config.
    #[arg(long = "only-lint", value_name = "LINT_ID", num_args(1..))]
    pub(crate) lint: Option<Vec<String>>,

    #[command(flatten)]
    pub(crate) build: BuildOpts,
}

foundry_config::impl_figment_convert!(LintArgs, build);

impl LintArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let project = config.solar_project()?;
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
                        warn!("cannot process path {}", path.display());
                    }
                }
                inputs
            }
        };

        if input.is_empty() {
            sh_println!("nothing to lint")?;
            return Ok(());
        }

        let parse_lints = |lints: &[String]| -> Result<Vec<SolLint>, SolLintError> {
            lints.iter().map(|s| SolLint::try_from(s.as_str())).collect()
        };

        // Override default lint config with user-defined lints
        // When --only-lint is used, bypass the severity filter by setting it to None
        let (include, exclude, severity) = match &self.lint {
            Some(cli_lints) => (Some(parse_lints(cli_lints)?), None, vec![]),
            None => {
                let severity = self.severity.clone().unwrap_or(config.lint.severity.clone());
                (None, Some(parse_lints(&config.lint.exclude_lints)?), severity)
            }
        };

        if project.compiler.solc.is_none() {
            return Err(eyre!("linting not supported for this language"));
        }

        let linter = SolidityLinter::new(path_config)
            .with_json_emitter(shell::is_json())
            .with_description(true)
            .with_lints(include)
            .without_lints(exclude)
            .with_severity(if severity.is_empty() { None } else { Some(severity) })
            .with_mixed_case_exceptions(&config.lint.mixed_case_exceptions);

        let output = ProjectCompiler::new().files(input.iter().cloned()).compile(&project)?;
        let solar_sources = get_solar_sources_from_compile_output(&config, &output, Some(&input))?;
        if solar_sources.input.sources.is_empty() {
            return Err(eyre!(
                "unable to lint. Solar only supports Solidity versions prior to 0.8.0"
            ));
        }

        // NOTE(rusowsky): Once solar can drop unsupported versions, rather than creating a new
        // compiler, we should reuse the parser from the project output.
        let mut compiler = solar::sema::Compiler::new(
            solar::interface::Session::builder().with_stderr_emitter().build(),
        );

        // Load the solar-compatible sources to the pcx before linting
        compiler.enter_mut(|compiler| {
            let mut pcx = compiler.parse();
            pcx.set_resolve_imports(true);
            configure_pcx_from_solc(&mut pcx, &config.project_paths(), &solar_sources, true);
            pcx.parse();
        });
        linter.lint(&input, config.deny, &mut compiler)?;

        Ok(())
    }
}
