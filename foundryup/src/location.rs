//! Provides locations where to find foundry files
use crate::{
    platform::Platform,
    process::{get_process, Process},
    utils,
};
use serde::{Deserialize, Serialize};
use std::env;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubTag {
    #[serde(rename = "ref")]
    tag_ref: String,
    node_id: String,
    url: String,
    object: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    sha: String,
    #[serde(rename = "type")]
    object_type: String,
    url: String,
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
}
