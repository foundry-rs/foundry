//! CLI diagnostics for unresolved Tempo fork epoch lengths.

use super::*;
use alloy_provider::{IpcConnect, builder};
use std::fs::{File, read_to_string};

const FALLBACK_WARNING: &str = "Warning: Tempo fork epoch length is unavailable; using 21600. \
                              Set --tempo.epoch-length to match the source.\n";

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_fallback_warning() {
    let (source_api, source) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    source_api.mine_one().await.unwrap();
    assert_eq!(tempo_consensus_identity(&source.http_provider()).await.epoch_length, 21600);
    let hidden = spawn_rpc_proxy_method_not_found_before(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        usize::MAX,
    )
    .await;

    for (endpoint, explicit, quiet, warn) in [
        (hidden.clone(), None, false, true),
        (hidden.clone(), Some("3"), false, false),
        (hidden.clone(), Some("21600"), false, false),
        (hidden, None, true, false),
        (source.http_endpoint(), None, false, false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let stderr = temp.path().join("stderr");
        let startup_state = temp.path().join("startup.json");
        let ipc = if cfg!(unix) {
            temp.path().join("anvil.ipc").to_string_lossy().into_owned()
        } else {
            format!(r"\\.\pipe\anvil_epoch_warning_{}.ipc", rand::random::<u64>())
        };
        let mut command = Command::new(anvil_binary());
        command.args([
            "--network",
            "tempo",
            "--accounts",
            "0",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--ipc",
            &ipc,
            "--dump-state",
            startup_state.to_str().unwrap(),
            "--state-interval",
            "1",
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
        // Periodic dumping starts after try_spawn returns, so this also waits for all startup
        // diagnostics. Serving RPC or printing the banner alone does not provide that guarantee.
        tokio::time::timeout(Duration::from_secs(10), async {
            while !startup_state.exists() {
                if let Some(status) = child.0.try_wait().unwrap() {
                    panic!("anvil exited with {status}: {}", read_to_string(&stderr).unwrap());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("anvil should finish startup");

        let provider = builder::<FoundryNetwork>().connect_ipc(IpcConnect::new(ipc)).await.unwrap();
        let identity = provider
            .raw_request::<_, TempoConsensusIdentity>("anvil_tempoConsensusIdentity".into(), ())
            .await
            .unwrap();
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
        assert_data_eq!(read_to_string(&stderr).unwrap(), expected.as_str());

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
        assert_data_eq!(read_to_string(&stderr).unwrap(), expected.as_str());

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
        assert_eq!(
            provider
                .raw_request::<_, TempoConsensusIdentity>("anvil_tempoConsensusIdentity".into(), ())
                .await
                .unwrap(),
            identity,
        );
        assert_data_eq!(read_to_string(&stderr).unwrap(), expected.as_str());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_fallback_warning_requires_successful_startup() {
    let (source_api, source) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    source_api.mine_one().await.unwrap();
    let hidden = spawn_rpc_proxy_method_not_found_before(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        usize::MAX,
    )
    .await;
    // Keep the listener alive so the fork is prepared successfully but the server cannot start.
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let bind_error = tokio::net::TcpListener::bind(address).await.unwrap_err();
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(anvil_binary())
            .args([
                "--network",
                "tempo",
                "--accounts",
                "0",
                "--host",
                "127.0.0.1",
                "--port",
                &address.port().to_string(),
                "--no-mining",
                "--no-storage-caching",
                "--color",
                "never",
                "--fork-url",
                &hidden,
                "--fork-block-number",
                "1",
            ])
            .env("RUST_LOG", "off")
            .env_remove("FOUNDRY_DEBUG")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("anvil should fail startup")
    .unwrap();
    assert!(!output.status.success());
    assert_data_eq!(String::from_utf8(output.stdout).unwrap(), "");
    assert_data_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("Error: {bind_error}\n").as_str(),
    );
}
