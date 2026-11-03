//! CLI diagnostics for unresolved Tempo fork epoch lengths.

use super::*;
use std::{
    fs::{File, read_to_string},
    sync::atomic::AtomicUsize,
};

const FALLBACK_WARNING: &str = "Warning: Tempo fork epoch length is unavailable; using 21600. \
                              Set --tempo.epoch-length to match the source.\n";

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_fallback_warning() {
    let (source_api, source) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
    )
    .await;
    source_api.anvil_mine(Some(U256::from(3)), None).await.unwrap();
    let hidden = spawn_rpc_proxy_rejecting_method_after(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        0,
    )
    .await;
    let node = CliNode::spawn("tempo", &["--fork-url", &hidden, "--fork-block-number", "1"]).await;
    let identity = tempo_consensus_identity(&node.provider).await;
    assert_eq!(identity.epoch_length, 21_600);
    let startup = consensus_output(&identity) + FALLBACK_WARNING;
    node.assert_stderr(&startup).await;

    reset(&node.provider, &hidden, 1).await.unwrap();
    assert_eq!(tempo_consensus_identity(&node.provider).await, identity);
    let after_reset = startup + FALLBACK_WARNING;
    node.assert_stderr(&after_reset).await;

    let error = reset(&node.provider, &hidden, 3).await.unwrap_err();
    assert_eq!(
        error.as_error_resp().unwrap().message,
        "expected Tempo consensus anchor epoch 0, got 1; set --tempo.epoch-length to match the fork source"
    );
    assert_eq!(tempo_consensus_identity(&node.provider).await, identity);
    node.assert_stderr(&after_reset).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_warning_suppression() {
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
    for (endpoint, flags, expected_length, quiet) in [
        (source.http_endpoint(), vec![], 3, false),
        (hidden.clone(), vec!["--tempo.epoch-length", "3"], 3, false),
        (hidden.clone(), vec!["--quiet"], 21_600, true),
    ] {
        let mut args = vec!["--fork-url", &endpoint, "--fork-block-number", "1"];
        args.extend(flags);
        let node = CliNode::spawn("tempo", &args).await;
        let identity = tempo_consensus_identity(&node.provider).await;
        assert_eq!(identity.epoch_length, expected_length);
        let expected = if quiet { String::new() } else { consensus_output(&identity) };
        node.assert_stderr(&expected).await;
        reset(&node.provider, &endpoint, 1).await.unwrap();
        node.assert_stderr(&expected).await;
    }

    let known = source.http_endpoint();
    let mirror = CliNode::spawn(
        "tempo",
        &["--fork-url", &hidden, "--fork-url", &known, "--fork-block-number", "1"],
    )
    .await;
    let identity = tempo_consensus_identity(&mirror.provider).await;
    assert_eq!(identity.epoch_length, 3);
    let expected = consensus_output(&identity);
    mirror.assert_stderr(&expected).await;
    mirror
        .provider
        .raw_request::<_, ()>(
            "anvil_reset".into(),
            serde_json::json!([{ "forking": { "blockNumber": 1 } }]),
        )
        .await
        .unwrap();
    mirror.assert_stderr(&expected).await;

    let local = CliNode::spawn("tempo", &[]).await;
    let identity = tempo_consensus_identity(&local.provider).await;
    assert_eq!(identity.epoch_length, 21_600);
    local.assert_stderr(&consensus_output(&identity)).await;
    let (source_api, source) = spawn(NodeConfig::test().with_no_mining(true)).await;
    source_api.mine_one().await.unwrap();
    let endpoint = source.http_endpoint();
    let ethereum =
        CliNode::spawn("ethereum", &["--fork-url", &endpoint, "--fork-block-number", "1"]).await;
    reset(&ethereum.provider, &endpoint, 1).await.unwrap();
    ethereum.assert_stderr("").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_late_epoch_metadata_does_not_warn() {
    let (source_api, source) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    source_api.mine_one().await.unwrap();
    let (endpoint, root_reads) =
        epoch_metadata::epoch_metadata_after_fork_root(source.http_endpoint(), 1).await;
    let node =
        CliNode::spawn("tempo", &["--fork-url", &endpoint, "--fork-block-number", "1"]).await;
    let identity = tempo_consensus_identity(&node.provider).await;
    assert_eq!(identity.epoch_length, 21_600);
    assert_eq!(root_reads.load(Ordering::Relaxed), 1);
    let expected = consensus_output(&identity);
    node.assert_stderr(&expected).await;

    let (endpoint, root_reads) =
        epoch_metadata::epoch_metadata_after_fork_root(source.http_endpoint(), 1).await;
    reset(&node.provider, &endpoint, 1).await.unwrap();
    assert_eq!(root_reads.load(Ordering::Relaxed), 1);
    node.assert_stderr(&expected).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_epoch_timeout_warning() {
    let (source_api, source) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
    )
    .await;
    source_api.mine_one().await.unwrap();
    let (endpoint, calls) = delayed_epoch_metadata(source.http_endpoint()).await;
    let node = CliNode::spawn(
        "tempo",
        &["--fork-url", &endpoint, "--fork-block-number", "1", "--no-fork-node-info"],
    )
    .await;
    let identity = tempo_consensus_identity(&node.provider).await;
    assert_eq!(identity.epoch_length, 21_600);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    node.assert_stderr(&(consensus_output(&identity) + FALLBACK_WARNING)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_consensus_fork_failed_startup_does_not_warn() {
    let (source_api, source) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    source_api.mine_one().await.unwrap();
    let hidden = spawn_rpc_proxy_rejecting_method_after(
        source.http_endpoint(),
        "anvil_tempoConsensusIdentity",
        0,
    )
    .await;
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let bind_error = TcpListener::bind(listener.local_addr().unwrap()).unwrap_err();
    let output = Command::new(anvil_binary())
        .args([
            "--network",
            "tempo",
            "--accounts",
            "0",
            "--host",
            "127.0.0.1",
            "--port",
            &listener.local_addr().unwrap().port().to_string(),
            "--fork-url",
            &hidden,
            "--fork-block-number",
            "1",
            "--no-storage-caching",
            "--color",
            "never",
        ])
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_data_eq!(String::from_utf8(output.stderr).unwrap(), format!("Error: {bind_error}\n"),);
}

struct CliNode {
    child: ChildGuard,
    provider: RetryProvider,
    stderr: PathBuf,
    _temp: tempfile::TempDir,
}

impl CliNode {
    async fn spawn(network: &str, args: &[&str]) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let temp = tempfile::tempdir().unwrap();
        let stderr = temp.path().join("stderr");
        let child = ChildGuard(
            Command::new(anvil_binary())
                .args([
                    "--network",
                    network,
                    "--accounts",
                    "0",
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                    "--no-mining",
                    "--color",
                    "never",
                ])
                .args(args)
                .args(args.contains(&"--fork-url").then_some("--no-storage-caching"))
                .env("RUST_LOG", "off")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(File::create(&stderr).unwrap())
                .spawn()
                .unwrap(),
        );
        let provider = http_provider(&format!("http://127.0.0.1:{port}"));
        let mut node = Self { child, provider, stderr, _temp: temp };
        tokio::time::timeout(Duration::from_secs(10), async {
            while node.provider.get_chain_id().await.is_err() {
                if let Some(status) = node.child.0.try_wait().unwrap() {
                    panic!("anvil exited with {status}: {}", read_to_string(&node.stderr).unwrap());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("anvil should finish startup");
        node
    }

    async fn assert_stderr(&self, expected: &str) {
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while read_to_string(&self.stderr).unwrap().len() < expected.len() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert_data_eq!(read_to_string(&self.stderr).unwrap(), expected);
    }
}

fn consensus_output(identity: &TempoConsensusIdentity) -> String {
    format!(
        "\nTempo Consensus\n==================\nIdentity:       {}\n\
         Proposer:       {}\nEpoch length:   {}\n",
        identity.identity, identity.proposer, identity.epoch_length,
    )
}

async fn reset(
    provider: &RetryProvider,
    endpoint: &str,
    block: u64,
) -> Result<(), alloy_transport::TransportError> {
    provider
        .raw_request(
            "anvil_reset".into(),
            serde_json::json!([{ "forking": { "jsonRpcUrl": endpoint, "blockNumber": block } }]),
        )
        .await
}

async fn delayed_epoch_metadata(endpoint: String) -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = reqwest::Client::new();
    let router = Router::new().route(
        "/",
        post({
            let calls = Arc::clone(&calls);
            move |Json(request): Json<serde_json::Value>| {
                let calls = Arc::clone(&calls);
                let client = client.clone();
                let endpoint = endpoint.clone();
                async move {
                    if request["method"] == "debug_chainConfig" {
                        calls.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(700)).await;
                        Json(serde_json::json!({
                            "jsonrpc": "2.0", "id": request["id"],
                            "result": { "epochLength": 3 },
                        }))
                    } else {
                        Json(
                            client
                                .post(endpoint)
                                .json(&request)
                                .send()
                                .await
                                .unwrap()
                                .json::<serde_json::Value>()
                                .await
                                .unwrap(),
                        )
                    }
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (endpoint, calls)
}
