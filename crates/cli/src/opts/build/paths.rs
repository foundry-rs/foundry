use clap::{Parser, ValueHint};
use eyre::{Result, ensure};
use foundry_compilers::{artifacts::remappings::Remapping, utils::canonicalized};
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
    ///
    /// The project root is the config file's parent directory.
    #[arg(
        long,
        conflicts_with = "root",
        value_hint = ValueHint::FilePath,
        value_name = "FILE"
    )]
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

impl ProjectPathOpts {
    /// Returns the root directory to use for configuring the project.
    ///
    /// Uses the parent of `--config-path`, then `--root`, otherwise see [`find_project_root`].
    ///
    /// # Panics
    ///
    /// Panics if `--config-path` is invalid or the project root cannot be found.
    #[track_caller]
    pub fn project_root(&self) -> PathBuf {
        if self.config_path.is_some() {
            // Preserve the existing config-path validation diagnostics.
            self.try_project_root().unwrap_or_else(|err| panic!("error: {err}"))
        } else {
            self.try_project_root().expect("could not determine project root")
        }
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

    /// Resolves the project root without loading configuration, validating `--config-path` first.
    pub fn try_project_root(&self) -> Result<PathBuf> {
        if let Some(config_path) = &self.config_path {
            ensure!(config_path.exists(), "config-path `{}` does not exist", config_path.display());
            ensure!(
                config_path.ends_with(Config::FILE_NAME),
                "the config-path must be a path to a foundry.toml file"
            );
            ensure!(config_path.is_file(), "config-path `{}` is not a file", config_path.display());
            let config_path = canonicalized(config_path);
            Ok(config_path.parent().unwrap().to_path_buf())
        } else if let Some(root) = &self.root {
            Ok(root.clone())
        } else {
            Ok(find_project_root(None)?)
        }
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
