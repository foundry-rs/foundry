//! Utility functions

use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

use crate::Config;
use ethers_solc::{
    error::SolcError,
    remappings::{Remapping, RemappingError},
};
use figment::value::Value;

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
pub fn find_git_root_path() -> eyre::Result<PathBuf> {
    let path =
        std::process::Command::new("git").args(&["rev-parse", "--show-toplevel"]).output()?.stdout;
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
    let boundary = find_git_root_path().unwrap_or_else(|_| std::env::current_dir().unwrap());
    let cwd = std::env::current_dir()?;
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

/// Parses all libraries in the form of
/// `<file>:<lib>:<addr>`
///
/// # Example
///
/// ```
/// use foundry_config::parse_libraries;
/// let libs = parse_libraries(&[
///     "src/DssSpell.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string(),
/// ])
/// .unwrap();
/// ```
pub fn parse_libraries(
    libs: &[String],
) -> Result<BTreeMap<String, BTreeMap<String, String>>, SolcError> {
    let mut libraries = BTreeMap::default();
    for lib in libs {
        let mut items = lib.split(':');
        let file = items
            .next()
            .ok_or_else(|| SolcError::msg(format!("failed to parse invalid library: {lib}")))?;
        let lib = items
            .next()
            .ok_or_else(|| SolcError::msg(format!("failed to parse invalid library: {lib}")))?;
        let addr = items
            .next()
            .ok_or_else(|| SolcError::msg(format!("failed to parse invalid library: {lib}")))?;
        if items.next().is_some() {
            return Err(SolcError::msg(format!("failed to parse invalid library: {lib}")))
        }
        libraries
            .entry(file.to_string())
            .or_insert_with(BTreeMap::default)
            .insert(lib.to_string(), addr.to_string());
    }
    Ok(libraries)
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
