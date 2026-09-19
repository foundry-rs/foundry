//! Same-binary comparisons with one isolated process and RPC session per attempt.

use crate::{EndpointArgs, RunArgs};
use alloy_primitives::B256;
use eyre::{Context, Result, bail, ensure};
use foundry_bench::results::RunnerMetadata;
use foundry_common::sh_println;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command};

pub mod capture;
pub mod proxy;
pub mod results;

const REPLAY_MARKER: &str = "Executing previous transactions from the block.";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u8,
    pub endpoint_label: String,
    pub chain_id: u64,
    pub client_version: String,
    pub source_context: Value,
    pub seed: u64,
    pub blocks: Vec<BlockInput>,
    pub cases: Vec<Case>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockInput {
    pub block_hash: B256,
    pub block_number: u64,
    pub parent_hash: B256,
    pub targets: Vec<TargetInput>,
    pub stratum: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetInput {
    pub id: String,
    pub tx_hash: B256,
    pub index: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub transaction_hash: B256,
    pub block_hash: B256,
    pub index: usize,
    pub positions: Vec<String>,
    pub stratum: String,
    pub expected_receipt_gas: Option<u64>,
    pub expected_receipt_status: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bal_response: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildIdentity {
    pub schema_version: u8,
    pub source_sha: String,
    pub cargo_lock_sha256: String,
    pub rustc: String,
    pub build_argv: Vec<String>,
    pub build_env: BTreeMap<String, String>,
    pub cast: Binary,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Binary {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    Auto,
    Miss,
    Replay,
}

impl Arm {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Miss => "miss",
            Self::Replay => "replay",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActualPath {
    BalHit,
    ReplayAfterProbe,
    ReplayNoProbe,
    Failed,
    Unknown,
}

pub fn classify(success: bool, stderr: &[u8], bal_requests: u64) -> ActualPath {
    if !success {
        return ActualPath::Failed;
    }
    if String::from_utf8_lossy(stderr).contains(REPLAY_MARKER) {
        if bal_requests == 0 { ActualPath::ReplayNoProbe } else { ActualPath::ReplayAfterProbe }
    } else if bal_requests > 0 {
        ActualPath::BalHit
    } else {
        ActualPath::Unknown
    }
}

pub fn schedule(seed: u64, round: usize, include_miss: bool) -> Vec<Arm> {
    if !include_miss {
        return if seed % 2 == (round % 2) as u64 {
            vec![Arm::Auto, Arm::Replay]
        } else {
            vec![Arm::Replay, Arm::Auto]
        };
    }
    const ORDERS: [[Arm; 3]; 6] = [
        [Arm::Auto, Arm::Miss, Arm::Replay],
        [Arm::Miss, Arm::Replay, Arm::Auto],
        [Arm::Replay, Arm::Auto, Arm::Miss],
        [Arm::Auto, Arm::Replay, Arm::Miss],
        [Arm::Replay, Arm::Miss, Arm::Auto],
        [Arm::Miss, Arm::Auto, Arm::Replay],
    ];
    ORDERS[((seed % 6) as usize + round % 6) % 6].to_vec()
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?).wrap_err_with(|| format!("invalid {}", path.display()))
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

pub fn new_output(path: &Path) -> Result<()> {
    ensure!(!path.exists(), "output directory already exists: {}", path.display());
    fs::create_dir_all(path.join("artifacts"))?;
    fs::create_dir_all(path.join("capture"))?;
    Ok(())
}

pub fn endpoint(args: &EndpointArgs) -> Result<String> {
    let endpoint = if let Some(name) = &args.rpc_env {
        env::var(name).wrap_err("the requested RPC environment variable is not set")?
    } else {
        let config =
            Config::load().map_err(|_| eyre::eyre!("cannot load existing Foundry config"))?;
        match config.get_rpc_url() {
            Some(value) => {
                value.map_err(|_| eyre::eyre!("configured RPC variable is unset"))?.into_owned()
            }
            None => foundry_test_utils::rpc::next_http_archive_rpc_url(),
        }
    };
    let url = reqwest::Url::parse(&endpoint).map_err(|_| eyre::eyre!("invalid RPC URL"))?;
    ensure!(matches!(url.scheme(), "http" | "https"), "only HTTP endpoints are supported");
    Ok(endpoint)
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    ensure!(manifest.schema_version == 1, "unsupported manifest version");
    ensure!(!manifest.cases.is_empty(), "manifest contains no cases");
    let mut ids = BTreeSet::new();
    for case in &manifest.cases {
        ensure!(safe_id(&case.id) && ids.insert(&case.id), "invalid or duplicate case id");
        ensure!(
            manifest.blocks.iter().any(|block| block.block_hash == case.block_hash
                && block
                    .targets
                    .iter()
                    .any(|target| (target.index, target.tx_hash)
                        == (case.index, case.transaction_hash))),
            "case does not match frozen block target: {}",
            case.id
        );
        policy(case, Arm::Auto)?;
    }
    Ok(())
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn policy(case: &Case, arm: Arm) -> Result<proxy::Policy> {
    if arm == Arm::Miss {
        return Ok(proxy::Policy::MethodNotFound);
    }
    if arm == Arm::Replay {
        return Ok(proxy::Policy::Passthrough);
    }
    Ok(match case.fault.as_deref() {
        None => {
            case.bal_response.clone().map_or(proxy::Policy::Passthrough, proxy::Policy::Recorded)
        }
        Some("method_not_found") => proxy::Policy::MethodNotFound,
        Some("null") => proxy::Policy::Null,
        Some("delayed_success") => {
            proxy::Policy::Delay { millis: 750, result: case.bal_response.clone() }
        }
        Some("malformed" | "missing_slot") => proxy::Policy::Recorded(
            case.bal_response.clone().ok_or_else(|| eyre::eyre!("fault requires recorded BAL"))?,
        ),
        Some(_) => bail!("unsupported BAL fault"),
    })
}

fn child_command(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command.env_clear();
    for key in ["PATH", "HOME", "TMPDIR", "SYSTEMROOT", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    command.current_dir(root).envs([
        ("FOUNDRY_CONFIG", "foundry.toml"),
        ("FOUNDRY_PROFILE", "default"),
        ("FOUNDRY_NO_STORAGE_CACHING", "true"),
        ("FOUNDRY_DISABLE_NIGHTLY_WARNING", "true"),
        ("NO_COLOR", "1"),
        ("CLICOLOR", "0"),
        ("TERM", "dumb"),
        ("RUST_LOG", "off"),
    ]);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
}

pub fn config_check() -> Result<()> {
    let config = Config::load().map_err(|_| eyre::eyre!("configuration preflight failed"))?;
    let figment = Config::figment();
    // EvmOpts consumes these directly from Figment, outside the Config field set.
    for key in ["fork_retries", "fork_retry_backoff", "fork_headers", "compute_units_per_second"] {
        ensure!(
            figment.extract_inner::<Value>(key).map_or(true, |value| value.is_null()),
            "non-default {key} affects BAL experiment"
        );
    }
    sh_println!("{}", checked_config(&config)?)?;
    Ok(())
}

fn checked_config(config: &Config) -> Result<Value> {
    let current = serde_json::to_value(config)?;
    let defaults = serde_json::to_value(Config::default())?;
    let mut settings = serde_json::Map::new();
    for key in [
        "hardfork",
        "evm_version",
        "chain_id",
        "verbosity",
        "tracing",
        "memory_limit",
        "gas_price",
        "gas_limit",
        "block_gas_limit",
        "create2_deployer",
        "no_rpc_rate_limit",
        "eth_rpc_timeout",
        "eth_rpc_accept_invalid_certs",
        "eth_rpc_no_proxy",
        "eth_rpc_curl",
    ] {
        ensure!(current.get(key) == defaults.get(key), "non-default {key} affects BAL experiment");
        settings.insert(key.into(), current.get(key).cloned().unwrap_or(Value::Null));
    }
    ensure!(
        config.networks == Config::default().networks,
        "network overrides affect BAL experiment"
    );
    ensure!(config.eth_rpc_jwt.is_none(), "eth_rpc_jwt affects BAL experiment");
    ensure!(config.eth_rpc_headers.is_none(), "eth_rpc_headers affects BAL experiment");
    ensure!(config.no_storage_caching, "storage cache must be disabled");
    settings.insert("networks".into(), serde_json::to_value(config.networks)?);
    settings.insert("no_storage_caching".into(), json!(true));
    settings.insert("fresh_process".into(), json!(true));
    settings.insert("external_identification".into(), json!(false));
    Ok(Value::Object(settings))
}

struct ChildOutput {
    status: Option<i32>,
    success: bool,
    timed_out: bool,
    elapsed: f64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    rpc_at_exit: Option<proxy::Snapshot>,
}

#[cfg(unix)]
struct ProcessGroup(u32);

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Each benchmark child owns its group; also stop descendants retaining pipe handles.
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

struct OutputReaders {
    stdout: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
}

impl Drop for OutputReaders {
    fn drop(&mut self) {
        self.stdout.abort();
        self.stderr.abort();
    }
}

async fn execute(
    mut command: Command,
    timeout: Duration,
    proxy: Option<&proxy::Proxy>,
) -> Result<ChildOutput> {
    let start = Instant::now();
    let mut child = command.spawn().wrap_err("could not start benchmark child")?;
    #[cfg(unix)]
    let _group = ProcessGroup(child.id().expect("spawned child id"));
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out = tokio::spawn(async move {
        let mut b = Vec::new();
        stdout.read_to_end(&mut b).await.map(|_| b)
    });
    let err = tokio::spawn(async move {
        let mut b = Vec::new();
        stderr.read_to_end(&mut b).await.map(|_| b)
    });
    let mut readers = OutputReaders { stdout: out, stderr: err };
    let waited = tokio::select! {
        result = tokio::time::timeout(timeout, child.wait()) => result,
        _ = tokio::signal::ctrl_c() => { child.kill().await.ok(); bail!(std::io::Error::new(std::io::ErrorKind::Interrupted, "benchmark interrupted")); }
    };
    let elapsed = start.elapsed().as_secs_f64();
    let timed_out = waited.is_err();
    let status = match waited {
        Ok(status) => status?,
        Err(_) => {
            #[cfg(unix)]
            if let Some(id) = child.id() {
                // The child owns its process group; terminate helpers together on timeout.
                unsafe {
                    libc::kill(-(id as i32), libc::SIGKILL);
                }
            }
            child.kill().await.ok();
            child.wait().await?
        }
    };
    let rpc_at_exit = proxy.map(proxy::Proxy::end_measurement);
    let stdout = tokio::time::timeout(Duration::from_secs(3), &mut readers.stdout)
        .await
        .wrap_err("stdout drain timeout")???;
    let stderr = tokio::time::timeout(Duration::from_secs(3), &mut readers.stderr)
        .await
        .wrap_err("stderr drain timeout")???;
    Ok(ChildOutput {
        status: status.code(),
        success: status.success() && !timed_out,
        timed_out,
        elapsed,
        stdout,
        stderr,
        rpc_at_exit,
    })
}

async fn check_binary(args: &RunArgs) -> Result<(Binary, Option<BuildIdentity>)> {
    let path = fs::canonicalize(&args.cast)?;
    let sha256 = digest(&fs::read(&path)?);
    let version = Command::new(&path)
        .arg("--version")
        .env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "true")
        .output()
        .await?;
    ensure!(version.status.success(), "could not read Cast version");
    let version = String::from_utf8(version.stdout)?.trim().to_owned();
    let help = Command::new(&path).args(["run", "--help"]).output().await?;
    ensure!(
        help.status.success() && String::from_utf8_lossy(&help.stdout).contains("--no-bal"),
        "Cast must support run --no-bal (PR #16931 or later); no source patches are applied"
    );
    let binary = Binary { path, sha256, version };
    let build = args.build_manifest.as_deref().map(read_json::<BuildIdentity>).transpose()?;
    if let Some(build) = &build {
        ensure!(
            build.schema_version == 1
                && build.source_sha.len() == 40
                && build.source_sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid build source identity"
        );
        ensure!(
            build.build_argv.iter().any(|a| a == "--locked")
                && build.build_argv.windows(2).any(|a| a == ["--profile", "profiling"]),
            "expected a locked profiling build"
        );
        ensure!(binary.sha256 == build.cast.sha256, "binary hash differs from build manifest");
        ensure!(
            binary.version == build.cast.version.trim(),
            "binary version differs from build manifest"
        );
    }
    Ok((binary, build))
}

/// State retained for one binary across validation, warmup and measured rounds.
struct PreparedRun {
    label: &'static str,
    args: RunArgs,
    binary: Binary,
    root: tempfile::TempDir,
    // A missing entry means uncaptured; None means validation or the oracle failed.
    oracles: BTreeMap<String, Option<String>>,
}

fn rounds(args: &RunArgs) -> impl Iterator<Item = (&'static str, usize)> + '_ {
    [("warmup", args.warmup_rounds), ("measured", args.rounds)]
        .into_iter()
        .flat_map(|(phase, count)| (0..count).map(move |round| (phase, round)))
}

pub async fn run(args: RunArgs) -> Result<()> {
    ensure!(args.timeout_seconds > 0 && args.rounds > 0, "invalid rounds or timeout");
    let manifest: Manifest = read_json(&args.manifest)?;
    validate_manifest(&manifest)?;
    let upstream = endpoint(&args.endpoint)?;
    let mut refs = Vec::new();
    if let Some(cast) = &args.baseline_cast {
        let mut base = args.clone();
        base.cast = cast.clone();
        base.build_manifest = args.baseline_build_manifest.clone();
        base.output_dir = args.output_dir.join("base/aggregate");
        refs.push(("base", base));
        let mut candidate = args.clone();
        candidate.output_dir = args.output_dir.join("candidate/aggregate");
        refs.push(("candidate", candidate));
        new_output(&args.output_dir)?;
    } else {
        refs.push(("candidate", args.clone()));
    }
    let manifest_hash = digest(&fs::read(&args.manifest)?);
    let runner = RunnerMetadata::default();
    let schedule_manifest = rounds(&args)
        .map(|(phase, round)| json!({"phase":phase,"round":round,"arms":schedule(manifest.seed,round,args.include_miss)}))
        .collect::<Vec<_>>();
    let mut runs = Vec::new();
    for (label, args) in refs {
        let (binary, build) = check_binary(&args).await?;
        new_output(&args.output_dir)?;
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("foundry.toml"),
            "[profile.default]\nno_storage_caching = true\n",
        )?;
        let mut check = child_command(&env::current_exe()?, root.path());
        check.arg("config-check");
        let config = execute(check, Duration::from_secs(30), None).await?;
        ensure!(
            config.success,
            "effective config preflight failed: {}",
            String::from_utf8_lossy(&config.stderr)
        );
        let effective_config: Value = serde_json::from_slice(&config.stdout)?;
        let run_manifest = json!({"schema_version":1, "panel":manifest,"panel_sha256":manifest_hash,
            "runner":runner,
            "build":build,"binary":binary,"effective_config":effective_config,"rounds":args.rounds,"warmup_rounds":args.warmup_rounds,
            "timeout_seconds":args.timeout_seconds,"server_bal_source":"unknown",
            "include_miss":args.include_miss,"worker_count":1,"schedule":schedule_manifest});
        write_json(&args.output_dir.join("manifest.json"), &run_manifest)?;
        runs.push(PreparedRun { label, args, binary, root, oracles: BTreeMap::new() });
    }
    let rpc = capture::Rpc::new(&upstream)?;
    ensure!(
        capture::quantity(&rpc.call("eth_chainId", json!([])).await?)? == manifest.chain_id,
        "endpoint chain differs from captured manifest"
    );
    for block in &manifest.blocks {
        let current = rpc
            .call("eth_getBlockByNumber", json!([format!("0x{:x}", block.block_number), false]))
            .await?;
        ensure!(
            current["hash"] == json!(block.block_hash)
                && current["parentHash"] == json!(block.parent_hash),
            "captured block is no longer canonical"
        );
        for target in &block.targets {
            ensure!(
                current["transactions"].get(target.index) == Some(&json!(target.tx_hash)),
                "captured target is not at its recorded block index: {}",
                target.id
            );
        }
    }
    // Validate each case once per binary before any warmup or measured rounds.
    for run in &mut runs {
        for case in &manifest.cases {
            if case.capture_error.is_some()
                || case.expected_receipt_gas.is_none()
                || case.expected_receipt_status.is_none()
            {
                record_uncaptured(&run.args, &manifest, case)?;
                continue;
            }
            let oracle = validate_case(
                &run.args,
                &upstream,
                run.root.path(),
                &run.binary.path,
                case,
                manifest.seed,
            )
            .await?;
            run.oracles.insert(case.id.clone(), oracle);
        }
    }
    let mut execution_order = Vec::new();
    let schedule_path = args.output_dir.join("schedule.json");
    write_json(&schedule_path, &json!({"schema_version":1,"execution_order":execution_order}))?;
    for (phase, round) in rounds(&args) {
        // Reverse whole ref rounds, while each ref retains the same arm schedule.
        let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for index in order.into_iter().filter(|index| *index < runs.len()) {
            let run = &runs[index];
            execution_order.push(json!({"ref":run.label,"round":round,"phase":phase}));
            write_json(
                &schedule_path,
                &json!({"schema_version":1,"execution_order":execution_order}),
            )?;
            for case in &manifest.cases {
                if let Some(oracle) = run.oracles.get(&case.id) {
                    for (order, arm) in
                        schedule(manifest.seed, round, args.include_miss).into_iter().enumerate()
                    {
                        let mut sample = attempt(
                            &run.args,
                            &upstream,
                            run.root.path(),
                            &run.binary.path,
                            case,
                            arm,
                            phase,
                            round,
                            order,
                            oracle.as_deref(),
                        )
                        .await?;
                        if oracle.is_none() && sample.correctness != "invalid_path" {
                            sample.correctness = "correctness_blocked".into();
                        }
                        append_sample(&run.args.output_dir, &sample)?;
                    }
                }
            }
        }
    }
    for run in &runs {
        results::report(&run.args.output_dir)?;
        sh_println!("BAL artifacts: {}", run.args.output_dir.display())?;
    }
    Ok(())
}

async fn validate_case(
    args: &RunArgs,
    upstream: &str,
    root: &Path,
    cast: &Path,
    case: &Case,
    seed: u64,
) -> Result<Option<String>> {
    let mut validation = BTreeMap::new();
    for arm in schedule(seed, 0, args.include_miss) {
        let sample =
            attempt(args, upstream, root, cast, case, arm, "validation", 0, 0, None).await?;
        validation.insert(arm, sample);
    }
    let oracle = &validation[&Arm::Replay];
    let equivalent = validation.values().all(|s| {
        s.correctness == "unchecked"
            && s.stdout_sha256 == oracle.stdout_sha256
            && s.local_gas == case.expected_receipt_gas
            && s.execution_success == case.expected_receipt_status
    });
    let mut default_oracle = None;
    if equivalent {
        let baseline =
            attempt(args, upstream, root, cast, case, Arm::Replay, "oracle", 0, 0, None).await?;
        if baseline.correctness == "unchecked"
            && baseline.local_gas == case.expected_receipt_gas
            && baseline.execution_success == case.expected_receipt_status
        {
            default_oracle = Some(baseline.stdout_sha256.clone());
        }
        append_sample(&args.output_dir, &baseline)?;
    }
    for mut sample in validation.into_values() {
        if sample.correctness != "invalid_path" {
            sample.correctness =
                if equivalent { "equivalent" } else { "correctness_blocked" }.into();
        }
        append_sample(&args.output_dir, &sample)?;
    }
    Ok(default_oracle)
}

#[allow(clippy::too_many_arguments)]
async fn attempt(
    args: &RunArgs,
    upstream: &str,
    root: &Path,
    cast: &Path,
    case: &Case,
    arm: Arm,
    phase: &str,
    round: usize,
    order: usize,
    expected_hash: Option<&str>,
) -> Result<results::Sample> {
    let proxy = proxy::Proxy::start(upstream, policy(case, arm)?).await?;
    let id = format!("{}-{phase}-{round}-{order}-{}", case.id, arm.name());
    let mut command = child_command(cast, root);
    command.args([
        "run",
        &case.transaction_hash.to_string(),
        "--rpc-url",
        proxy.url(),
        "--disable-external-identification",
    ]);
    if arm == Arm::Replay {
        command.arg("--no-bal");
    }
    if phase == "validation" {
        command.arg("-vvvvv");
    }
    let process_start = Instant::now();
    let output = execute(command, Duration::from_secs(args.timeout_seconds), Some(&proxy)).await;
    let observed_duration = process_start.elapsed().as_secs_f64();
    let at_exit = proxy.snapshot();
    let session = proxy.finish().await;
    if let Err(error) = &output
        && error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::Interrupted)
    {
        return Err(output.err().expect("interrupted process error"));
    }
    let output = output.unwrap_or_else(|_| ChildOutput {
        status: None,
        success: false,
        timed_out: false,
        elapsed: observed_duration,
        stdout: Vec::new(),
        stderr: b"benchmark child spawn or output collection failed\n".to_vec(),
        rpc_at_exit: None,
    });
    let at_exit = output.rpc_at_exit.unwrap_or(at_exit);
    fs::write(args.output_dir.join("artifacts").join(format!("{id}.stdout")), &output.stdout)?;
    fs::write(args.output_dir.join("artifacts").join(format!("{id}.stderr")), &output.stderr)?;
    let bal_count = at_exit
        .client_requests_by_method
        .iter()
        .filter(|(m, _)| proxy::is_bal_method(m))
        .map(|(_, n)| *n)
        .sum();
    let actual_path = classify(output.success, &output.stderr, bal_count);
    let stdout_sha256 = digest(&output.stdout);
    let path_valid = actual_path != ActualPath::Unknown
        && actual_path != ActualPath::Failed
        && (arm != Arm::Replay || (bal_count == 0 && actual_path == ActualPath::ReplayNoProbe))
        // Before Cancun, Cast skips the BAL probe even with the injected-miss policy.
        && (arm != Arm::Miss
            || matches!(actual_path, ActualPath::ReplayAfterProbe | ActualPath::ReplayNoProbe))
        // An unsupported injection must not become an ordinary miss if Cast replays successfully.
        && !session.events.iter().any(|event| {
            event.issues.iter().any(|issue| issue == "unsupported_bal_batch_injection")
        });
    let (local_gas, execution_success) = parse_output(&output.stdout, &output.stderr);
    let correctness = if !path_valid {
        "invalid_path"
    } else if expected_hash.is_some_and(|hash| hash != stdout_sha256)
        || local_gas != case.expected_receipt_gas
        || execution_success != case.expected_receipt_status
    {
        "correctness_blocked"
    } else if expected_hash.is_some() {
        "equivalent"
    } else {
        "unchecked"
    };
    for event in &session.events {
        append_json(
            &args.output_dir.join("rpc-events.jsonl"),
            &json!({"sample_id":id,"phase":phase,"event":event}),
        )?;
    }
    Ok(results::Sample {
        schema_version: 1,
        id,
        case_id: case.id.clone(),
        phase: phase.into(),
        round,
        order,
        arm,
        actual_path,
        exit_code: output.status,
        timed_out: output.timed_out,
        wall_time_seconds: (!output.timed_out && output.status.is_some()).then_some(output.elapsed),
        observed_duration_seconds: output.elapsed,
        stdout_sha256,
        local_gas,
        execution_success,
        correctness: correctness.into(),
        fault: case.fault.clone(),
        synthetic: arm == Arm::Miss || case.bal_response.is_some() || case.fault.is_some(),
        rpc_at_exit: at_exit,
        rpc: session.snapshot,
        bal_events: session
            .events
            .into_iter()
            .filter(|event| event.calls.iter().any(|call| proxy::is_bal_method(&call.method)))
            .collect(),
    })
}

fn parse_output(stdout: &[u8], stderr: &[u8]) -> (Option<u64>, Option<bool>) {
    let text = String::from_utf8_lossy(stdout);
    let gas = text.lines().find_map(|line| line.strip_prefix("Gas used: ")?.trim().parse().ok());
    let success = if String::from_utf8_lossy(stderr)
        .lines()
        .any(|line| matches!(line, "Error: Transaction failed." | "Transaction failed."))
    {
        Some(false)
    } else {
        // Cast prints the result after traces, which can contain arbitrary revert strings.
        text.lines().rev().find_map(|line| match line {
            "Transaction successfully executed." => Some(true),
            "Transaction failed." => Some(false),
            _ => None,
        })
    };
    (gas, success)
}

fn record_uncaptured(args: &RunArgs, manifest: &Manifest, case: &Case) -> Result<()> {
    for (phase, round) in rounds(args) {
        for (order, arm) in
            schedule(manifest.seed, round, args.include_miss).into_iter().enumerate()
        {
            append_sample(
                &args.output_dir,
                &results::Sample {
                    schema_version: 1,
                    id: format!("{}-{phase}-{round}-{order}-{}", case.id, arm.name()),
                    case_id: case.id.clone(),
                    phase: phase.into(),
                    round,
                    order,
                    arm,
                    actual_path: ActualPath::Failed,
                    exit_code: None,
                    timed_out: false,
                    wall_time_seconds: None,
                    observed_duration_seconds: 0.0,
                    stdout_sha256: digest(b""),
                    local_gas: None,
                    execution_success: None,
                    correctness: "capture_incomplete".into(),
                    fault: case.fault.clone(),
                    synthetic: arm == Arm::Miss
                        || case.bal_response.is_some()
                        || case.fault.is_some(),
                    rpc_at_exit: proxy::Snapshot::default(),
                    rpc: proxy::Snapshot::default(),
                    bal_events: Vec::new(),
                },
            )?;
        }
    }
    Ok(())
}

fn append_sample(output: &Path, sample: &results::Sample) -> Result<()> {
    append_json(&output.join("samples.jsonl"), sample)
}

pub fn append_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ActualPath, Arm, Case, Manifest, REPLAY_MARKER, checked_config, classify, parse_output,
        record_uncaptured, results::Sample, schedule,
    };
    use crate::{EndpointArgs, RunArgs};
    use alloy_primitives::B256;
    use foundry_config::Config;
    use serde_json::json;
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
    };

    #[cfg(unix)]
    use super::{
        child_command, execute,
        proxy::{Policy, Proxy},
    };
    #[cfg(unix)]
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    #[test]
    fn replay_marker_disambiguates_successful_bal_response_and_first_transaction() {
        assert_eq!(classify(true, REPLAY_MARKER.as_bytes(), 1), ActualPath::ReplayAfterProbe);
        assert_eq!(classify(true, REPLAY_MARKER.as_bytes(), 0), ActualPath::ReplayNoProbe);
        assert_eq!(classify(true, b"", 0), ActualPath::Unknown);
        assert_eq!(classify(true, b"", 1), ActualPath::BalHit);
        assert_eq!(classify(false, b"", 1), ActualPath::Failed);
    }

    #[test]
    fn six_rounds_balance_every_arm_in_every_position() {
        let mut counts = BTreeMap::new();
        for round in 0..6 {
            for (position, arm) in schedule(7928, round, true).into_iter().enumerate() {
                *counts.entry((position, arm)).or_insert(0) += 1;
            }
        }
        for position in 0..3 {
            for arm in [Arm::Auto, Arm::Miss, Arm::Replay] {
                assert_eq!(counts[&(position, arm)], 2);
            }
        }
    }

    #[test]
    fn default_comparison_alternates_the_same_binary_controls() {
        assert_eq!(schedule(7928, 0, false), [Arm::Auto, Arm::Replay]);
        assert_eq!(schedule(7928, 1, false), [Arm::Replay, Arm::Auto]);
        assert_eq!(schedule(7929, 0, false), [Arm::Replay, Arm::Auto]);
    }

    #[test]
    fn gas_and_revert_are_semantic_output() {
        assert_eq!(
            parse_output(b"Traces:\nTransaction successfully executed.\nGas used: 21000\n", b""),
            (Some(21000), Some(true))
        );
        assert_eq!(
            parse_output(b"Gas used: 30000\n", b"Transaction failed.\n"),
            (Some(30000), Some(false))
        );
    }

    #[test]
    fn revert_reason_is_not_execution_success() {
        let stdout = b"Traces:\n  [0] Contract::run()\n    [Revert] Transaction successfully executed.\n\nGas used: 30000\n";
        assert_eq!(
            parse_output(stdout, b"Error: Transaction failed.\n"),
            (Some(30000), Some(false))
        );
        assert_eq!(parse_output(stdout, b""), (Some(30000), None));
    }

    #[test]
    fn execution_failure_takes_precedence_over_success_text() {
        let stdout = b"Transaction successfully executed.\nGas used: 30000\n";
        for stderr in [b"Error: Transaction failed.\n".as_slice(), b"Transaction failed.\n"] {
            assert_eq!(parse_output(stdout, stderr), (Some(30000), Some(false)));
        }
    }

    #[test]
    fn execution_result_follows_multiline_revert_reasons() {
        let stdout = b"Traces:\n    [Revert] misleading reason:\nTransaction successfully executed.\n\nGas used: 30000\n";
        assert_eq!(
            parse_output(stdout, b"Error: Transaction failed.\n"),
            (Some(30000), Some(false))
        );
        let stdout = b"Traces:\n    [Revert] caught inner revert:\nTransaction failed.\n\nTransaction successfully executed.\nGas used: 30000\n";
        assert_eq!(parse_output(stdout, b""), (Some(30000), Some(true)));
        let stdout = b"Traces:\n    [Revert] misleading reason:\nTransaction successfully executed.\n\nTransaction failed.\nGas used: 30000\n";
        assert_eq!(parse_output(stdout, b""), (Some(30000), Some(false)));
    }

    #[test]
    fn execution_status_requires_a_complete_result_line() {
        for text in [
            b"    [Revert] Transaction failed.\n".as_slice(),
            b"    [Return] Transaction successfully executed.\n",
            b"Warning: Transaction failed. Retrying.\n",
            b"Transaction successfully executed. extra text\n",
        ] {
            assert_eq!(parse_output(text, b""), (None, None));
            assert_eq!(parse_output(b"", text), (None, None));
        }
        assert_eq!(parse_output(b"Transaction failed.\n", b""), (None, Some(false)));
    }

    #[test]
    fn preflight_rejects_rpc_and_execution_overrides_without_recording_secrets() {
        let config = Config {
            no_storage_caching: true,
            eth_rpc_url: Some("https://provider.invalid/private-token".into()),
            etherscan_api_key: Some("private-etherscan-key".into()),
            ..Default::default()
        };
        let settings = checked_config(&config).unwrap();
        assert_eq!(settings["memory_limit"], config.memory_limit);
        assert_eq!(settings["eth_rpc_timeout"], serde_json::Value::Null);
        assert_eq!(settings["no_rpc_rate_limit"], false);
        assert!(!settings.to_string().contains("private-"));
        for (key, value) in [
            ("memory_limit", json!(1024)),
            ("eth_rpc_timeout", json!(7)),
            ("no_rpc_rate_limit", json!(true)),
            ("eth_rpc_headers", json!(["Authorization: private-header-key"])),
            ("eth_rpc_jwt", json!("private-jwt-key")),
        ] {
            let mut value_config = serde_json::to_value(&config).unwrap();
            value_config[key] = value;
            let changed = serde_json::from_value::<Config>(value_config).unwrap();
            let error = checked_config(&changed).unwrap_err().to_string();
            assert!(error.contains(key));
            assert!(!error.contains("private-"));
        }
    }

    #[test]
    fn capture_failure_preserves_every_scheduled_arm_and_round_without_fake_timings() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = RunArgs {
            endpoint: EndpointArgs { rpc_env: None },
            manifest: directory.path().join("panel.json"),
            build_manifest: None,
            cast: directory.path().join("cast"),
            baseline_cast: None,
            baseline_build_manifest: None,
            include_miss: true,
            output_dir: directory.path().to_owned(),
            rounds: 10,
            warmup_rounds: 2,
            timeout_seconds: 30,
        };
        let case = Case {
            id: "missing-receipt".into(),
            transaction_hash: B256::with_last_byte(1),
            block_hash: B256::with_last_byte(2),
            index: 0,
            positions: vec!["first".into()],
            stratum: "fixture".into(),
            expected_receipt_gas: None,
            expected_receipt_status: None,
            capture_error: Some("receipt_unavailable".into()),
            bal_response: None,
            fault: None,
        };
        let manifest = Manifest {
            schema_version: 1,
            endpoint_label: "fixture".into(),
            chain_id: 1,
            client_version: "fixture".into(),
            source_context: json!({}),
            seed: 3,
            blocks: Vec::new(),
            cases: vec![case.clone()],
        };
        record_uncaptured(&args, &manifest, &case).unwrap();
        let path = directory.path().join("samples.jsonl");
        let samples = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Sample>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(samples.len(), 36);
        assert_eq!(samples.iter().map(|sample| &sample.id).collect::<BTreeSet<_>>().len(), 36);
        for arm in [Arm::Auto, Arm::Miss, Arm::Replay] {
            let measured = samples
                .iter()
                .filter(|sample| sample.phase == "measured" && sample.arm == arm)
                .collect::<Vec<_>>();
            assert_eq!(measured.len(), 10);
            assert_eq!(
                measured.iter().map(|sample| sample.round).collect::<BTreeSet<_>>(),
                (0..10).collect()
            );
            assert_eq!(
                samples
                    .iter()
                    .filter(|sample| sample.phase == "warmup" && sample.arm == arm)
                    .count(),
                2
            );
        }
        for sample in &samples {
            assert_eq!(sample.actual_path, ActualPath::Failed);
            assert_eq!(sample.correctness, "capture_incomplete");
            assert_eq!(sample.synthetic, sample.arm == Arm::Miss);
            assert!(sample.exit_code.is_none() && sample.wall_time_seconds.is_none());
            assert!(sample.local_gas.is_none() && sample.execution_success.is_none());
            assert!(!sample.timed_out);
        }
        fs::remove_file(path).unwrap();
        args.warmup_rounds = 0;
        args.rounds = 2;
        args.include_miss = false;
        record_uncaptured(&args, &manifest, &case).unwrap();
        let raw = fs::read_to_string(directory.path().join("samples.jsonl")).unwrap();
        let measured = raw
            .lines()
            .map(|line| serde_json::from_str::<Sample>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(measured.len(), 4);
        assert!(
            measured.iter().all(|sample| sample.phase == "measured" && sample.arm != Arm::Miss)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_environment_is_restricted_and_keeps_disk_cache_disabled() {
        let root = tempfile::tempdir().unwrap();
        let output = execute(
            child_command(Path::new("/usr/bin/env"), root.path()),
            Duration::from_secs(10),
            None,
        )
        .await
        .unwrap();
        assert!(output.success);
        let text = String::from_utf8(output.stdout).unwrap();
        let values =
            text.lines().map(|line| line.split_once('=').unwrap()).collect::<BTreeMap<_, _>>();
        assert_eq!(values["FOUNDRY_NO_STORAGE_CACHING"], "true");
        assert_eq!(values["FOUNDRY_PROFILE"], "default");
        assert_eq!(values["RUST_LOG"], "off");
        assert_eq!(values["NO_COLOR"], "1");
        for key in values.keys() {
            assert!(
                matches!(
                    *key,
                    "PATH"
                        | "HOME"
                        | "TMPDIR"
                        | "SYSTEMROOT"
                        | "SSL_CERT_FILE"
                        | "SSL_CERT_DIR"
                        | "FOUNDRY_CONFIG"
                        | "FOUNDRY_PROFILE"
                        | "FOUNDRY_NO_STORAGE_CACHING"
                        | "FOUNDRY_DISABLE_NIGHTLY_WARNING"
                        | "NO_COLOR"
                        | "CLICOLOR"
                        | "TERM"
                        | "RUST_LOG"
                ),
                "unexpected inherited environment key: {key}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_drains_both_pipes_larger_than_pipe_capacity() {
        let root = tempfile::tempdir().unwrap();
        let mut command = child_command(Path::new("/bin/sh"), root.path());
        command.args(["-c", "dd if=/dev/zero bs=65536 count=2 2>/dev/null; dd if=/dev/zero bs=65536 count=2 1>&2 2>/dev/null"]);
        let output = execute(command, Duration::from_secs(10), None).await.unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, vec![0; 131072]);
        assert_eq!(output.stderr, vec![0; 131072]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_timeout_terminates_descendants_and_preserves_partial_output() {
        let root = tempfile::tempdir().unwrap();
        let mut command = child_command(Path::new("/bin/sh"), root.path());
        command
            .args(["-c", "(sleep 1; printf leaked > descendant-survived) & printf started; wait"]);
        let output = execute(command, Duration::from_millis(300), None).await.unwrap();
        assert!(output.timed_out);
        assert!(!output.success);
        assert_eq!(output.stdout, b"started");
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!root.path().join("descendant-survived").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inherited_pipe_deadline_cleans_up_successful_child_descendants() {
        let root = tempfile::tempdir().unwrap();
        let mut command = child_command(Path::new("/bin/sh"), root.path());
        command.args([
            "-c",
            "(sleep 4; printf leaked > descendant-survived) & printf started; exit 0",
        ]);
        let started = Instant::now();
        let result = execute(command, Duration::from_secs(10), None).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("stdout drain timeout"));
        assert!(started.elapsed() < Duration::from_secs(8));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!root.path().join("descendant-survived").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_exit_rpc_snapshot_does_not_leak_into_next_proxy_session() {
        let root = tempfile::tempdir().unwrap();
        let proxy = Proxy::start("http://127.0.0.1:9", Policy::MethodNotFound).await.unwrap();
        reqwest::Client::new()
            .post(proxy.url())
            .json(&json!({
                "jsonrpc":"2.0", "id":1, "method":"eth_getBlockAccessListByBlockHash", "params":[]
            }))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let mut command = child_command(Path::new("/bin/sh"), root.path());
        command.args(["-c", "printf completed"]);
        let output = execute(command, Duration::from_secs(10), Some(&proxy)).await.unwrap();
        assert!(output.success);
        assert_eq!(
            output.rpc_at_exit.unwrap().client_requests_by_method["eth_getBlockAccessListByBlockHash"],
            1
        );
        let session = proxy.finish().await;
        assert_eq!(session.snapshot.client_http_exchanges, 1);
        let next = Proxy::start("http://127.0.0.1:9", Policy::MethodNotFound).await.unwrap();
        let mut command = child_command(Path::new("/bin/sh"), root.path());
        command.args(["-c", "printf next"]);
        let output = execute(command, Duration::from_secs(10), Some(&next)).await.unwrap();
        assert!(output.rpc_at_exit.unwrap().client_requests_by_method.is_empty());
        assert_eq!(next.finish().await.snapshot.client_http_exchanges, 0);
    }
}
