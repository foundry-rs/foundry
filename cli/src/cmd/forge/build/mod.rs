//! Build command
use crate::{
    cmd::{
        forge::{
            install::{self, DependencyInstallOpts},
            watch::WatchArgs,
        },
        Cmd, LoadConfig,
    },
    compile,
    utils::p_println,
};
use clap::Parser;
use ethers::solc::{Project, ProjectCompileOutput};
use eyre::WrapErr;
use foundry_config::{
    figment::{
        self,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Metadata, Profile, Provider,
    },
    Config,
};
use serde::Serialize;
use watchexec::config::{InitConfig, RuntimeConfig};

mod core;
pub use self::core::CoreBuildArgs;

mod paths;
pub use self::paths::ProjectPathsArgs;

// All `forge build` related arguments
//
// CLI arguments take the highest precedence in the Config/Figment hierarchy.
// In order to override them in the foundry `Config` they need to be merged into an existing
// `figment::Provider`, like `foundry_config::Config` is.
//
// # Example
//
// ```ignore
// use foundry_config::Config;
// # fn t(args: BuildArgs) {
// let config = Config::from(&args);
// # }
// ```
//
// `BuildArgs` implements `figment::Provider` in which all config related fields are serialized and
// then merged into an existing `Config`, effectively overwriting them.
//
// Some arguments are marked as `#[serde(skip)]` and require manual processing in
// `figment::Provider` implementation
#[derive(Debug, Clone, Parser, Serialize)]
pub struct BuildArgs {
    #[clap(flatten)]
    #[serde(flatten)]
    pub args: CoreBuildArgs,

    #[clap(help = "Print compiled contract names.", long = "names")]
    #[serde(skip)]
    pub names: bool,

    #[clap(help = "Print compiled contract sizes.", long = "sizes")]
    #[serde(skip)]
    pub sizes: bool,

    #[clap(flatten, next_help_heading = "WATCH OPTIONS")]
    #[serde(skip)]
    pub watch: WatchArgs,
}

impl Cmd for BuildArgs {
    type Output = ProjectCompileOutput;
    fn run(self) -> eyre::Result<Self::Output> {
        let mut config = self.load_config_emit_warnings();
        let project = config.project()?;

        // try to auto install missing submodules in the default install dir but only if git is
        // installed
        if which::which("git").is_ok() &&
            install::has_missing_dependencies(&project.root(), &config.install_lib_dir())
        {
            // The extra newline is needed, otherwise the compiler output will overwrite the
            // message
            p_println!(!self.args.silent => "Missing dependencies found. Installing now.\n");
            install::install(
                &mut config,
                Vec::new(),
                DependencyInstallOpts {
                    // TODO(onbjerg): We should settle on --quiet or --silent.
                    quiet: self.args.silent,
                    ..Default::default()
                },
            ).wrap_err("Your project has missing dependencies that could not be installed. Please ensure git is installed and available.")?
        }

        if self.args.silent {
            compile::suppress_compile(&project)
        } else {
            compile::compile(&project, self.names, self.sizes)
        }
    }
}

impl BuildArgs {
    /// Returns the `Project` for the current workspace
    ///
    /// This loads the `foundry_config::Config` for the current workspace (see
    /// [`utils::find_project_root_path`] and merges the cli `BuildArgs` into it before returning
    /// [`foundry_config::Config::project()`]
    pub fn project(&self) -> eyre::Result<Project> {
        self.args.project()
    }

    /// Returns whether `BuildArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    /// Returns the [`watchexec::InitConfig`] and [`watchexec::RuntimeConfig`] necessary to
    /// bootstrap a new [`watchexe::Watchexec`] loop.
    pub(crate) fn watchexec_config(&self) -> eyre::Result<(InitConfig, RuntimeConfig)> {
        // use the path arguments or if none where provided the `src` dir
        self.watch.watchexec_config(|| {
            let config = Config::from(self);
            vec![config.src, config.test, config.script]
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

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

foundry_config::impl_figment_convert!(BuildArgs, args);
