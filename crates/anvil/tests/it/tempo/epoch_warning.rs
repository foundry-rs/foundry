//! CLI diagnostics for unresolved Tempo fork epoch lengths.

use super::*;
use std::fs::{File, read_to_string};

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_fallback_warning() {
    let (source_api, source) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
    )
    .await;
    source_api.mine_one().await.unwrap();
    let hidden = spawn_rpc_proxy_rejecting_method_after(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        0,
    )
    .await;
    let (late, late_root_reads) =
        epoch_metadata::epoch_metadata_after_fork_root(source.http_endpoint(), 1).await;

    for (urls, explicit, quiet, length, warn, reset) in [
        (vec![hidden.clone()], None, false, 21_600, true, None),
        (vec![hidden.clone()], Some("3"), false, 3, false, None),
        (vec![source.http_endpoint()], None, false, 3, false, None),
        (vec![hidden.clone(), source.http_endpoint()], None, false, 3, false, None),
        (vec![hidden.clone()], None, true, 21_600, false, None),
        (vec![], None, false, 21_600, false, None),
        (vec![source.http_endpoint()], None, false, 3, false, Some((late, 3, false))),
        (vec![source.http_endpoint()], None, false, 3, false, Some((hidden, 21_600, true))),
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
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-mining",
            "--color",
            "never",
        ]);
        for url in &urls {
            command.args(["--fork-url", url]);
        }
        if !urls.is_empty() {
            command.args(["--fork-block-number", "1", "--no-storage-caching"]);
        }
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
        let identity = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if provider.get_chain_id().await.is_ok() {
                    break tempo_consensus_identity(&provider).await;
                }
                if let Some(status) = child.0.try_wait().unwrap() {
                    panic!("anvil exited with {status}: {}", read_to_string(&stderr).unwrap());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("anvil should finish startup");
        assert_eq!(identity.epoch_length, length);
        let mut expected = String::new();
        if warn {
            expected.push_str(
                "Warning: Tempo fork epoch length is unavailable; using 21600. \
                 Set --tempo.epoch-length to match the source.\n",
            );
        }
        if !quiet {
            expected.push_str(&format!(
                "\nTempo Consensus\n==================\nIdentity:       {}\nProposer:       {}\nEpoch length:   {length}\n",
                identity.identity, identity.proposer,
            ));
            // The RPC listener can become ready before the startup banner is written.
            tokio::time::timeout(Duration::from_secs(5), async {
                let last_line = format!("Epoch length:   {length}\n");
                while !read_to_string(&stderr).unwrap().ends_with(&last_line) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("anvil should print its consensus identity");
        }
        if let Some((endpoint, length, warn)) = reset {
            provider
                .client()
                .request::<_, serde_json::Value>(
                    "anvil_reset",
                    (serde_json::json!({ "forking": {
                        "jsonRpcUrl": endpoint,
                        "blockNumber": 1,
                    } }),),
                )
                .await
                .unwrap();
            assert_eq!(tempo_consensus_identity(&provider).await.epoch_length, length);
            if warn {
                expected.push_str(
                    "Warning: Tempo fork epoch length is unavailable; using 21600. \
                     Set --tempo.epoch-length to match the source.\n",
                );
            }
        }
        drop(child);
        assert_data_eq!(read_to_string(&stderr).unwrap(), expected);
    }
    assert_eq!(late_root_reads.load(Ordering::Relaxed), 2);
}
