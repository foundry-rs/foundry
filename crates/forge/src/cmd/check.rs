use super::{install, watch::WatchArgs};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{BuildOpts, configure_pcx_from_solc},
    utils::LoadConfig,
};
use foundry_common::shell;
use foundry_compilers::{
    CompilerInput, Graph, Language, Project,
    artifacts::{Source, Sources},
    compilers::multi::MultiCompilerLanguage,
    multi::MultiCompilerParser,
    solc::{SolcLanguage, SolcVersionedInput},
    utils::source_files_iter,
};
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile, Provider,
        error::Kind::InvalidType,
        value::{Map, Value},
    },
};
use serde::Serialize;
use solar::sema::Compiler;
use std::path::PathBuf;

foundry_config::merge_impl_figment_convert!(CheckArgs, build);

/// CLI arguments for `forge check`.
///
/// Similar to `forge build`, but only performs parsing, lowering and semantic analysis.
/// Skips codegen and optimizer steps for faster feedback.
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(next_help_heading = "Check options", about = None, long_about = None)]
pub struct CheckArgs {
    /// Check source files from specified paths.
    #[serde(skip)]
    pub paths: Option<Vec<PathBuf>>,

    #[command(flatten)]
    #[serde(flatten)]
    pub build: BuildOpts,

    #[command(flatten)]
    #[serde(skip)]
    pub watch: WatchArgs,
}

impl CheckArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;

        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

        let project = config.project()?;

        // Collect sources to check if subdirectories specified.
        let mut files = vec![];
        if let Some(paths) = &self.paths {
            for path in paths {
                let joined = project.root().join(path);
                let path = if joined.exists() { &joined } else { path };
                files.extend(source_files_iter(path, MultiCompilerLanguage::FILE_EXTENSIONS));
            }
            if files.is_empty() {
                eyre::bail!("No source files found in specified check paths.")
            }
        }

        // Run check using Solar for Solidity files
        self.check_solidity(&project, &config, files.as_slice())?;

        if !shell::is_json() {
            sh_println!("Check completed successfully")?;
        }

        Ok(())
    }

    fn check_solidity(
        &self,
        project: &Project,
        config: &Config,
        target_files: &[PathBuf],
    ) -> Result<()> {
        let sources = if target_files.is_empty() {
            project.paths.read_input_files()?
        } else {
            let mut sources = Sources::new();
            for path in target_files {
                let canonical = dunce::canonicalize(path)?;
                let source = Source::read(&canonical)?;
                sources.insert(canonical, source);
            }
            sources
        };

        let sources_by_version =
            Graph::<MultiCompilerParser>::resolve_sources(&project.paths, sources)?
                .into_sources_by_version(project)?;

        for (lang, versions) in sources_by_version.sources {
            // Only check Solidity sources
            if lang != MultiCompilerLanguage::Solc(SolcLanguage::Solidity) {
                continue;
            }

            for (version, sources, _) in versions {
                let vinput = SolcVersionedInput::build(
                    sources,
                    config.solc_settings()?,
                    SolcLanguage::Solidity,
                    version.clone(),
                );

                let mut sess = solar::interface::Session::builder().with_stderr_emitter().build();
                sess.dcx.set_flags_mut(|flags| flags.track_diagnostics = false);

                let mut compiler = Compiler::new(sess);

                let result = compiler.enter_mut(|compiler| -> Result<()> {
                    let mut pcx = compiler.parse();
                    configure_pcx_from_solc(&mut pcx, &project.paths, &vinput, true);
                    pcx.parse();

                    let _ = compiler.lower_asts();

                    Ok(())
                });

                if compiler.sess().dcx.has_errors().is_err() {
                    eyre::bail!("Check failed for Solidity version {}", version);
                }

                result?;
            }
        }

        Ok(())
    }

    /// Returns the `Project` for the current workspace
    pub fn project(&self) -> Result<Project> {
        self.build.project()
    }

    /// Returns whether `CheckArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    /// Returns the [`watchexec::Config`] necessary to bootstrap a new watch loop.
    pub(crate) fn watchexec_config(&self) -> Result<watchexec::Config> {
        self.watch.watchexec_config(|| {
            let config = self.load_config()?;
            let foundry_toml: PathBuf = config.root.join(Config::FILE_NAME);
            Ok([config.src, config.test, config.script, foundry_toml])
        })
    }
}

// Make this args a `figment::Provider` so that it can be merged into the `Config`
impl Provider for CheckArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Check Args Provider")
    }

    fn data(&self) -> Result<figment::value::Map<Profile, figment::value::Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let dict = value.into_dict().ok_or(error)?;

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
