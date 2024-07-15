use super::{install, watch::WatchArgs};
use clap::Parser;
use eyre::Result;
use foundry_cli::{opts::CoreBuildArgs, utils::LoadConfig};
use foundry_common::compile::ProjectCompiler;
use foundry_compilers::{
    compilers::{multi::MultiCompilerLanguage, Language},
    utils::source_files_iter,
    Project, ProjectCompileOutput,
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
use std::path::PathBuf;

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
    #[arg(long)]
    #[serde(skip)]
    pub sizes: bool,

    #[command(flatten)]
    #[serde(flatten)]
    pub args: CoreBuildArgs,

    #[command(flatten)]
    #[serde(skip)]
    pub watch: WatchArgs,

    /// Output the compilation errors in the json format.
    /// This is useful when you want to use the output in other tools.
    #[arg(long, conflicts_with = "silent")]
    #[serde(skip)]
    pub format_json: bool,
}

impl BuildArgs {
    pub fn run(self) -> Result<ProjectCompileOutput> {
        let mut config = self.try_load_config_emit_warnings()?;

        if install::install_missing_dependencies(&mut config, self.args.silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
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
        }

        let compiler = ProjectCompiler::new()
            .files(files)
            .print_names(self.names)
            .print_sizes(self.sizes)
            .quiet(self.format_json)
            .bail(!self.format_json);

        let output = compiler.compile(&project)?;

        if self.format_json {
            println!("{}", serde_json::to_string_pretty(&output.output())?);
        }

        Ok(output)
    }

    /// Returns the `Project` for the current workspace
    ///
    /// This loads the `foundry_config::Config` for the current workspace (see
    /// [`utils::find_project_root_path`] and merges the cli `BuildArgs` into it before returning
    /// [`foundry_config::Config::project()`]
    pub fn project(&self) -> Result<Project> {
        self.args.project()
    }

    /// Returns whether `BuildArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    /// Returns the [`watchexec::InitConfig`] and [`watchexec::RuntimeConfig`] necessary to
    /// bootstrap a new [`watchexe::Watchexec`] loop.
    pub(crate) fn watchexec_config(&self) -> Result<watchexec::Config> {
        // use the path arguments or if none where provided the `src` dir
        self.watch.watchexec_config(|| {
            let config = Config::from(self);
            [config.src, config.test, config.script]
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
    use foundry_config::filter::SkipBuildFilter;

    #[test]
    fn can_parse_build_filters() {
        let args: BuildArgs = BuildArgs::parse_from(["foundry-cli", "--skip", "tests"]);
        assert_eq!(args.args.skip, Some(vec![SkipBuildFilter::Tests]));

        let args: BuildArgs = BuildArgs::parse_from(["foundry-cli", "--skip", "scripts"]);
        assert_eq!(args.args.skip, Some(vec![SkipBuildFilter::Scripts]));

        let args: BuildArgs =
            BuildArgs::parse_from(["foundry-cli", "--skip", "tests", "--skip", "scripts"]);
        assert_eq!(args.args.skip, Some(vec![SkipBuildFilter::Tests, SkipBuildFilter::Scripts]));

        let args: BuildArgs = BuildArgs::parse_from(["foundry-cli", "--skip", "tests", "scripts"]);
        assert_eq!(args.args.skip, Some(vec![SkipBuildFilter::Tests, SkipBuildFilter::Scripts]));
    }

    #[test]
    fn check_conflicts() {
        let args: std::result::Result<BuildArgs, clap::Error> =
            BuildArgs::try_parse_from(["foundry-cli", "--format-json", "--silent"]);
        assert!(args.is_err());
        assert!(args.unwrap_err().kind() == clap::error::ErrorKind::ArgumentConflict);
    }
}
