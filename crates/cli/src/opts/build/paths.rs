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
use std::{ffi::OsStr, path::PathBuf};

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
        value_name = "FILE",
        value_parser = parse_config_path
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
        if let Some(env_remappings) =
            self.remappings_env.as_ref().and_then(|env| remappings_from_env_var(env))
        {
            remappings.extend(env_remappings.expect("Failed to parse env var remappings"));
        }
        remappings
    }
}

/// Parses and validates `--config-path`.
fn parse_config_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err(format!("config-path `{}` does not exist", path.display()));
    }
    if path.file_name() != Some(OsStr::new(Config::FILE_NAME)) {
        return Err("the config-path must be a path to a foundry.toml file".to_string());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::parse_config_path;
    use foundry_config::Config;
    use std::path::PathBuf;

    #[test]
    fn parse_config_path_rejects_nonexistent_path() {
        let path = PathBuf::from("/definitely/nonexistent/path/foundry.toml");
        let err = parse_config_path(path.to_str().expect("utf8 path")).unwrap_err();
        assert!(err.contains("does not exist"), "unexpected error: {err}");
    }

    #[test]
    fn parse_config_path_rejects_non_foundry_toml_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().with_file_name("not-foundry.toml");
        std::fs::write(&path, "").unwrap();

        let err = parse_config_path(path.to_str().expect("utf8 path")).unwrap_err();
        assert!(err.contains(Config::FILE_NAME), "error should mention required file name: {err}");
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
