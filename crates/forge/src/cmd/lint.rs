use clap::{Parser, ValueHint};
use eyre::{Result, eyre};
use forge_lint::{
    linter::Linter,
    sol::{SolLint, SolLintError, SolidityLinter},
};
use foundry_cli::{
    opts::{BuildOpts, configure_pcx_all_sources},
    utils::{FoundryPathExt, LoadConfig},
};
use foundry_common::shell;
use foundry_compilers::{FileFilter, solc::SolcLanguage, utils::SOLC_EXTENSIONS};
use foundry_config::{SkipBuildFilters, filter::expand_globs, lint::Severity};
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
        let format_json = shell::is_json();
        let config = self.load_config()?;
        let project = config.ephemeral_project()?;
        let path_config = config.project_paths();

        // Expand ignore globs and canonicalize from the get go
        let ignored = expand_globs(&config.root, config.lint.ignore.iter())?
            .iter()
            .flat_map(foundry_common::fs::canonicalize_path)
            .collect::<Vec<_>>();

        let cwd = std::env::current_dir()?;
        let mut input = match &self.paths[..] {
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
                        inputs.push(path.clone());
                    } else {
                        warn!("cannot process path {}", path.display());
                    }
                }
                inputs
            }
        };
        let skip = SkipBuildFilters::new(config.skip.clone(), config.root.clone());
        input.retain(|path| skip.is_match(path));

        if input.is_empty() {
            sh_status!("nothing to lint")?;
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
            .with_json_emitter(format_json)
            .with_json_emitter_stdout(format_json)
            .with_description(true)
            .with_lints(include)
            .without_lints(exclude)
            .with_severity(if severity.is_empty() { None } else { Some(severity) })
            .with_lint_specific(&config.lint.lint_specific);

        let mut opts = solar::interface::config::CompileOpts::default();
        if format_json {
            opts.error_format = solar::interface::config::ErrorFormat::RustcJson;
        }
        let session = solar::interface::Session::builder().opts(opts);
        let session =
            if format_json { session.build() } else { session.with_stderr_emitter().build() };
        if format_json {
            let writer = Box::new(std::io::BufWriter::new(std::io::stdout()));
            let emitter = solar::interface::diagnostics::JsonEmitter::new(
                writer,
                session.clone_source_map(),
                solar::interface::ColorChoice::Never,
            )
            .rustc_like(true);
            session.dcx.set_emitter(Box::new(emitter));
        }
        let mut compiler = solar::sema::Compiler::new(session);

        // Load the solar-compatible sources to the pcx before linting
        let has_sources = compiler.enter_mut(|compiler| -> Result<bool> {
            let mut pcx = compiler.parse();
            pcx.set_resolve_imports(true);
            let has_sources =
                configure_pcx_all_sources(&mut pcx, &config, Some(&project), Some(&input))?;
            pcx.parse();
            Ok(has_sources)
        })?;
        if !has_sources {
            return Err(eyre!("unable to lint. Solar only supports Solidity versions >=0.8.0"));
        }
        if let Err(err) = linter.lint(&input, config.deny, &mut compiler) {
            if format_json && compiler.dcx().has_errors().is_err() {
                // Solar already emitted the error, so bypass the top-level error printer.
                std::process::exit(1);
            }
            return Err(err);
        }

        Ok(())
    }
}
