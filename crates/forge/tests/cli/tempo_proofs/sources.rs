//! Pinned upstream Solidity sources for the Tempo proof integration test.

use foundry_common::fs;
use foundry_test_utils::TestProject;
use reqwest::Client;
use std::{
    io::{ErrorKind, Write},
    path::PathBuf,
    time::Duration,
};
use tempfile::NamedTempFile;

const COMMONWARE: &str = "commonwarexyz/monorepo/456538f0e63a2d8e4ee309af27af836f9466d5a1";
const OPTIMISM: &str = "ethereum-optimism/optimism/b84018de4fc5ea47f13b6ebdf7a2f793d8334ea8";

pub(super) async fn prepare(prj: &TestProject) {
    let client = Client::builder().timeout(Duration::from_secs(30)).build().unwrap();
    let cache = std::env::var_os("FOUNDRY_EXTERNAL_TEST_CACHE")
        .map(|root| PathBuf::from(root).join("tempo-solidity-sources"));
    for (upstream, prefix, directory, files) in [
        (
            COMMONWARE,
            "sol/",
            "lib/commonware",
            &[
                "LICENSE-APACHE",
                "LICENSE-MIT",
                "sol/src/certificate/LibBLS12381.sol",
                "sol/src/certificate/LibBLS12381Threshold.sol",
                "sol/src/codec/LibCodec.sol",
                "sol/src/simplex/LibSimplex.sol",
                "sol/src/simplex/LibSimplexBLS12381Threshold.sol",
            ][..],
        ),
        (
            OPTIMISM,
            "packages/contracts-bedrock/",
            "lib/optimism",
            &[
                "LICENSE",
                "packages/contracts-bedrock/src/libraries/Bytes.sol",
                "packages/contracts-bedrock/src/libraries/rlp/RLPErrors.sol",
                "packages/contracts-bedrock/src/libraries/rlp/RLPReader.sol",
                "packages/contracts-bedrock/src/libraries/trie/MerkleTrie.sol",
            ][..],
        ),
    ] {
        for &file in files {
            let cached = cache.as_ref().map(|root| root.join(upstream).join(file));
            let data = if let Some(path) = &cached
                && path.exists()
            {
                fs::read(path).unwrap()
            } else {
                let data = client
                    .get(format!("https://raw.githubusercontent.com/{upstream}/{file}"))
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap()
                    .to_vec();
                if let Some(path) = &cached {
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    let mut temporary = NamedTempFile::new_in(path.parent().unwrap()).unwrap();
                    temporary.write_all(&data).unwrap();
                    if let Err(error) = temporary.persist_noclobber(path) {
                        assert_eq!(error.error.kind(), ErrorKind::AlreadyExists, "{error}");
                    }
                }
                data
            };
            let path = file.strip_prefix(prefix).unwrap_or(file);
            prj.create_file(format!("{directory}/{path}"), std::str::from_utf8(&data).unwrap());
        }
    }
}
