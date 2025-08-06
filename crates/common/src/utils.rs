//! Uncategorised utilities.

use alloy_primitives::{hex, keccak256, Bytes, B256, U256};
use eyre::{eyre, Result};
use foundry_compilers::artifacts::BytecodeObject;
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};
use cargo_metadata::MetadataCommand;
use convert_case::{Case, Casing};
use toml::Value;

static BYTECODE_PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"__\$.{34}\$__").expect("invalid regex"));

/// Block on a future using the current tokio runtime on the current thread.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    block_on_handle(&tokio::runtime::Handle::current(), future)
}

/// Block on a future using the current tokio runtime on the current thread with the given handle.
pub fn block_on_handle<F: std::future::Future>(
    handle: &tokio::runtime::Handle,
    future: F,
) -> F::Output {
    tokio::task::block_in_place(|| handle.block_on(future))
}

/// Computes the storage slot as specified by `ERC-7201`, using the `erc7201` formula ID.
///
/// This is defined as:
///
/// ```text
/// erc7201(id: string) = keccak256(keccak256(id) - 1) & ~0xff
/// ```
///
/// # Examples
///
/// ```
/// use alloy_primitives::b256;
/// use foundry_common::erc7201;
///
/// assert_eq!(
///     erc7201("example.main"),
///     b256!("0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab500"),
/// );
/// ```
pub fn erc7201(id: &str) -> B256 {
    let x = U256::from_be_bytes(keccak256(id).0) - U256::from(1);
    keccak256(x.to_be_bytes::<32>()) & B256::from(!U256::from(0xff))
}

/// Utility function to ignore metadata hash of the given bytecode.
/// This assumes that the metadata is at the end of the bytecode.
pub fn ignore_metadata_hash(bytecode: &[u8]) -> &[u8] {
    // Get the last two bytes of the bytecode to find the length of CBOR metadata.
    let Some((rest, metadata_len_bytes)) = bytecode.split_last_chunk() else { return bytecode };
    let metadata_len = u16::from_be_bytes(*metadata_len_bytes) as usize;
    if metadata_len > rest.len() {
        return bytecode;
    }
    let (rest, metadata) = rest.split_at(rest.len() - metadata_len);
    if ciborium::from_reader::<ciborium::Value, _>(metadata).is_ok() { rest } else { bytecode }
}

/// Strips all __$xxx$__ placeholders from the bytecode if it's an unlinked bytecode.
/// by replacing them with 20 zero bytes.
/// This is useful for matching bytecodes to a contract source, and for the source map,
/// in which the actual address of the placeholder isn't important.
pub fn strip_bytecode_placeholders(bytecode: &BytecodeObject) -> Option<Bytes> {
    match &bytecode {
        BytecodeObject::Bytecode(bytes) => Some(bytes.clone()),
        BytecodeObject::Unlinked(s) => {
            // Replace all __$xxx$__ placeholders with 32 zero bytes
            let s = (*BYTECODE_PLACEHOLDER_RE).replace_all(s, "00".repeat(40));
            let bytes = hex::decode(s.as_bytes());
            Some(bytes.ok()?.into())
        }
    }
}

#[derive(Debug, Clone)]
pub struct RustProjectInfo {
    pub path: PathBuf,
    pub package_name: String,
    pub sdk_version: Option<String>,
}

/// Find all Rust projects in the given directory
pub fn find_rust_contracts(
    src_root: &Path,
    project_root: Option<&Path>,
) -> Result<BTreeMap<String, RustProjectInfo>> {
    let mut projects = BTreeMap::new();

    if !src_root.is_dir() {
        return Ok(projects);
    }

    let gitignore = project_root.and_then(|root| load_gitignore(root).ok()).unwrap_or_default();

    // Use WalkDir for directory traversal similar to the original function
    for entry in walkdir::WalkDir::new(src_root)
        .into_iter()
        .filter_entry(|e| {
            e.file_type().is_dir() 
            && !should_skip_directory(e.path(), &gitignore, project_root)
        })
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let cargo_toml = path.join("Cargo.toml");
        
        if cargo_toml.exists() {
            if let Ok(info) = read_cargo_info(&cargo_toml) {
                let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                
                let project_info = RustProjectInfo {
                    path: canonical_path,
                    package_name: info.package_name.clone(),
                    sdk_version: info.sdk_version,
                };
                
                projects.insert(info.package_name, project_info);
            }
        }
    }

    Ok(projects)
}

/// Check if directory should be skipped
fn should_skip_directory(
    dir: &Path,
    gitignore: &GitignoreRules,
    project_root: Option<&Path>,
) -> bool {
    // Standard directories to skip
    if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
        if matches!(
            name,
            "target" | ".git" | "node_modules" | ".idea" | ".vscode" | "dist" | "build"
        ) {
            return true;
        }
    }

    // Check gitignore rules
    if let Some(root) = project_root {
        if let Ok(relative) = dir.strip_prefix(root) {
            return gitignore.should_ignore(relative);
        }
    }

    false
}

struct CargoInfo {
    package_name: String,
    sdk_version: Option<String>,
}

/// Read package info from Cargo.toml
fn read_cargo_info(cargo_toml_path: &Path) -> eyre::Result<CargoInfo> {
    let content = std::fs::read_to_string(cargo_toml_path)?;
    let value: Value = toml::from_str(&content)?;

    let package_name = value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| eyre::eyre!("Package name not found in Cargo.toml"))?
        .to_string();

    let sdk_version = get_sdk_version(&value);

    Ok(CargoInfo { package_name, sdk_version })
}

/// Extract fluentbase-sdk version from dependencies
fn get_sdk_version(cargo_toml: &Value) -> Option<String> {
    cargo_toml.get("dependencies")?.get("fluentbase-sdk")?.as_str().map(String::from).or_else(
        || {
            cargo_toml
                .get("dependencies")?
                .get("fluentbase-sdk")?
                .as_table()?
                .get("tag")
                .or_else(|| {
                    cargo_toml
                        .get("dependencies")?
                        .get("fluentbase-sdk")?
                        .as_table()?
                        .get("version")
                })
                .and_then(|v| v.as_str())
                .map(String::from)
        },
    )
}

#[derive(Default)]
struct GitignoreRules {
    patterns: Vec<String>,
}

impl GitignoreRules {
    fn should_ignore(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        self.patterns.iter().any(|pattern| {
            if pattern.starts_with('/') {
                path_str.starts_with(&pattern[1..])
            } else if pattern.ends_with('/') {
                path.is_dir() && path_str.contains(&pattern[..pattern.len() - 1])
            } else {
                path.components().any(|component| {
                    component
                        .as_os_str()
                        .to_str()
                        .map_or(false, |name| name == pattern || matches_simple_glob(name, pattern))
                })
            }
        })
    }
}

/// Load gitignore rules from file
fn load_gitignore(project_root: &Path) -> eyre::Result<GitignoreRules> {
    let gitignore_path = project_root.join(".gitignore");

    if !gitignore_path.exists() {
        return Ok(GitignoreRules::default());
    }

    let content = std::fs::read_to_string(gitignore_path)?;
    let patterns: Vec<String> = content
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .map(|line| line.trim().to_string())
        .collect();

    Ok(GitignoreRules { patterns })
}

/// Simple glob pattern matching
fn matches_simple_glob(text: &str, pattern: &str) -> bool {
    match (pattern.starts_with('*'), pattern.ends_with('*')) {
        (true, true) => {
            let middle = &pattern[1..pattern.len() - 1];
            text.contains(middle)
        }
        (true, false) => {
            let suffix = &pattern[1..];
            text.ends_with(suffix)
        }
        (false, true) => {
            let prefix = &pattern[..pattern.len() - 1];
            text.starts_with(prefix)
        }
        (false, false) => text == pattern,
    }
}

pub fn normalize_contract_name(name: &str) -> String {
    // Check if the name has .wasm suffix
    if let Some(base_name) = name.strip_suffix(".wasm") {
        // Convert from PascalCase to kebab-case
        base_name.to_case(Case::Kebab)
    } else {
        // Already in kebab-case format or other format
        name.to_string()
    }
}
