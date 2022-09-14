//! Utility functions

use crate::Config;
use ethers_core::types::{serde_helpers::Numeric, U256};
use ethers_solc::remappings::{Remapping, RemappingError};
use figment::value::Value;
use serde::{Deserialize, Deserializer};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

/// Loads the config for the current project workspace
pub fn load_config() -> Config {
    load_config_with_root(None)
}

/// Loads the config for the current project workspace or the provided root path
pub fn load_config_with_root(root: Option<PathBuf>) -> Config {
    if let Some(root) = root {
        Config::load_with_root(root)
    } else {
        Config::load_with_root(find_project_root_path().unwrap())
    }
    .sanitized()
}

/// Returns the path of the top-level directory of the working git tree. If there is no working
/// tree, an error is returned.
pub fn find_git_root_path(relative_to: impl AsRef<Path>) -> eyre::Result<PathBuf> {
    let path = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(relative_to.as_ref())
        .output()?
        .stdout;
    let path = std::str::from_utf8(&path)?.trim_end_matches('\n');
    Ok(PathBuf::from(path))
}

/// Returns the root path to set for the project root
///
/// traverse the dir tree up and look for a `foundry.toml` file starting at the cwd, but only until
/// the root dir of the current repo so that
///
/// ```text
/// -- foundry.toml
///
/// -- repo
///   |__ .git
///   |__sub
///      |__ cwd
/// ```
/// will still detect `repo` as root
pub fn find_project_root_path() -> std::io::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let boundary = find_git_root_path(&cwd)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| cwd.clone());
    let mut cwd = cwd.as_path();
    // traverse as long as we're in the current git repo cwd
    while cwd.starts_with(&boundary) {
        let file_path = cwd.join(Config::FILE_NAME);
        if file_path.is_file() {
            return Ok(cwd.to_path_buf())
        }
        if let Some(parent) = cwd.parent() {
            cwd = parent;
        } else {
            break
        }
    }
    // no foundry.toml found
    Ok(boundary)
}

/// Returns all [`Remapping`]s contained in the `remappings` str separated by newlines
///
/// # Example
///
/// ```
/// use foundry_config::remappings_from_newline;
/// let remappings: Result<Vec<_>, _> = remappings_from_newline(
///     r#"
///              file-ds-test/=lib/ds-test/
///              file-other/=lib/other/
///          "#,
/// )
/// .collect();
/// ```
pub fn remappings_from_newline(
    remappings: &str,
) -> impl Iterator<Item = Result<Remapping, RemappingError>> + '_ {
    remappings.lines().map(|x| x.trim()).filter(|x| !x.is_empty()).map(Remapping::from_str)
}

/// Returns the remappings from the given var
///
/// Returns `None` if the env var is not set, otherwise all Remappings, See
/// `remappings_from_newline`
pub fn remappings_from_env_var(env_var: &str) -> Option<Result<Vec<Remapping>, RemappingError>> {
    let val = std::env::var(env_var).ok()?;
    Some(remappings_from_newline(&val).collect())
}

/// Converts the `val` into a `figment::Value::Array`
///
/// The values should be separated by commas, surrounding brackets are also supported `[a,b,c]`
pub fn to_array_value(val: &str) -> Result<Value, figment::Error> {
    let value: Value = match Value::from(val) {
        Value::String(_, val) => val
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into(),
        Value::Empty(_, _) => Vec::<Value>::new().into(),
        val @ Value::Array(_, _) => val,
        _ => return Err(format!("Invalid value `{val}`, expected an array").into()),
    };
    Ok(value)
}

/// Returns a list of _unique_ paths to all folders under `root` that contain a `foundry.toml` file
///
/// This will also resolve symlinks
///
/// # Example
///
/// ```no_run
/// use foundry_config::utils;
/// let dirs = utils::foundry_toml_dirs("./lib");
/// ```
///
/// for following layout this will return
/// `["lib/dep1", "lib/dep2"]`
///
/// ```text
/// lib
/// └── dep1
/// │   ├── foundry.toml
/// └── dep2
///     ├── foundry.toml
/// ```
pub fn foundry_toml_dirs(root: impl AsRef<Path>) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
        .filter_map(|e| ethers_solc::utils::canonicalize(e.path()).ok())
        .filter(|p| p.join(Config::FILE_NAME).exists())
        .collect()
}

/// Returns a remapping for the given dir
pub(crate) fn get_dir_remapping(dir: impl AsRef<Path>) -> Option<Remapping> {
    let dir = dir.as_ref();
    if let Some(dir_name) = dir.file_name().and_then(|s| s.to_str()).filter(|s| !s.is_empty()) {
        let mut r = Remapping { name: format!("{dir_name}/"), path: format!("{}", dir.display()) };
        if !r.path.ends_with('/') {
            r.path.push('/')
        }
        Some(r)
    } else {
        None
    }
}

/// Deserialize stringified percent. The value must be between 0 and 100 inclusive.
pub(crate) fn deserialize_stringified_percent<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let num: U256 =
        Numeric::deserialize(deserializer)?.try_into().map_err(serde::de::Error::custom)?;
    let num: u64 = num.try_into().map_err(serde::de::Error::custom)?;
    if num <= 100 {
        num.try_into().map_err(serde::de::Error::custom)
    } else {
        Err(serde::de::Error::custom("percent must be lte 100"))
    }
}
