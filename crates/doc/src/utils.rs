//! `GitSource` + `Deployments` helpers.
//!
//! Pure functions ported from the legacy `forge-doc` preprocessors:
//! * `git_source_url`: `<repo>/blob/<commit>/<rel>` for a source file.
//! * `read_deployments`: load `<dir>/<network>/<contract>.json` artifacts.

use alloy_primitives::Address;
use path_slash::PathExt;
use serde::Deserialize;
use solar::ast::ContractKind;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

// ── git source ────────────────────────────────────────────────────────────────

/// Build the `{repo}/blob/{commit}/{rel}` URL for a source file.
///
/// Returns `None` if `item_path` is not under `root` (i.e. for absolute external paths).
/// `commit` falls back to `"HEAD"` when empty (GitHub's `blob/HEAD/...` resolves
/// to the repository's default branch regardless of whether it is `main`,
/// `master`, or anything else).
///
/// Path components are joined with `/` so the URL is well-formed on Windows.
pub fn git_source_url(repo: &str, commit: &str, root: &Path, item_path: &Path) -> Option<String> {
    let repo = repo.trim_end_matches('/');
    let commit = if commit.is_empty() { "HEAD" } else { commit };
    let rel = item_path.strip_prefix(root).ok()?;
    Some(format!("{repo}/blob/{commit}/{}", rel.to_slash_lossy()))
}

/// Return a `{repo}/raw/{commit}/...` URL suitable for embedding binary assets
/// (images, fonts, etc.) directly rather than the GitHub blob viewer page.
pub fn git_raw_url(repo: &str, commit: &str, root: &Path, item_path: &Path) -> Option<String> {
    let repo = repo.trim_end_matches('/');
    let commit = if commit.is_empty() { "HEAD" } else { commit };
    let rel = item_path.strip_prefix(root).ok()?;
    Some(format!("{repo}/raw/{commit}/{}", rel.to_slash_lossy()))
}

// ── deployments ──────────────────────────────────────────────────────────────

/// A contract deployment entry, deserialised from `<dir>/<network>/<contract>.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct Deployment {
    /// The contract address.
    pub address: Address,
    /// The network name (filled in from the parent directory name).
    pub network: Option<String>,
}

/// Read all deployments for a Solidity contract file.
///
/// Walks `deployments_dir` (defaulting to `<root>/deployments`), looks for a
/// `<network>/<contract-stem>.json` file in each top-level subdirectory, and
/// returns the parsed [`Deployment`]s tagged with their network name.
///
/// Errors when reading individual entries are silently skipped to mirror the
/// legacy preprocessor's lenient behaviour.
pub fn read_deployments(
    root: &Path,
    deployments_dir: Option<&Path>,
    contract_file: &Path,
) -> Vec<Deployment> {
    let dir = root.join(deployments_dir.unwrap_or_else(|| Path::new("deployments")));
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    // Switch ".sol" -> ".json" and keep just the file name.
    let mut filename: PathBuf = contract_file.to_path_buf();
    filename.set_extension("json");
    let Some(filename) = filename.file_name().map(PathBuf::from) else { return Vec::new() };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_dir() {
            continue;
        }
        let Ok(network) = entry.file_name().into_string() else { continue };
        let path = entry.path().join(&filename);
        let Ok(content) = fs::read_to_string(&path) else { continue };
        let Ok(mut deployment) = serde_json::from_str::<Deployment>(&content) else { continue };
        deployment.network = Some(network);
        out.push(deployment);
    }
    // Sort for deterministic output across platforms (fs::read_dir order is unspecified).
    out.sort_by(|a, b| {
        a.network.as_deref().unwrap_or("").cmp(b.network.as_deref().unwrap_or("")).then_with(|| {
            let af = format!("{:#x}", a.address);
            let bf = format!("{:#x}", b.address);
            af.cmp(&bf)
        })
    });
    out
}

/// Map a source to its documentation location, retaining the legacy external-library layout.
pub(crate) fn relative_source_path(root: &Path, source: &Path) -> PathBuf {
    source.strip_prefix(root).map(Path::to_path_buf).unwrap_or_else(|_| {
        let components = source.components().collect::<Vec<_>>();
        PathBuf::from("lib")
            .join(components[components.len().saturating_sub(3)..].iter().collect::<PathBuf>())
    })
}

/// Whether joining a generated path can stay within the documentation directory.
pub(crate) fn is_safe_output_path(path: &Path) -> bool {
    !path.is_absolute()
        && !path.components().any(|component| {
            matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
        })
}

/// The filename prefix used for a contract page.
pub(crate) const fn contract_kind_str(kind: ContractKind) -> &'static str {
    match kind {
        ContractKind::Contract => "contract",
        ContractKind::AbstractContract => "abstract",
        ContractKind::Interface => "interface",
        ContractKind::Library => "library",
    }
}

/// Construct an item page beside its source in the documentation tree.
pub(crate) fn page_path(source: &Path, prefix: &str, name: &str) -> PathBuf {
    source.parent().unwrap_or(Path::new("")).join(format!("{prefix}.{name}.mdx"))
}
