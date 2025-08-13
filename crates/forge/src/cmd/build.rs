use super::{install, watch::WatchArgs};
use clap::Parser;
use eyre::{Result, eyre};
use forge_lint::{linter::Linter, sol::SolidityLinter};
use foundry_cli::{
    opts::{BuildOpts, solar_pcx_from_build_opts},
    utils::{LoadConfig, cache_local_signatures},
};
use foundry_common::{compile::ProjectCompiler, shell};
use foundry_compilers::{
    CompilationError, FileFilter, Project, ProjectCompileOutput,
    compilers::{Language, multi::MultiCompilerLanguage},
    solc::SolcLanguage,
    utils::source_files_iter,
};
use foundry_config::{
    Config, SkipBuildFilters,
    figment::{
        self, Metadata, Profile, Provider,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
    },
    filter::expand_globs,
};
use serde::Serialize;
use std::path::PathBuf;

foundry_config::merge_impl_figment_convert!(BuildArgs, build);

/// CLI arguments for `forge build`.
///
/// CLI arguments take the highest precedence in the Config/Figment hierarchy.
/// In order to override them in the foundry `Config` they need to be merged into an existing
/// `figment::Provider`, like `foundry_config::Config` is.
///
/// `BuildArgs` implements `figment::Provider` in which all config related fields are serialized and
/// then merged into an existing `Config`, effectively overwriting them.
///
/// Some arguments are marked as `#[serde(skip)]` and require manual processing in
/// `figment::Provider` implementation
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(next_help_heading = "Build options", about = None, long_about = None)] // override doc
pub struct BuildArgs {
    /// Build source files from specified paths.
    #[serde(skip)]
    pub paths: Option<Vec<PathBuf>>,

    /// Print compiled contract names.
    #[arg(long)]
    #[serde(skip)]
    pub names: bool,

    /// Print compiled contract sizes.
    /// Constructor argument length is not included in the calculation of initcode size.
    #[arg(long)]
    #[serde(skip)]
    pub sizes: bool,

    /// Ignore initcode contract bytecode size limit introduced by EIP-3860.
    #[arg(long, alias = "ignore-initcode-size")]
    #[serde(skip)]
    pub ignore_eip_3860: bool,

    #[command(flatten)]
    #[serde(flatten)]
    pub build: BuildOpts,

    #[command(flatten)]
    #[serde(skip)]
    pub watch: WatchArgs,
}

impl BuildArgs {
    pub fn run(self) -> Result<ProjectCompileOutput> {
        let mut config = self.load_config()?;

        if install::install_missing_dependencies(&mut config) && config.auto_detect_remappings {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

        let project = config.project()?;

        // Collect sources to compile if build subdirectories specified.
        let mut files = vec![];
        if let Some(paths) = &self.paths {
            for path in paths {
                let joined = project.root().join(path);
                let path = if joined.exists() { &joined } else { path };
                files.extend(source_files_iter(path, MultiCompilerLanguage::FILE_EXTENSIONS));
            }
            if files.is_empty() {
                eyre::bail!("No source files found in specified build paths.")
            }
        }

        let format_json = shell::is_json();
        let compiler = ProjectCompiler::new()
            .files(files)
            .dynamic_test_linking(config.dynamic_test_linking)
            .print_names(self.names)
            .print_sizes(self.sizes)
            .ignore_eip_3860(self.ignore_eip_3860)
            .bail(!format_json);

        let output = compiler.compile(&project)?;

        // Cache project selectors.
        cache_local_signatures(&output)?;

        if format_json && !self.names && !self.sizes {
            sh_println!("{}", serde_json::to_string_pretty(&output.output())?)?;
        }

        if !config.lint.lint_on_build {
            return Ok(output);
        }

        // Only run the `SolidityLinter` if there are no compilation errors
        if !output.output().errors.iter().any(|e| e.is_error()) {
            self.lint(&project, &config).map_err(|err| eyre!("Lint failed: {err}"))?;
        }

        Ok(output)
    }

    fn lint(&self, project: &Project, config: &Config) -> Result<()> {
        let format_json = shell::is_json();
        if project.compiler.solc.is_some() && !shell::is_quiet() {
            let linter = SolidityLinter::new(config.project_paths())
                .with_json_emitter(format_json)
                .with_description(!format_json)
                .with_severity(if config.lint.severity.is_empty() {
                    None
                } else {
                    Some(config.lint.severity.clone())
                })
                .without_lints(if config.lint.exclude_lints.is_empty() {
                    None
                } else {
                    Some(
                        config
                            .lint
                            .exclude_lints
                            .iter()
                            .filter_map(|s| forge_lint::sol::SolLint::try_from(s.as_str()).ok())
                            .collect(),
                    )
                });

            // Expand ignore globs and canonicalize from the get go
            let ignored = expand_globs(&config.root, config.lint.ignore.iter())?
                .iter()
                .flat_map(foundry_common::fs::canonicalize_path)
                .collect::<Vec<_>>();

            let skip = SkipBuildFilters::new(config.skip.clone(), config.root.clone());
            let curr_dir = std::env::current_dir()?;
            let input_files = config
                .project_paths::<SolcLanguage>()
                .input_files_iter()
                .filter(|p| {
                    skip.is_match(p)
                        && !(ignored.contains(p) || ignored.contains(&curr_dir.join(p)))
                })
                .collect::<Vec<_>>();

            if !input_files.is_empty() {
                let sess = linter.init();

                let pcx = solar_pcx_from_build_opts(
                    &sess,
                    &self.build,
                    Some(project),
                    Some(&input_files),
                )?;
                linter.early_lint(&input_files, pcx);

                let pcx = solar_pcx_from_build_opts(
                    &sess,
                    &self.build,
                    Some(project),
                    Some(&input_files),
                )?;
                linter.late_lint(&input_files, pcx);
            }
        }

        Ok(())
    }

    /// Returns the `Project` for the current workspace
    ///
    /// This loads the `foundry_config::Config` for the current workspace (see
    /// [`foundry_config::utils::find_project_root`] and merges the cli `BuildArgs` into it before
    /// returning [`foundry_config::Config::project()`]
    pub fn project(&self) -> Result<Project> {
        self.build.project()
    }

    /// Returns whether `BuildArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    /// Returns the [`watchexec::Config`] necessary to bootstrap a new watch loop.
    pub(crate) fn watchexec_config(&self) -> Result<watchexec::Config> {
        // Use the path arguments or if none where provided the `src`, `test` and `script`
        // directories as well as the `foundry.toml` configuration file.
        self.watch.watchexec_config(|| {
            let config = self.load_config()?;
            let foundry_toml: PathBuf = config.root.join(Config::FILE_NAME);
            Ok([config.src, config.test, config.script, foundry_toml])
        })
    }
}

// Make this args a `figment::Provider` so that it can be merged into the `Config`
impl Provider for BuildArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Build Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let mut dict = value.into_dict().ok_or(error)?;

        if self.names {
            dict.insert("names".to_string(), true.into());
        }

        if self.sizes {
            dict.insert("sizes".to_string(), true.into());
        }

        if self.ignore_eip_3860 {
            dict.insert("ignore_eip_3860".to_string(), true.into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
