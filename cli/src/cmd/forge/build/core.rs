use crate::{cmd::forge::build::ProjectPathsArgs, opts::forge::CompilerArgs};
use clap::{Parser, ValueHint};
use ethers::solc::{
    artifacts::RevertStrings, remappings::Remapping, utils::canonicalized, Project,
};
use foundry_config::{
    figment,
    figment::{
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Figment, Metadata, Profile, Provider,
    },
    Config,
};
use serde::Serialize;
use std::path::PathBuf;

/// Various arguments used by multiple subcommands
#[derive(Debug, Clone, Parser, Serialize, Default)]
pub struct CoreBuildArgs {
    #[clap(
        help_heading = "CACHE OPTIONS",
        help = "Clear the cache and artifacts folder and recompile.",
        long
    )]
    #[serde(skip)]
    pub force: bool,

    #[clap(
        help_heading = "LINKER OPTIONS",
        help = "Set pre-linked libraries.",
        long,
        env = "DAPP_LIBRARIES",
        value_name = "LIBRARIES"
    )]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,

    #[clap(flatten, next_help_heading = "COMPILER OPTIONS")]
    #[serde(flatten)]
    pub compiler: CompilerArgs,

    #[clap(
        help_heading = "COMPILER OPTIONS",
        help = "Ignore solc warnings by error code.",
        long,
        value_name = "ERROR_CODES"
    )]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_error_codes: Vec<u64>,

    #[clap(help_heading = "COMPILER OPTIONS", help = "Do not auto-detect solc.", long)]
    #[serde(skip)]
    pub no_auto_detect: bool,

    /// Specify the solc version, or a path to a local solc, to build with.
    ///
    /// Valid values are in the format `x.y.z`, `solc:x.y.z` or `path/to/solc`.
    #[clap(help_heading = "COMPILER OPTIONS", value_name = "SOLC_VERSION", long = "use")]
    #[serde(skip)]
    pub use_solc: Option<String>,

    #[clap(
        help_heading = "COMPILER OPTIONS",
        help = "Do not access the network.",
        long_help = "Do not access the network. Missing solc versions will not be installed.",
        long
    )]
    #[serde(skip)]
    pub offline: bool,

    #[clap(
        help_heading = "COMPILER OPTIONS",
        help = "Use the Yul intermediate representation compilation pipeline.",
        long
    )]
    #[serde(skip)]
    pub via_ir: bool,

    #[clap(flatten, next_help_heading = "PROJECT OPTIONS")]
    #[serde(flatten)]
    pub project_paths: ProjectPathsArgs,

    #[clap(
        help_heading = "PROJECT OPTIONS",
        help = "The path to the contract artifacts folder.",
        long = "out",
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    #[serde(rename = "out", skip_serializing_if = "Option::is_none")]
    pub out_path: Option<PathBuf>,

    #[clap(
        help_heading = "PROJECT OPTIONS",
        help = r#"Revert string configuration. Possible values are "default", "strip" (remove), "debug" (Solidity-generated revert strings) and "verboseDebug""#,
        long = "revert-strings",
        value_name = "REVERT"
    )]
    #[serde(skip)]
    pub revert_strings: Option<RevertStrings>,

    #[clap(help_heading = "COMPILER OPTIONS", long, help = "Don't print anything on startup.")]
    #[serde(skip)]
    pub silent: bool,

    #[clap(help_heading = "PROJECT OPTIONS", help = "Generate build info files.", long)]
    #[serde(skip)]
    pub build_info: bool,
}

impl CoreBuildArgs {
    /// Returns the `Project` for the current workspace
    ///
    /// This loads the `foundry_config::Config` for the current workspace (see
    /// [`utils::find_project_root_path`] and merges the cli `BuildArgs` into it before returning
    /// [`foundry_config::Config::project()`]
    pub fn project(&self) -> eyre::Result<Project> {
        let config: Config = self.into();
        Ok(config.project()?)
    }

    /// Returns the remappings to add to the config
    #[deprecated(note = "Use ProjectPathsArgs::get_remappings() instead")]
    pub fn get_remappings(&self) -> Vec<Remapping> {
        self.project_paths.get_remappings()
    }
}

// Loads project's figment and merges the build cli arguments into it
impl<'a> From<&'a CoreBuildArgs> for Figment {
    fn from(args: &'a CoreBuildArgs) -> Self {
        let figment = if let Some(ref config_path) = args.project_paths.config_path {
            if !config_path.exists() {
                panic!("error: config-path `{}` does not exist", config_path.display())
            }
            if !config_path.ends_with(Config::FILE_NAME) {
                panic!("error: the config-path must be a path to a foundry.toml file")
            }
            let config_path = canonicalized(config_path);
            Config::figment_with_root(config_path.parent().unwrap())
        } else {
            Config::figment_with_root(args.project_paths.project_root())
        };

        // remappings should stack
        let mut remappings = args.project_paths.get_remappings();
        remappings
            .extend(figment.extract_inner::<Vec<Remapping>>("remappings").unwrap_or_default());
        remappings.sort_by(|a, b| a.name.cmp(&b.name));
        remappings.dedup_by(|a, b| a.name.eq(&b.name));
        figment.merge(("remappings", remappings)).merge(args)
    }
}

impl<'a> From<&'a CoreBuildArgs> for Config {
    fn from(args: &'a CoreBuildArgs) -> Self {
        let figment: Figment = args.into();
        let mut config = Config::from_provider(figment).sanitized();
        // if `--config-path` is set we need to adjust the config's root path to the actual root
        // path for the project, otherwise it will the parent dir of the `--config-path`
        if args.project_paths.config_path.is_some() {
            config.__root = args.project_paths.project_root().into();
        }
        config
    }
}

impl Provider for CoreBuildArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Core Build Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let mut dict = value.into_dict().ok_or(error)?;

        if self.no_auto_detect {
            dict.insert("auto_detect_solc".to_string(), false.into());
        }

        if let Some(ref solc) = self.use_solc {
            dict.insert("solc".to_string(), solc.trim_start_matches("solc:").into());
        }

        if self.offline {
            dict.insert("offline".to_string(), true.into());
        }

        if self.via_ir {
            dict.insert("via_ir".to_string(), true.into());
        }

        if self.force {
            dict.insert("force".to_string(), self.force.into());
        }

        if self.build_info {
            dict.insert("build_info".to_string(), self.build_info.into());
        }

        if self.compiler.optimize {
            dict.insert("optimizer".to_string(), self.compiler.optimize.into());
        }

        if !self.compiler.extra_output.is_empty() {
            let selection: Vec<_> =
                self.compiler.extra_output.iter().map(|s| s.to_string()).collect();
            dict.insert("extra_output".to_string(), selection.into());
        }

        if !self.compiler.extra_output_files.is_empty() {
            let selection: Vec<_> =
                self.compiler.extra_output_files.iter().map(|s| s.to_string()).collect();
            dict.insert("extra_output_files".to_string(), selection.into());
        }

        if let Some(ref revert) = self.revert_strings {
            dict.insert("revert_strings".to_string(), revert.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
