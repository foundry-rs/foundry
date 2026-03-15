use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_compilers::artifacts::remappings::Remapping;
use foundry_config::{
    Config, figment,
    figment::{
        Metadata, Profile, Provider,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
    },
    find_project_root, remappings_from_env_var,
};
use serde::Serialize;
use std::path::PathBuf;

/// Common arguments for a project's paths.
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(next_help_heading = "Project options")]
pub struct ProjectPathOpts {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    #[serde(skip)]
    pub root: Option<PathBuf>,

    /// The contracts source directory.
    #[arg(long, short = 'C', value_hint = ValueHint::DirPath, value_name = "PATH")]
    #[serde(rename = "src", skip_serializing_if = "Option::is_none")]
    pub contracts: Option<PathBuf>,

    /// The project's remappings.
    #[arg(long, short = 'R')]
    #[serde(skip)]
    pub remappings: Vec<Remapping>,

    /// The project's remappings from the environment.
    #[arg(long, value_name = "ENV")]
    #[serde(skip)]
    pub remappings_env: Option<String>,

    /// The path to the compiler cache.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_path: Option<PathBuf>,

    /// The path to the library folder.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    #[serde(rename = "libs", skip_serializing_if = "Vec::is_empty")]
    pub lib_paths: Vec<PathBuf>,

    /// Use the Hardhat-style project layout.
    ///
    /// This is the same as using: `--contracts contracts --lib-paths node_modules`.
    #[arg(long, conflicts_with = "contracts", visible_alias = "hh")]
    #[serde(skip)]
    pub hardhat: bool,

    /// Path to the config file.
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "FILE"
    )]
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

impl ProjectPathOpts {
    /// Returns the root directory to use for configuring the project.
    ///
    /// This will be the `--root` argument if provided, otherwise see [`find_project_root`].
    ///
    /// # Panics
    ///
    /// Panics if the project root directory cannot be found. See [`find_project_root`].
    #[track_caller]
    pub fn project_root(&self) -> PathBuf {
        self.root
            .clone()
            .unwrap_or_else(|| find_project_root(None).expect("could not determine project root"))
    }

    /// Returns the remappings to add to the config
    pub fn get_remappings(&self) -> Vec<Remapping> {
        let mut remappings = self.remappings.clone();
        if let Some(remappings_env) = self.remappings_env.as_deref()
            && let Some(env_remappings) = remappings_from_env_var(remappings_env)
        {
            match env_remappings {
                Ok(env_remappings) => remappings.extend(env_remappings),
                Err(err) => {
                    let _ = sh_warn!(
                        "failed to parse env var remappings from `{remappings_env}`: {err}"
                    );
                }
            }
        }
        remappings
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectPathOpts;

    #[test]
    fn get_remappings_ignores_invalid_remappings_env_var() {
        let env_name = "FOUNDRY_CLI_TEST_INVALID_REMAPPINGS";
        unsafe {
            std::env::set_var(env_name, "this-is-not-a-remapping");
        }

        let opts =
            ProjectPathOpts { remappings_env: Some(env_name.to_string()), ..Default::default() };
        let remappings = opts.get_remappings();
        assert!(remappings.is_empty());

        unsafe {
            std::env::remove_var(env_name);
        }
    }

    #[test]
    fn get_remappings_parses_valid_remappings_env_var() {
        let env_name = "FOUNDRY_CLI_TEST_VALID_REMAPPINGS";
        unsafe {
            std::env::set_var(env_name, "forge-std/=lib/forge-std/src/");
        }

        let opts =
            ProjectPathOpts { remappings_env: Some(env_name.to_string()), ..Default::default() };
        let remappings = opts.get_remappings();
        assert_eq!(remappings.len(), 1);
        assert_eq!(remappings[0].name, "forge-std/");
        assert_eq!(remappings[0].path, "lib/forge-std/src/");

        unsafe {
            std::env::remove_var(env_name);
        }
    }
}

foundry_config::impl_figment_convert!(ProjectPathOpts);

// Make this args a `figment::Provider` so that it can be merged into the `Config`
impl Provider for ProjectPathOpts {
    fn metadata(&self) -> Metadata {
        Metadata::named("Project Paths Args Provider")
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

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
