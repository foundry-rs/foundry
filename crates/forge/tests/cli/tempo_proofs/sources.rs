//! Pinned upstream Solidity sources for the Tempo proof integration test.

use alloy_primitives::hex;
use eyre::{Result, ensure};
use foundry_common::fs;
use foundry_test_utils::TestProject;
use reqwest::Client;
use revm::precompile::crypto;
use serde::Deserialize;
use std::{
    io::{ErrorKind, Write},
    path::PathBuf,
    time::Duration,
};
use tempfile::NamedTempFile;

#[derive(Deserialize)]
struct Source {
    path: String,
    url: String,
    sha256: String,
}

pub(super) async fn prepare(prj: &TestProject) {
    let sources = serde_json::from_str::<Vec<Source>>(include_str!(
        "../../../../../testdata/fixtures/tempo-proof-sources.lock.json"
    ))
    .unwrap();
    let client = Client::builder().timeout(Duration::from_secs(30)).build().unwrap();
    let cache = std::env::var_os("FOUNDRY_EXTERNAL_TEST_CACHE")
        .map(|root| PathBuf::from(root).join("tempo-solidity-sources"));
    if let Some(cache) = &cache {
        fs::create_dir_all(cache).unwrap();
    }
    for source in sources {
        let cached = cache.as_ref().map(|cache| cache.join(&source.sha256));
        let data = if let Some(path) = &cached
            && path.exists()
        {
            fs::read(path).unwrap()
        } else {
            client
                .get(&source.url)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .to_vec()
        };
        verify(&source, &data).unwrap();
        if let Some(path) = &cached
            && !path.exists()
        {
            let mut temporary = NamedTempFile::new_in(path.parent().unwrap()).unwrap();
            temporary.write_all(&data).unwrap();
            if let Err(error) = temporary.persist_noclobber(path) {
                assert_eq!(error.error.kind(), ErrorKind::AlreadyExists, "{error}");
                verify(&source, &fs::read(path).unwrap()).unwrap();
            }
        }
        prj.create_file(&source.path, std::str::from_utf8(&data).unwrap());
    }
}

fn verify(source: &Source, data: &[u8]) -> Result<()> {
    let actual = hex::encode(crypto().sha256(data));
    ensure!(
        actual == source.sha256,
        "source checksum mismatch: {} (expected {}, got {actual})",
        source.path,
        source.sha256
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_checksum_rejects_corruption() {
        let source = Source {
            path: "test.sol".into(),
            url: String::new(),
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        verify(&source, b"abc").unwrap();
        assert!(verify(&source, b"abd").is_err());
    }
}
