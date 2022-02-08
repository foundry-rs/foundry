//! build command

use ethers::solc::{MinimalCombinedArtifacts, Project, ProjectCompileOutput};
use std::path::PathBuf;

use crate::{cmd::Cmd, opts::forge::CompilerArgs};

use clap::{Parser, ValueHint};
use ethers::solc::remappings::Remapping;
use foundry_config::{
    figment::{
        self,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Figment, Metadata, Profile, Provider,
    },
    find_project_root_path, remappings_from_env_var, Config,
};
use serde::Serialize;

// Loads project's figment and merges the build cli arguments into it
impl<'a> From<&'a BuildArgs> for Figment {
    fn from(args: &'a BuildArgs) -> Self {
        let figment = if let Some(root) = args.root.clone() {
            Config::figment_with_root(root)
        } else {
            Config::figment_with_root(find_project_root_path().unwrap())
        };

        // remappings should stack
        let mut remappings = args.get_remappings();
        remappings
            .extend(figment.extract_inner::<Vec<Remapping>>("remappings").unwrap_or_default());
        remappings.sort_by(|a, b| a.name.cmp(&b.name));
        remappings.dedup_by(|a, b| a.name.eq(&b.name));
        figment.merge(("remappings", remappings)).merge(args)
    }
}

impl<'a> From<&'a BuildArgs> for Config {
    fn from(args: &'a BuildArgs) -> Self {
        let figment: Figment = args.into();
        Config::from_provider(figment).sanitized()
    }
}

/// All `forge build` related arguments
///
/// CLI arguments take the highest precedence in the Config/Figment hierarchy.
/// In order to override them in the foundry `Config` they need to be merged into an existing
/// `figment::Provider`, like `foundry_config::Config` is.
///
/// # Example
///
/// ```ignore
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
#[derive(Debug, Clone, Parser, Serialize)]
pub struct BuildArgs {
    #[clap(
        help = "the project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
        long,
        value_hint = ValueHint::DirPath
    )]
    #[serde(skip)]
    pub root: Option<PathBuf>,

    #[clap(
        env = "DAPP_SRC",
        help = "the directory relative to the root under which the smart contracts are",
        long,
        short,
        value_hint = ValueHint::DirPath
    )]
    #[serde(rename = "src", skip_serializing_if = "Option::is_none")]
    pub contracts: Option<PathBuf>,

    #[clap(help = "the remappings", long, short)]
    #[serde(skip)]
    pub remappings: Vec<ethers::solc::remappings::Remapping>,

    #[clap(help = "the env var that holds remappings", long = "remappings-env")]
    #[serde(skip)]
    pub remappings_env: Option<String>,

    #[clap(
        help = "the paths where your libraries are installed",
        long,
        value_hint = ValueHint::DirPath
    )]
    #[serde(rename = "libs", skip_serializing_if = "Vec::is_empty")]
    pub lib_paths: Vec<PathBuf>,

    #[clap(
        help = "path to where the contract artifacts are stored",
        long = "out",
        short,
        value_hint = ValueHint::DirPath
    )]
    #[serde(rename = "out", skip_serializing_if = "Option::is_none")]
    pub out_path: Option<PathBuf>,

    #[clap(flatten)]
    #[serde(flatten)]
    pub compiler: CompilerArgs,

    #[clap(help = "ignore warnings with specific error codes", long)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_error_codes: Vec<u64>,

    #[clap(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    #[serde(skip)]
    pub no_auto_detect: bool,

    #[clap(
        help = "force recompilation of the project, deletes the cache and artifacts folders",
        long
    )]
    #[serde(skip)]
    pub force: bool,

    #[clap(
        help = "uses hardhat style project layout. This a convenience flag and is the same as `--contracts contracts --lib-paths node_modules`",
        long,
        conflicts_with = "contracts",
        alias = "hh"
    )]
    #[serde(skip)]
    pub hardhat: bool,

    #[clap(help = "add linked libraries", long, env = "DAPP_LIBRARIES")]
    pub libraries: Vec<String>,
}

impl Cmd for BuildArgs {
    type Output = ProjectCompileOutput<MinimalCombinedArtifacts>;
    fn run(self) -> eyre::Result<Self::Output> {
        let project = self.project()?;
        super::compile(&project)
    }
}

impl BuildArgs {
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
    pub fn get_remappings(&self) -> Vec<Remapping> {
        let mut remappings = self.remappings.clone();
        if let Some(env_remappings) =
            self.remappings_env.as_ref().and_then(|env| remappings_from_env_var(env))
        {
            remappings.extend(env_remappings.expect("Failed to parse env var remappings"));
        }
        remappings
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

        let mut libs =
            self.lib_paths.iter().map(|p| format!("{}", p.display())).collect::<Vec<_>>();
        if self.hardhat {
            dict.insert("src".to_string(), "contracts".to_string().into());
            libs.push("node_modules".to_string());
        }

        if !libs.is_empty() {
            dict.insert("libs".to_string(), libs.into());
        }

        #[cfg(not(target_arch = "aarch64"))]
        if self.no_auto_detect {
            dict.insert("auto_detect_solc".to_string(), false.into());
        }

        #[cfg(target_arch = "aarch64")]
        {
            if !self.no_auto_detect {
                println!("Solidity compiler autodetection is disabled on ARM.");
                println!("You can track progress on ARM support in https://github.com/gakonst/foundry/issues/525");
                println!("To silence this warning use --no-auto-detect");
            }
            dict.insert("auto_detect_solc".to_string(), false.into());
        }

        if self.force {
            dict.insert("force".to_string(), self.force.into());
        }

        if self.compiler.optimize {
            dict.insert("optimizer".to_string(), self.compiler.optimize.into());
        }

        if let Some(extra) = &self.compiler.extra_output {
            dict.insert("extra_output".to_string(), extra.clone().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
