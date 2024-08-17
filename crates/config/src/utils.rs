//! Utility functions

use crate::Config;
use alloy_primitives::U256;
use eyre::WrapErr;
use figment::value::Value;
use foundry_compilers::artifacts::{
    remappings::{Remapping, RemappingError},
    EvmVersion,
};
use revm_primitives::SpecId;
use serde::{de::Error, Deserialize, Deserializer};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use toml_edit::{DocumentMut, Item};

/// Loads the config for the current project workspace
pub fn load_config() -> Config {
    load_config_with_root(None)
}

/// Loads the config for the current project workspace or the provided root path
pub fn load_config_with_root(root: Option<PathBuf>) -> Config {
    if let Some(root) = root {
        Config::load_with_root(root)
    } else {
        Config::load_with_root(find_project_root_path(None).unwrap())
    }
    .sanitized()
}

/// Returns the path of the top-level directory of the working git tree. If there is no working
/// tree, an error is returned.
pub fn find_git_root_path(relative_to: impl AsRef<Path>) -> eyre::Result<PathBuf> {
    let path = relative_to.as_ref();
    let path = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .wrap_err_with(|| {
            format!("Failed detect git root path in current dir: {}", path.display())
        })?
        .stdout;
    let path = std::str::from_utf8(&path)?.trim_end_matches('\n');
    Ok(PathBuf::from(path))
}

/// Returns the root path to set for the project root
///
/// traverse the dir tree up and look for a `foundry.toml` file starting at the given path or cwd,
/// but only until the root dir of the current repo so that
///
/// ```text
/// -- foundry.toml
///
/// -- repo
///   |__ .git
///   |__sub
///      |__ [given_path | cwd]
/// ```
/// will still detect `repo` as root
pub fn find_project_root_path(path: Option<&PathBuf>) -> std::io::Result<PathBuf> {
    let cwd = &std::env::current_dir()?;
    let cwd = path.unwrap_or(cwd);
    let boundary = find_git_root_path(cwd)
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
        .filter_map(|e| foundry_compilers::utils::canonicalize(e.path()).ok())
        .filter(|p| p.join(Config::FILE_NAME).exists())
        .collect()
}

/// Returns a remapping for the given dir
pub(crate) fn get_dir_remapping(dir: impl AsRef<Path>) -> Option<Remapping> {
    let dir = dir.as_ref();
    if let Some(dir_name) = dir.file_name().and_then(|s| s.to_str()).filter(|s| !s.is_empty()) {
        let mut r = Remapping {
            context: None,
            name: format!("{dir_name}/"),
            path: format!("{}", dir.display()),
        };
        if !r.path.ends_with('/') {
            r.path.push('/')
        }
        Some(r)
    } else {
        None
    }
}

/// Returns all available `profile` keys in a given `.toml` file
///
/// i.e. The toml below would return would return `["default", "ci", "local"]`
/// ```toml
/// [profile.default]
/// ...
/// [profile.ci]
/// ...
/// [profile.local]
/// ```
pub fn get_available_profiles(toml_path: impl AsRef<Path>) -> eyre::Result<Vec<String>> {
    let mut result = vec![Config::DEFAULT_PROFILE.to_string()];

    if !toml_path.as_ref().exists() {
        return Ok(result)
    }

    let doc = read_toml(toml_path)?;

    if let Some(Item::Table(profiles)) = doc.as_table().get(Config::PROFILE_SECTION) {
        for (profile, _) in profiles {
            let p = profile.to_string();
            if !result.contains(&p) {
                result.push(p);
            }
        }
    }

    Ok(result)
}

/// Returns a [`toml_edit::Document`] loaded from the provided `path`.
/// Can raise an error in case of I/O or parsing errors.
fn read_toml(path: impl AsRef<Path>) -> eyre::Result<DocumentMut> {
    let path = path.as_ref().to_owned();
    let doc: DocumentMut = std::fs::read_to_string(path)?.parse()?;
    Ok(doc)
}

/// Deserialize stringified percent. The value must be between 0 and 100 inclusive.
pub(crate) fn deserialize_stringified_percent<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let num: U256 = Numeric::deserialize(deserializer)?.into();
    let num: u64 = num.try_into().map_err(serde::de::Error::custom)?;
    if num <= 100 {
        num.try_into().map_err(serde::de::Error::custom)
    } else {
        Err(serde::de::Error::custom("percent must be lte 100"))
    }
}

/// Deserialize a `u64` or "max" for `u64::MAX`.
pub(crate) fn deserialize_u64_or_max<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Val {
        Number(u64),
        String(String),
    }

    match Val::deserialize(deserializer)? {
        Val::Number(num) => Ok(num),
        Val::String(s) if s.eq_ignore_ascii_case("max") => Ok(u64::MAX),
        Val::String(s) => s.parse::<u64>().map_err(D::Error::custom),
    }
}

/// Deserialize a `usize` or "max" for `usize::MAX`.
pub(crate) fn deserialize_usize_or_max<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_u64_or_max(deserializer)?.try_into().map_err(D::Error::custom)
}

/// Helper type to parse both `u64` and `U256`
#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum Numeric {
    /// A [U256] value.
    U256(U256),
    /// A `u64` value.
    Num(u64),
}

impl From<Numeric> for U256 {
    fn from(n: Numeric) -> Self {
        match n {
            Numeric::U256(n) => n,
            Numeric::Num(n) => Self::from(n),
        }
    }
}

impl FromStr for Numeric {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("0x") {
            U256::from_str_radix(s, 16).map(Numeric::U256).map_err(|err| err.to_string())
        } else {
            U256::from_str(s).map(Numeric::U256).map_err(|err| err.to_string())
        }
    }
}

/// Returns the [SpecId] derived from [EvmVersion]
#[inline]
pub fn evm_spec_id(evm_version: &EvmVersion, alphanet: bool) -> SpecId {
    if alphanet {
        return SpecId::PRAGUE_EOF;
    }
    match evm_version {
        EvmVersion::Homestead => SpecId::HOMESTEAD,
        EvmVersion::TangerineWhistle => SpecId::TANGERINE,
        EvmVersion::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        EvmVersion::Byzantium => SpecId::BYZANTIUM,
        EvmVersion::Constantinople => SpecId::CONSTANTINOPLE,
        EvmVersion::Petersburg => SpecId::PETERSBURG,
        EvmVersion::Istanbul => SpecId::ISTANBUL,
        EvmVersion::Berlin => SpecId::BERLIN,
        EvmVersion::London => SpecId::LONDON,
        EvmVersion::Paris => SpecId::MERGE,
        EvmVersion::Shanghai => SpecId::SHANGHAI,
        EvmVersion::Cancun => SpecId::CANCUN,
        EvmVersion::Prague => SpecId::PRAGUE_EOF,
    }
}

#[cfg(test)]
mod tests {
    use crate::get_available_profiles;
    use std::path::Path;

    #[test]
    fn get_profiles_from_toml() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r"
                [foo.baz]
                libs = ['node_modules', 'lib']

                [profile.default]
                libs = ['node_modules', 'lib']

                [profile.ci]
                libs = ['node_modules', 'lib']

                [profile.local]
                libs = ['node_modules', 'lib']
            ",
            )?;

            let path = Path::new("./foundry.toml");
            let profiles = get_available_profiles(path).unwrap();

            assert_eq!(
                profiles,
                vec!["default".to_string(), "ci".to_string(), "local".to_string()]
            );

            Ok(())
        });
    }
}
