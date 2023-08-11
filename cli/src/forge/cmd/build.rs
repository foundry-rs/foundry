use super::{install, watch::WatchArgs};
use clap::Parser;
use ethers::solc::{Project, ProjectCompileOutput};
use foundry_cli::{
    opts::CoreBuildArgs,
    utils::{Cmd, LoadConfig},
};
use foundry_common::{
    compile,
    compile::{ProjectCompiler, SkipBuildFilter},
};
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

foundry_config::merge_impl_figment_convert!(BuildArgs, args);

/// CLI arguments for `forge build`.
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
#[clap(next_help_heading = "Build options", about = None, long_about = None)] // override doc
pub struct BuildArgs {
    /// Print compiled contract names.
    #[clap(long)]
    #[serde(skip)]
    pub names: bool,

    /// Print compiled contract sizes.
    #[clap(long)]
    #[serde(skip)]
    pub sizes: bool,

    /// Skip building files whose names contain the given filter.
    ///
    /// `test` and `script` are aliases for `.t.sol` and `.s.sol`.
    #[clap(long, num_args(1..))]
    #[serde(skip)]
    pub skip: Option<Vec<SkipBuildFilter>>,

    #[clap(flatten)]
    #[serde(flatten)]
    pub args: CoreBuildArgs,

    #[clap(flatten)]
    #[serde(skip)]
    pub watch: WatchArgs,
}

impl Cmd for BuildArgs {
    type Output = ProjectCompileOutput;

    fn run(self) -> eyre::Result<Self::Output> {
        let mut config = self.try_load_config_emit_warnings()?;
        let mut project = config.project()?;

        if install::install_missing_dependencies(&mut config, self.args.silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
            project = config.project()?;
        }

        let filters = self.skip.unwrap_or_default();

        if self.args.silent {
            compile::suppress_compile_with_filter(&project, filters)
        } else {
            let compiler = ProjectCompiler::with_filter(self.names, self.sizes, filters);
            compiler.compile(&project)
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
