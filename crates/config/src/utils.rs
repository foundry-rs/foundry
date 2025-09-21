//! Utility functions

use crate::Config;
use alloy_primitives::U256;
use figment::value::Value;
use foundry_compilers::artifacts::{
    EvmVersion,
    remappings::{Remapping, RemappingError},
};
use revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Deserializer, Serializer, de::Error};
use std::{
    io,
    path::{Path, PathBuf},
    str::FromStr,
};

// TODO: Why do these exist separately from `Config::load`?

/// Loads the config for the current project workspace.
pub fn load_config() -> eyre::Result<Config> {
    load_config_with_root(None)
}

/// Loads the config for the current project workspace or the provided root path.
pub fn load_config_with_root(root: Option<&Path>) -> eyre::Result<Config> {
    let root = match root {
        Some(root) => root,
        None => &find_project_root(None)?,
    };
    Ok(Config::load_with_root(root)?.sanitized())
}

/// Returns the path of the top-level directory of the working git tree.
pub fn find_git_root(relative_to: &Path) -> io::Result<Option<PathBuf>> {
    let root =
        if relative_to.is_absolute() { relative_to } else { &dunce::canonicalize(relative_to)? };
    Ok(root.ancestors().find(|p| p.join(".git").exists()).map(Path::to_path_buf))
}

/// Returns the root path to set for the project root.
///
/// Traverse the dir tree up and look for a `foundry.toml` file starting at the given path or cwd,
/// but only until the root dir of the current repo so that:
///
/// ```text
/// -- foundry.toml
///
/// -- repo
///   |__ .git
///   |__sub
///      |__ [given_path | cwd]
/// ```
///
/// will still detect `repo` as root.
///
/// Returns `repo` or `cwd` if no `foundry.toml` is found in the tree.
///
/// Returns an error if:
/// - `cwd` is `Some` and is not a valid directory;
/// - `cwd` is `None` and the [`std::env::current_dir`] call fails.
pub fn find_project_root(cwd: Option<&Path>) -> io::Result<PathBuf> {
    let cwd = match cwd {
        Some(path) => path,
        None => &std::env::current_dir()?,
    };
    let boundary = find_git_root(cwd)?;
    let found = cwd
        .ancestors()
        // Don't look outside of the git repo if it exists.
        .take_while(|p| if let Some(boundary) = &boundary { p.starts_with(boundary) } else { true })
        .find(|p| p.join(Config::FILE_NAME).is_file())
        .map(Path::to_path_buf);
    Ok(found.or(boundary).unwrap_or_else(|| cwd.to_path_buf()))
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
        .filter_map(|e| dunce::canonicalize(e.path()).ok())
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

/// Deserialize into `U256` from either a `u64` or a `U256` hex string.
pub fn deserialize_u64_to_u256<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumericValue {
        U256(U256),
        U64(u64),
    }

    match NumericValue::deserialize(deserializer)? {
        NumericValue::U64(n) => Ok(U256::from(n)),
        NumericValue::U256(n) => Ok(n),
    }
}

/// Serialize `U256` as `u64` if it fits, otherwise as a hex string.
/// If the number fits into a i64, serialize it as number without quotation marks.
/// If the number fits into a u64, serialize it as a stringified number with quotation marks.
/// Otherwise, serialize it as a hex string with quotation marks.
pub fn serialize_u64_or_u256<S>(n: &U256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // The TOML specification handles integers as i64 so the number representation is limited to
    // i64. If the number is larger than `i64::MAX` and up to `u64::MAX`, we serialize it as a
    // string to avoid losing precision.
    if let Ok(n_i64) = i64::try_from(*n) {
        serializer.serialize_i64(n_i64)
    } else if let Ok(n_u64) = u64::try_from(*n) {
        serializer.serialize_str(&n_u64.to_string())
    } else {
        serializer.serialize_str(&format!("{n:#x}"))
    }
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
pub fn evm_spec_id(evm_version: EvmVersion) -> SpecId {
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
        EvmVersion::Prague => SpecId::PRAGUE,
        EvmVersion::Osaka => SpecId::OSAKA,
    }
}
