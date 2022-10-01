//! Build command
use crate::cmd::{
    forge::{
        build::filter::{SkipBuildFilter, SkipBuildFilters},
        install::{self},
        watch::WatchArgs,
    },
    Cmd, LoadConfig,
};
use clap::{ArgAction, Parser};
use ethers::solc::{Project, ProjectCompileOutput};
use foundry_common::{compile, compile::ProjectCompiler};
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
use tracing::trace;
use watchexec::config::{InitConfig, RuntimeConfig};

mod core;
pub use self::core::CoreBuildArgs;

mod paths;
pub use self::paths::ProjectPathsArgs;

mod filter;

foundry_config::merge_impl_figment_convert!(BuildArgs, args);

/// All `forge build` related arguments
///
/// CLI arguments take the highest precedence in the Config/Figment hierarchy.
/// In order to override them in the foundry `Config` they need to be merged into an existing
/// `figment::Provider`, like `foundry_config::Config` is.
///
/// # Example
///
/// ```
/// use foundry_cli::cmd::forge::build::BuildArgs;
/// use foundry_config::Config;
/// # fn t(args: BuildArgs) {
/// let config = Config::from(&args);
/// # }
/// ```
///
/// `BuildArgs` implements `figment::Provider` in which all config related fields are serialized and
/// then merged into an existing `Config`, effectively overwriting them.
///
/// Some arguments are marked as `#[serde(skip)]` and require manual processing in
/// `figment::Provider` implementation
#[derive(Debug, Clone, Parser, Serialize, Default)]
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

    #[clap(
        long,
        num_args(1..),
        action = ArgAction::Append,
        help = "Skip building whose names contain FILTER. `test` and `script` are aliases for `.t.sol` and `.s.sol`. (this flag can be used multiple times)")]
    #[serde(skip)]
    pub skip: Option<Vec<SkipBuildFilter>>,

    #[clap(flatten, next_help_heading = "WATCH OPTIONS")]
    #[serde(skip)]
    pub watch: WatchArgs,
}

impl Cmd for BuildArgs {
    type Output = ProjectCompileOutput;
    fn run(self) -> eyre::Result<Self::Output> {
        let mut config = self.try_load_config_emit_warnings()?;
        let mut project = config.project()?;

        if install::install_missing_dependencies(&mut config, &project, self.args.silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
            project = config.project()?;
        }

        let filters = self.skip.unwrap_or_default();

        if self.args.silent {
            if filters.is_empty() {
                compile::suppress_compile(&project)
            } else {
                trace!(?filters, "compile with filters suppressed");
                compile::suppress_compile_sparse(&project, SkipBuildFilters(filters))
            }
        } else {
            let compiler = ProjectCompiler::new(self.names, self.sizes);
            if filters.is_empty() {
                compiler.compile(&project)
            } else {
                trace!(?filters, "compile with filters");
                compiler.compile_sparse(&project, SkipBuildFilters(filters))
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_build_filters() {
        let args: BuildArgs = BuildArgs::parse_from(["foundry-cli", "--skip", "tests"]);
        assert_eq!(args.skip, Some(vec![SkipBuildFilter::Tests]));

        let args: BuildArgs = BuildArgs::parse_from(["foundry-cli", "--skip", "scripts"]);
        assert_eq!(args.skip, Some(vec![SkipBuildFilter::Scripts]));

        let args: BuildArgs =
            BuildArgs::parse_from(["foundry-cli", "--skip", "tests", "--skip", "scripts"]);
        assert_eq!(args.skip, Some(vec![SkipBuildFilter::Tests, SkipBuildFilter::Scripts]));

        let args: BuildArgs = BuildArgs::parse_from(["foundry-cli", "--skip", "tests", "scripts"]);
        assert_eq!(args.skip, Some(vec![SkipBuildFilter::Tests, SkipBuildFilter::Scripts]));
    }
}
