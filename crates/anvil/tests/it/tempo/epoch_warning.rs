//! CLI diagnostics for unresolved Tempo fork epoch lengths.

use super::*;
use std::fs::{File, read_to_string};

const FALLBACK_WARNING: &str = "Warning: Tempo fork epoch length is unavailable; using 21600. \
                              Set --tempo.epoch-length to match the source.\n";

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_fallback_warning() {
    let (source_api, source) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    source_api.mine_one().await.unwrap();
    let hidden = spawn_rpc_proxy_method_not_found_before(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        usize::MAX,
    )
    .await;

    for (endpoint, explicit, quiet, warn) in [
        (hidden.clone(), None, false, true),
        (hidden.clone(), Some("3"), false, false),
        (hidden, None, true, false),
        (source.http_endpoint(), None, false, false),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let temp = tempfile::tempdir().unwrap();
        let stderr = temp.path().join("stderr");
        let mut command = Command::new(anvil_binary());
        command.args([
            "--network",
            "tempo",
            "--accounts",
            "0",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-mining",
            "--no-storage-caching",
            "--color",
            "never",
            "--fork-url",
            &endpoint,
            "--fork-block-number",
            "1",
        ]);
        if let Some(length) = explicit {
            command.args(["--tempo.epoch-length", length]);
        }
        if quiet {
            command.arg("--quiet");
        }
        let mut child = ChildGuard(
            command
                .env("RUST_LOG", "off")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(File::create(&stderr).unwrap())
                .spawn()
                .unwrap(),
        );
        let provider = http_provider(&format!("http://127.0.0.1:{port}"));
        tokio::time::timeout(Duration::from_secs(10), async {
            while provider.get_chain_id().await.is_err() {
                if let Some(status) = child.0.try_wait().unwrap() {
                    panic!("anvil exited with {status}: {}", read_to_string(&stderr).unwrap());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("anvil should finish startup");

        let identity = tempo_consensus_identity(&provider).await;
        let mut expected = if quiet {
            String::new()
        } else {
            format!(
                "\nTempo Consensus\n==================\nIdentity:       {}\n\
                 Proposer:       {}\nEpoch length:   {}\n",
                identity.identity, identity.proposer, identity.epoch_length,
            )
        };
        if warn {
            expected.push_str(FALLBACK_WARNING);
        }
        assert_stderr(&stderr, &expected).await;

        provider
            .raw_request::<_, ()>(
                "anvil_reset".into(),
                serde_json::json!([{ "forking": {
                    "jsonRpcUrl": endpoint, "blockNumber": 1,
                } }]),
            )
            .await
            .unwrap();
        if warn {
            expected.push_str(FALLBACK_WARNING);
        }
        assert_stderr(&stderr, &expected).await;

        // A failed reset must not report a fallback or change the active identity.
        assert!(
            provider
                .raw_request::<_, ()>(
                    "anvil_reset".into(),
                    serde_json::json!([{ "forking": {
                        "jsonRpcUrl": endpoint, "blockNumber": 2,
                    } }]),
                )
                .await
                .is_err()
        );
        assert_eq!(tempo_consensus_identity(&provider).await, identity);
        assert_stderr(&stderr, &expected).await;
    }
}

async fn assert_stderr(path: &std::path::Path, expected: &str) {
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while read_to_string(path).unwrap().len() < expected.len() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert_data_eq!(read_to_string(path).unwrap(), expected);
}
