//! Provides locations where to find foundry files
use crate::{platform::Platform, process::get_process, utils};
use serde::Deserialize;

use url::Url;

const FOUNDRY_REPO: &str = "foundry-rs/foundry";

/// Returns the url for the tarball for tag and version
pub fn release_tarball_url(tag: impl AsRef<str>, version: impl AsRef<str>) -> eyre::Result<Url> {
    let platform = Platform::current().ensure_supported()?;
    let tag = tag.as_ref();
    let version = version.as_ref();
    utils::parse_url(
        &format!(
            "https://github.com/${FOUNDRY_REPO}/releases/download/${tag}/foundry_${version}_${}_${}.tar.gz",
            platform.platform_name(),
            platform.arch_name(),
        )
    )
}

/// Returns the newest available foundryup version
pub async fn get_available_foundryup_version() -> eyre::Result<Release> {
    // TODO switch to proper release cycle to detect new versions
    let release = fetch_latest_github_release_nightly().await?;
    Ok(Release { version: release.name, tarball_url: utils::parse_url(&release.tarball_url)? })
}

/// Fetches the latest github release
async fn fetch_latest_github_release() -> eyre::Result<GithubRelease> {
    let process = get_process();
    Ok(process
        .client()
        .get(format!("https://api.github.com/repos/{FOUNDRY_REPO}/releases/latest"))
        .send()
        .await?
        .json()
        .await?)
}

/// Fetches the latest github release
async fn fetch_latest_github_release_nightly() -> eyre::Result<GithubRelease> {
    let process = get_process();
    Ok(process
        .client()
        .get(format!("https://api.github.com/repos/{FOUNDRY_REPO}/releases/tags/nightly"))
        .send()
        .await?
        .json()
        .await?)
}

/// Returns the github tag for `nightly`
pub async fn fetch_nightly_tag() -> eyre::Result<GithubTag> {
    let process = get_process();
    Ok(process
        .client()
        .get(format!("https://api.github.com/repos/{FOUNDRY_REPO}/git/refs/tags/nightly"))
        .send()
        .await?
        .json()
        .await?)
}

/// Bindings for a github tag
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GithubTag {
    #[serde(rename = "ref")]
    pub tag_ref: String,
    pub node_id: String,
    pub url: String,
    pub object: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Object {
    pub sha: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub url: String,
}

/// Bindings for a github release (excerpt) <https://docs.github.com/en/rest/releases/releases#get-the-latest-release>
#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub url: String,
    pub html_url: String,
    pub assets_url: String,
    pub upload_url: String,
    pub tarball_url: String,
    pub zipball_url: String,
    pub id: i64,
    pub node_id: String,
    pub tag_name: String,
    pub target_commitish: String,
    pub name: String,
    pub body: String,
    pub created_at: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub url: String,
    pub browser_download_url: String,
    pub id: i64,
    pub node_id: String,
    pub name: String,
    pub label: String,
    pub state: String,
    pub content_type: String,
    pub size: i64,
    pub download_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Represents a release with version and where to find the tarball
#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub tarball_url: Url,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nightly_exists() {}

    #[tokio::test]
    async fn can_fetch_nightly_tag() {
        let _tag = fetch_nightly_tag().await.unwrap();
    }

    #[tokio::test]
    async fn can_fetch_latest_release() {
        let _release = get_available_foundryup_version().await.unwrap();
    }
}
