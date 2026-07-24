//! Acquisition, compilation, and matching of external Solidity sources.

use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Constructor;
use alloy_primitives::{Address, Bytes, keccak256};
use eyre::{Result, bail, eyre};
use forge_verify::sourcify::SOURCIFY_URL;
use foundry_compilers::{artifacts::CompilerOutput, solc::Solc};
use foundry_config::{Chain, NamedChain};
use futures::StreamExt;
use semver::Version;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const MAX_INPUT_SIZE: usize = 10 * 1024 * 1024;
const MAX_SOURCES: usize = 256;
const MAX_FETCHES: usize = 64;
const MAX_COMPILER_VERSIONS: usize = 8;
const MAX_COMPILATIONS: usize = 32;
const MAX_CANDIDATES: usize = 512;
const MAX_CREATION_BYTECODE: usize = 64 * 1024 * 1024;
const MAX_RETAINED_METADATA: usize = 8 * 1024 * 1024;
const MAX_RETAINED_SOURCE_INPUT: usize = 64 * 1024 * 1024;
const MAX_SOURCE_PATH: usize = 4096;
const MAX_COMPILER_VERSION: usize = 256;
const MAX_CACHED_ERROR_CHARS: usize = 512;
const MAX_STDOUT: usize = 32 * 1024 * 1024;
const MAX_STDERR: usize = 1024 * 1024;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MAX_PROVENANCE_ADDRESSES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum SourceProvider {
    Etherscan { endpoint: String },
    Sourcify { endpoint: String },
}

impl SourceProvider {
    const fn metadata_len(&self) -> usize {
        match self {
            Self::Etherscan { endpoint } | Self::Sourcify { endpoint } => endpoint.len(),
        }
    }
}

impl std::fmt::Display for SourceProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Etherscan { .. } => f.write_str("Etherscan"),
            Self::Sourcify { .. } => f.write_str("Sourcify"),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ExternalSource {
    pub input: Arc<Value>,
    pub version: Version,
    pub provider: SourceProvider,
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub input: Arc<Value>,
    pub fingerprint: String,
    pub version: Version,
    pub fqn: String,
    pub constructor: Option<Constructor>,
    pub creation_bytecode: Bytes,
}

#[derive(Clone, Debug)]
pub(super) struct ExternalMatch {
    pub input: Arc<Value>,
    pub version: Version,
    pub fqn: String,
    pub creation_bytecode: Bytes,
    pub constructor_args: Bytes,
}

#[derive(Clone, Debug)]
pub(super) enum MatchResult {
    None,
    Unique(ExternalMatch),
    Ambiguous(Vec<ExternalMatch>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FetchKey {
    chain: u64,
    provider: SourceProvider,
    address: Address,
}

type CompiledCandidates = Result<(Arc<[Candidate]>, bool), String>;

/// Per-script-run state. Both caches deliberately retain failures, preventing repeated network or
/// compiler work for the same provenance.
pub(super) struct ExternalResolver {
    http: reqwest::Client,
    fetch_cache: HashMap<FetchKey, Result<Option<ExternalSource>, String>>,
    compile_cache: HashMap<(Version, String), CompiledCandidates>,
    compiler_versions: HashSet<Version>,
    compilations: usize,
    retained_source_input: usize,
    retained_candidates: usize,
    retained_metadata: usize,
    retained_creation_bytecode: usize,
}

impl ExternalResolver {
    pub(super) fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            fetch_cache: HashMap::new(),
            compile_cache: HashMap::new(),
            compiler_versions: HashSet::new(),
            compilations: 0,
            retained_source_input: 0,
            retained_candidates: 0,
            retained_metadata: 0,
            retained_creation_bytecode: 0,
        })
    }

    pub(super) async fn resolve_etherscan(
        &mut self,
        chain: Chain,
        address: Address,
        endpoint: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Option<ExternalSource>, String> {
        let endpoint = endpoint.ok_or_else(|| "Etherscan is not configured".to_string())?;
        let provider = SourceProvider::Etherscan { endpoint: endpoint.to_string() };
        let key = FetchKey { chain: chain.id(), provider: provider.clone(), address };
        if let Some(cached) = self.fetch_cache.get(&key) {
            return cached.clone();
        }
        if self.fetch_cache.len() >= MAX_FETCHES {
            return Err("external source fetch limit exceeded".to_string());
        }
        let result = async {
            let address = address.to_string();
            let response = self
                .http
                .get(endpoint)
                .query(&[
                    ("module", "contract"),
                    ("action", "getsourcecode"),
                    ("address", address.as_str()),
                    ("apikey", api_key.unwrap_or_default()),
                ])
                .send()
                .await
                .map_err(|_| "Etherscan request failed".to_string())?;
            if !response.status().is_success() {
                return Err(format!("Etherscan returned HTTP {}", response.status()));
            }
            if response.content_length().is_some_and(|length| length > MAX_INPUT_SIZE as u64) {
                return Err("Etherscan response exceeds 10 MiB".to_string());
            }
            let body = read_capped_body(response, MAX_INPUT_SIZE, "Etherscan").await?;
            let (input, version) = parse_etherscan_response(&body)?;
            Ok(Some(ExternalSource { input: Arc::new(input), version, provider }))
        }
        .await;
        self.cache_source_result(key, result)
    }

    pub(super) async fn resolve_sourcify(
        &mut self,
        chain: Chain,
        address: Address,
        endpoint: Option<&str>,
    ) -> Result<Option<ExternalSource>, String> {
        let Some(endpoint) = endpoint else { return Ok(None) };
        // Canonical Sourcify cannot observe ephemeral local chains. Avoid a guaranteed external
        // request so local external-verification workflows remain hermetic.
        if endpoint == SOURCIFY_URL
            && matches!(
                chain.named(),
                Some(NamedChain::Dev | NamedChain::AnvilHardhat | NamedChain::Cannon)
            )
        {
            return Ok(None);
        }
        let provider = SourceProvider::Sourcify { endpoint: endpoint.to_string() };
        let key = FetchKey { chain: chain.id(), provider: provider.clone(), address };
        if let Some(cached) = self.fetch_cache.get(&key) {
            return cached.clone();
        }
        if self.fetch_cache.len() >= MAX_FETCHES {
            return Err("external source fetch limit exceeded".to_string());
        }
        let result = async {
            let endpoint = endpoint.trim_end_matches('/');
            let url = format!(
                "{endpoint}/v2/contract/{}/{address}?fields=stdJsonInput,compilation.compilerVersion",
                chain.id()
            );
            let response = self.http.get(url).send().await.map_err(|_| "Sourcify request failed".to_string())?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None)
            }
            if !response.status().is_success() {
                return Err(format!("Sourcify returned HTTP {}", response.status()))
            }
            if response.content_length().is_some_and(|length| length > MAX_INPUT_SIZE as u64) {
                return Err("Sourcify response exceeds 10 MiB".to_string())
            }
            let body = read_capped_body(response, MAX_INPUT_SIZE, "Sourcify").await?;
            let response: SourcifyResponse =
                serde_json::from_slice(&body).map_err(|e| format!("invalid Sourcify response: {e}"))?;
            validate_input(&response.std_json_input).map_err(|e| e.to_string())?;
            let version = parse_compiler_version(&response.compilation.compiler_version)
                .map_err(|e| e.to_string())?;
            Ok(Some(ExternalSource {
                input: Arc::new(response.std_json_input),
                version,
                provider,
            }))
        }
        .await;
        self.cache_source_result(key, result)
    }

    pub(super) async fn compile(
        &mut self,
        source: &ExternalSource,
    ) -> Result<(Arc<[Candidate]>, bool), String> {
        let fingerprint = fingerprint(&source.input).map_err(|e| e.to_string())?;
        let key = (source.version.clone(), fingerprint);
        if let Some(cached) = self.compile_cache.get(&key) {
            return cached.clone();
        }
        if self.compilations >= MAX_COMPILATIONS {
            return Err("external compilation limit exceeded".to_string());
        }
        let new_version = !self.compiler_versions.contains(&source.version);
        if new_version && self.compiler_versions.len() >= MAX_COMPILER_VERSIONS {
            return Err("external compiler version limit exceeded".to_string());
        }
        let version_len = source.version.to_string().len();
        let cache_metadata = key
            .1
            .len()
            .saturating_add(version_len)
            .saturating_add(if new_version { version_len } else { 0 });
        self.compilations += 1;
        let result = match compile_source(source).await {
            Ok((candidates, has_unresolved_links)) => self
                .charge_candidates(candidates, cache_metadata)
                .map(|candidates| (candidates, has_unresolved_links)),
            Err(err) => Err(err),
        };
        let result = match result {
            Ok(candidates) => Ok(candidates),
            Err(err) => {
                let err = bound_cached_error(err);
                self.charge_metadata(cache_metadata.saturating_add(err.len()))?;
                Err(err)
            }
        };
        self.compiler_versions.insert(source.version.clone());
        self.compile_cache.insert(key, result.clone());
        result
    }

    fn charge_candidates(
        &mut self,
        candidates: Vec<Candidate>,
        cache_metadata: usize,
    ) -> Result<Arc<[Candidate]>, String> {
        let bytes = candidates.iter().fold(0usize, |total, candidate| {
            total.saturating_add(candidate.creation_bytecode.len())
        });
        let metadata = candidates.iter().try_fold(cache_metadata, |total, candidate| {
            let constructor = candidate
                .constructor
                .as_ref()
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|_| "failed to measure candidate constructor".to_string())?
                .map_or(0, |constructor| constructor.len());
            Ok::<_, String>(
                total
                    .saturating_add(candidate.fqn.len())
                    .saturating_add(candidate.fingerprint.len())
                    .saturating_add(candidate.version.to_string().len())
                    .saturating_add(constructor),
            )
        })?;
        if self.retained_candidates.saturating_add(candidates.len()) > MAX_CANDIDATES {
            return Err("external cumulative candidate limit exceeded".to_string());
        }
        if self.retained_metadata.saturating_add(metadata) > MAX_RETAINED_METADATA {
            return Err("external cumulative metadata limit exceeded".to_string());
        }
        if self.retained_creation_bytecode.saturating_add(bytes) > MAX_CREATION_BYTECODE {
            return Err("external cumulative creation bytecode limit exceeded".to_string());
        }
        self.retained_candidates += candidates.len();
        self.retained_metadata += metadata;
        self.retained_creation_bytecode += bytes;
        Ok(Arc::from(candidates))
    }

    fn cache_source_result(
        &mut self,
        key: FetchKey,
        result: Result<Option<ExternalSource>, String>,
    ) -> Result<Option<ExternalSource>, String> {
        let result = result.map_err(bound_cached_error);
        let source_bytes = match &result {
            Ok(Some(source)) => serde_json::to_vec(&source.input)
                .map_err(|_| "failed to measure Standard JSON input".to_string())?
                .len(),
            Ok(None) | Err(_) => 0,
        };
        let retained = key.provider.metadata_len().saturating_add(match &result {
            Ok(Some(source)) => {
                source.provider.metadata_len().saturating_add(source.version.to_string().len())
            }
            Ok(None) => 0,
            Err(err) => err.len(),
        });
        if self.retained_source_input.saturating_add(source_bytes) > MAX_RETAINED_SOURCE_INPUT {
            return Err("external cumulative source input limit exceeded".to_string());
        }
        if self.retained_metadata.saturating_add(retained) > MAX_RETAINED_METADATA {
            return Err("external cumulative metadata limit exceeded".to_string());
        }
        self.retained_source_input += source_bytes;
        self.retained_metadata += retained;
        self.fetch_cache.insert(key, result.clone());
        result
    }

    fn charge_metadata(&mut self, bytes: usize) -> Result<(), String> {
        if self.retained_metadata.saturating_add(bytes) > MAX_RETAINED_METADATA {
            return Err("external cumulative metadata limit exceeded".to_string());
        }
        self.retained_metadata += bytes;
        Ok(())
    }
}

fn bound_cached_error(error: String) -> String {
    let mut chars = error.chars();
    let error = chars.by_ref().take(MAX_CACHED_ERROR_CHARS).collect::<String>();
    if chars.next().is_some() { format!("{error}…") } else { error }
}

async fn read_capped_body(
    response: reqwest::Response,
    limit: usize,
    provider: &str,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| format!("invalid {provider} response"))?;
        if !append_capped(&mut body, &chunk, limit) {
            return Err(format!("{provider} response exceeds 10 MiB"));
        }
    }
    Ok(body)
}

fn append_capped(output: &mut Vec<u8>, chunk: &[u8], limit: usize) -> bool {
    if output.len().saturating_add(chunk.len()) > limit {
        return false;
    }
    output.extend_from_slice(chunk);
    true
}

fn parse_etherscan_response(body: &[u8]) -> Result<(Value, Version), String> {
    let response: Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid Etherscan response: {e}"))?;
    if response.get("status").and_then(Value::as_str) != Some("1") {
        let message = response.get("result").and_then(Value::as_str).unwrap_or("request failed");
        return Err(format!("Etherscan request failed: {message}"));
    }
    let item = response
        .get("result")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .ok_or_else(|| "empty Etherscan response".to_string())?;
    let compiler_version = item
        .get("CompilerVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| "Etherscan compiler version is missing".to_string())?;
    let compiler_version =
        parse_compiler_version(compiler_version).map_err(|err| err.to_string())?;
    let source =
        item.get("SourceCode").ok_or_else(|| "Etherscan source code is missing".to_string())?;
    let input = match source {
        Value::Object(_) => source.clone(),
        Value::String(source) => {
            let wrapped = source.starts_with("{{") && source.ends_with("}}");
            let source = if wrapped { &source[1..source.len() - 1] } else { source };
            serde_json::from_str(source)
                .map_err(|_| "Etherscan did not return Standard JSON sources".to_string())?
        }
        _ => return Err("Etherscan did not return Standard JSON sources".to_string()),
    };
    validate_input(&input).map_err(|e| e.to_string())?;
    Ok((input, compiler_version))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcifyResponse {
    std_json_input: Value,
    compilation: SourcifyCompilation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcifyCompilation {
    compiler_version: String,
}

fn parse_compiler_version(version: &str) -> Result<Version> {
    if version.len() > MAX_COMPILER_VERSION {
        bail!("compiler version exceeds {MAX_COMPILER_VERSION} bytes");
    }
    Version::parse(version.trim_start_matches('v')).map_err(Into::into)
}

fn validate_input(input: &Value) -> Result<()> {
    if serde_json::to_vec(input)?.len() > MAX_INPUT_SIZE {
        bail!("Standard JSON input exceeds 10 MiB");
    }
    let object = input.as_object().ok_or_else(|| eyre!("Standard JSON input is not an object"))?;
    if object.get("language").and_then(Value::as_str) != Some("Solidity") {
        bail!("source language is not Solidity");
    }
    let sources = object
        .get("sources")
        .and_then(Value::as_object)
        .ok_or_else(|| eyre!("Standard JSON sources are missing"))?;
    if sources.is_empty() || sources.len() > MAX_SOURCES {
        bail!("Standard JSON must contain 1 to 256 sources");
    }
    for (path, source) in sources {
        validate_identifier(path, "source path")?;
        let source = source.as_object().ok_or_else(|| eyre!("invalid source entry"))?;
        if source.get("content").and_then(Value::as_str).is_none_or(str::is_empty) {
            bail!("every source must contain nonempty literal content");
        }
    }
    if !object.get("settings").is_some_and(Value::is_object) {
        bail!("Standard JSON settings must be an object");
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_SOURCE_PATH {
        bail!("{label} must contain 1 to 4096 bytes");
    }
    if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("{label} must contain printable ASCII characters");
    }
    Ok(())
}

fn compilation_input(input: &Value) -> Result<Value> {
    validate_input(input)?;
    let mut input = input.clone();
    input["settings"]["outputSelection"] = json!({
        "*": { "*": ["abi", "evm.bytecode.object", "evm.bytecode.linkReferences"] }
    });
    Ok(input)
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonicalize(&object[key])))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        value => value.clone(),
    }
}

fn fingerprint(input: &Value) -> Result<String> {
    validate_input(input)?;
    Ok(keccak256(serde_json::to_vec(&canonicalize(input))?).to_string())
}

fn compiler_matches(requested: &Version, actual: &Version) -> bool {
    requested.major == actual.major
        && requested.minor == actual.minor
        && requested.patch == actual.patch
        && requested.pre == actual.pre
        && requested.build.as_str().split('.').take(2).eq(actual.build.as_str().split('.').take(2))
}

fn compiler_identity(version: &Version) -> String {
    let build = version.build.as_str().split('.').take(2).collect::<Vec<_>>().join(".");
    format!("{}.{}.{}-{}+{build}", version.major, version.minor, version.patch, version.pre)
}

async fn compile_source(source: &ExternalSource) -> Result<(Vec<Candidate>, bool), String> {
    let input = compilation_input(&source.input).map_err(|e| e.to_string())?;
    let svm_version =
        Version::new(source.version.major, source.version.minor, source.version.patch);
    let solc = match Solc::find_svm_installed_version(&svm_version).map_err(|e| e.to_string())? {
        Some(solc) => solc,
        None => tokio::time::timeout(Duration::from_secs(60), Solc::install(&svm_version))
            .await
            .map_err(|_| "solc installation timed out".to_string())?
            .map_err(|e| e.to_string())?,
    };
    let mut version_command = Command::new(&solc.solc);
    version_command.arg("--version");
    let version_output =
        run_bounded_command(version_command, None, 64 * 1024, 64 * 1024, Duration::from_secs(10))
            .await?;
    let actual = parse_solc_version_output(&version_output.0)?;
    if !compiler_matches(&source.version, &actual) {
        return Err(format!("installed solc {actual} does not match requested {}", source.version));
    }
    let workdir =
        tempfile::tempdir().map_err(|_| "failed to create compiler sandbox".to_string())?;
    let raw_input =
        serde_json::to_vec(&input).map_err(|_| "failed to encode compiler input".to_string())?;
    let mut command = Command::new(&solc.solc);
    command
        .arg("--standard-json")
        .arg("--base-path")
        .arg(workdir.path())
        .current_dir(workdir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Older solc versions do not expose an import-callback switch. Literal source validation and
    // an empty base/current directory prevent their filesystem importer from resolving a file.
    if source.version >= Version::new(0, 8, 22) {
        command.arg("--no-import-callback");
    }
    let (stdout, stderr, status) =
        run_bounded_command(command, Some(raw_input), MAX_STDOUT, MAX_STDERR, COMPILE_TIMEOUT)
            .await?;
    if !status.success() {
        return Err(format!("solc compilation failed: {}", sanitize_remote(&stderr)));
    }
    let output: CompilerOutput =
        serde_json::from_slice(&stdout).map_err(|_| "solc returned invalid JSON".to_string())?;
    if output.has_error() {
        return Err("solc compilation failed".to_string());
    }
    candidates_from_output(source, output)
}

fn candidates_from_output(
    source: &ExternalSource,
    output: CompilerOutput,
) -> Result<(Vec<Candidate>, bool), String> {
    let fingerprint = fingerprint(&source.input).map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    let mut creation_bytecode_bytes = 0usize;
    let mut has_unresolved_links = false;
    for (path, contracts) in output.contracts {
        for (name, contract) in contracts {
            let Some(abi) = contract.abi else { continue };
            let Some(bytecode) = contract.evm.and_then(|evm| evm.bytecode) else { continue };
            if !bytecode.link_references.is_empty() || bytecode.object.is_unlinked() {
                has_unresolved_links = true;
                continue;
            }
            let Some(bytes) = bytecode.object.into_bytes() else { continue };
            if bytes.is_empty() {
                continue;
            }
            validate_identifier(&path.to_string_lossy(), "compiler source path")
                .map_err(|e| e.to_string())?;
            validate_identifier(&name, "contract name").map_err(|e| e.to_string())?;
            if candidates.len() >= MAX_CANDIDATES {
                return Err("external candidate limit exceeded".to_string());
            }
            if creation_bytecode_bytes.saturating_add(bytes.len()) > MAX_CREATION_BYTECODE {
                return Err("external creation bytecode limit exceeded".to_string());
            }
            creation_bytecode_bytes += bytes.len();
            candidates.push(Candidate {
                input: source.input.clone(),
                fingerprint: fingerprint.clone(),
                version: source.version.clone(),
                fqn: format!("{}:{name}", path.display()),
                constructor: abi.constructor().cloned(),
                creation_bytecode: bytes,
            });
        }
    }
    Ok((candidates, has_unresolved_links))
}

fn parse_solc_version_output(output: &[u8]) -> Result<Version, String> {
    let output =
        std::str::from_utf8(output).map_err(|_| "invalid solc version output".to_string())?;
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Version: "))
        .ok_or_else(|| "solc version output is missing Version line".to_string())
        // Linux solc builds identify GCC as `g++`, whose plus signs are invalid in SemVer build
        // metadata. Use the same normalization as foundry-compilers' solc version parser.
        .and_then(|version| {
            Version::parse(&version.replace(".g++", ".gcc"))
                .map_err(|_| "invalid solc version".to_string())
        })
}

async fn run_bounded_command(
    mut command: Command,
    input: Option<Vec<u8>>,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
) -> Result<(Vec<u8>, Vec<u8>, std::process::ExitStatus), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    // Reserve part of the caller's timeout for cancelling I/O, terminating, and reaping. This
    // keeps the complete subprocess lifecycle within one absolute deadline.
    let cleanup_window = std::cmp::min(timeout / 10, Duration::from_secs(1));
    let execution_deadline = deadline - cleanup_window;
    command
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| "failed to start subprocess".to_string())?;
    let stdout =
        child.stdout.take().ok_or_else(|| "failed to open subprocess stdout".to_string())?;
    let stderr =
        child.stderr.take().ok_or_else(|| "failed to open subprocess stderr".to_string())?;
    let mut input_task = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        tokio::spawn(async move {
            stdin.write_all(&input).await?;
            stdin.shutdown().await
        })
    });
    let mut output_task = tokio::spawn(async move {
        tokio::try_join!(read_capped(stdout, stdout_limit), read_capped(stderr, stderr_limit))
    });
    let mut input_joined = false;
    let mut output_joined = false;
    let lifecycle = async {
        let mut completed_output = None;
        let status = tokio::select! {
            result = child.wait() => {
                result.map_err(|_| "failed to wait for subprocess".to_string())?
            }
            output = &mut output_task => {
                output_joined = true;
                match output {
                    Ok(Ok(output)) => {
                        completed_output = Some(output);
                        child.wait().await.map_err(|_| "failed to wait for subprocess".to_string())?
                    }
                    Ok(Err(err)) => return Err(err),
                    Err(_) => return Err("subprocess output task failed".to_string()),
                }
            }
        };
        if let Some(task) = input_task.as_mut() {
            let input = task.await;
            input_joined = true;
            input
                .map_err(|_| "subprocess input task failed".to_string())?
                .map_err(|_| "failed to write subprocess input".to_string())?;
        }
        let (stdout, stderr) = match completed_output {
            Some(output) => output,
            None => {
                let output = (&mut output_task).await;
                output_joined = true;
                output.map_err(|_| "subprocess output task failed".to_string())??
            }
        };
        Ok((stdout, stderr, status))
    };
    let result = match tokio::time::timeout_at(execution_deadline, lifecycle).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(err),
        Err(_) => Err("subprocess timed out".to_string()),
    };
    if result.is_ok() {
        return result;
    }

    if let Some(task) = &input_task {
        task.abort();
    }
    output_task.abort();
    let _ = child.start_kill();
    let cleanup = async {
        if !input_joined && let Some(task) = input_task {
            let _ = task.await;
        }
        if !output_joined {
            let _ = output_task.await;
        }
        let _ = child.wait().await;
    };
    // If the OS does not complete cleanup in its reserved window, dropping the child retains
    // `kill_on_drop` as a final best-effort safeguard while preserving the caller's deadline.
    let _ = tokio::time::timeout_at(deadline, cleanup).await;
    result
}

async fn read_capped(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let read =
            reader.read(&mut chunk).await.map_err(|_| "failed to read solc output".to_string())?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > limit {
            return Err("solc output limit exceeded".to_string());
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

fn sanitize_remote(message: &[u8]) -> String {
    let message = String::from_utf8_lossy(message);
    let clean = message
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(160)
        .collect::<String>();
    if clean.is_empty() { "no diagnostic".to_string() } else { clean }
}

pub(super) fn match_candidates<'a>(
    observed: &[u8],
    candidates: impl IntoIterator<Item = &'a Candidate>,
) -> MatchResult {
    let mut matches = Vec::new();
    for candidate in candidates {
        let bytecode = candidate.creation_bytecode.as_ref();
        if bytecode.is_empty() {
            continue;
        }
        let Some(suffix) = observed.strip_prefix(bytecode) else { continue };
        let valid = match &candidate.constructor {
            None => suffix.is_empty(),
            Some(constructor) if constructor.inputs.is_empty() => suffix.is_empty(),
            Some(constructor) => constructor
                .abi_decode_input(suffix)
                .ok()
                .and_then(|values| constructor.abi_encode_input(&values).ok())
                .is_some_and(|encoded| encoded == suffix),
        };
        if valid
            && !matches.iter().any(|item: &ExternalMatch| {
                item.fqn == candidate.fqn
                    && compiler_identity(&item.version) == compiler_identity(&candidate.version)
                    && item.creation_bytecode == candidate.creation_bytecode
                    && item.constructor_args.as_ref() == suffix
            })
        {
            matches.push(ExternalMatch {
                input: candidate.input.clone(),
                version: candidate.version.clone(),
                fqn: candidate.fqn.clone(),
                creation_bytecode: candidate.creation_bytecode.clone(),
                constructor_args: Bytes::copy_from_slice(suffix),
            });
        }
    }
    match matches.len() {
        0 => MatchResult::None,
        1 => MatchResult::Unique(matches.pop().unwrap()),
        _ => MatchResult::Ambiguous(matches),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_abi::JsonAbi;
    use tokio::net::TcpListener;

    fn input() -> Value {
        json!({
            "language": "Solidity",
            "sources": { "A.sol": { "content": "contract A {}", "custom": 1 } },
            "settings": { "optimizer": { "enabled": true }, "outputSelection": {"old": []} },
            "unknown": { "preserved": true }
        })
    }

    fn candidate(fqn: &str, abi: JsonAbi) -> Candidate {
        Candidate {
            input: Arc::new(input()),
            fingerprint: "fingerprint".into(),
            version: Version::parse("0.8.30+commit.73712a01").unwrap(),
            fqn: fqn.into(),
            constructor: abi.constructor().cloned(),
            creation_bytecode: Bytes::from_static(&[0x60, 0x00]),
        }
    }

    #[test]
    fn validation_preserves_unknown_fields_and_only_changes_output_selection() {
        let original = input();
        let compiled = compilation_input(&original).unwrap();
        assert_eq!(compiled["unknown"], original["unknown"]);
        assert_eq!(compiled["sources"], original["sources"]);
        assert_eq!(compiled["settings"]["optimizer"], original["settings"]["optimizer"]);
        assert_ne!(
            compiled["settings"]["outputSelection"],
            original["settings"]["outputSelection"]
        );
        assert_eq!(original["settings"]["outputSelection"], json!({"old": []}));

        let mut url_only = input();
        url_only["sources"]["A.sol"] = json!({"urls": ["ipfs://source"]});
        assert!(validate_input(&url_only).is_err());
    }

    #[test]
    fn source_paths_are_bounded_printable_ascii() {
        assert!(validate_identifier("src/A.sol", "path").is_ok());
        assert!(validate_identifier("bad\npath.sol", "path").is_err());
        assert!(validate_identifier("café.sol", "path").is_err());
        assert!(validate_identifier(&"x".repeat(MAX_SOURCE_PATH + 1), "path").is_err());
    }

    #[test]
    fn cumulative_source_budget_charges_success_once() {
        let mut resolver = ExternalResolver::new().unwrap();
        let source = ExternalSource {
            input: Arc::new(input()),
            version: Version::new(0, 8, 30),
            provider: SourceProvider::Sourcify { endpoint: SOURCIFY_URL.into() },
        };
        let key = FetchKey { chain: 1, provider: source.provider.clone(), address: Address::ZERO };
        let charged = resolver.cache_source_result(key.clone(), Ok(Some(source))).unwrap();
        assert!(charged.is_some());
        let charged_bytes = resolver.retained_source_input;
        // A cache hit returns the retained value directly and never invokes accounting again.
        assert!(resolver.fetch_cache.get(&key).unwrap().is_ok());
        assert_eq!(resolver.retained_source_input, charged_bytes);

        resolver.retained_source_input = MAX_RETAINED_SOURCE_INPUT;
        let rejected_key = FetchKey { address: Address::with_last_byte(1), ..key };
        assert!(
            resolver
                .cache_source_result(rejected_key.clone(), Ok(charged))
                .unwrap_err()
                .contains("cumulative")
        );
        assert!(!resolver.fetch_cache.contains_key(&rejected_key));
    }

    #[test]
    fn cumulative_candidate_metadata_budget_is_enforced() {
        let mut resolver = ExternalResolver::new().unwrap();
        let retained_candidate = candidate("A.sol:A", JsonAbi::default());
        let expected = retained_candidate.fqn.len()
            + retained_candidate.fingerprint.len()
            + retained_candidate.version.to_string().len();
        resolver.charge_candidates(vec![retained_candidate], 0).unwrap();
        assert_eq!(resolver.retained_metadata, expected);

        let mut resolver = ExternalResolver::new().unwrap();
        resolver.retained_metadata = MAX_RETAINED_METADATA;
        let error = resolver
            .charge_candidates(vec![candidate("A.sol:A", JsonAbi::default())], 0)
            .unwrap_err();
        assert!(error.contains("metadata"));
        assert_eq!(resolver.retained_metadata, MAX_RETAINED_METADATA);
        assert_eq!(resolver.retained_candidates, 0);
        assert_eq!(resolver.retained_creation_bytecode, 0);

        let mut resolver = ExternalResolver::new().unwrap();
        resolver.retained_creation_bytecode = MAX_CREATION_BYTECODE;
        let error = resolver
            .charge_candidates(vec![candidate("A.sol:A", JsonAbi::default())], 0)
            .unwrap_err();
        assert!(error.contains("bytecode"));
        assert_eq!(resolver.retained_metadata, 0);
        assert_eq!(resolver.retained_candidates, 0);
        assert_eq!(resolver.retained_creation_bytecode, MAX_CREATION_BYTECODE);
    }

    #[test]
    fn candidate_constructor_metadata_is_charged() {
        let abi: JsonAbi = serde_json::from_value(json!([{
            "type": "constructor", "inputs": [{"name":"n", "type":"uint256"}]
        }]))
        .unwrap();
        let candidate = candidate("A.sol:A", abi);
        let constructor_bytes =
            serde_json::to_vec(candidate.constructor.as_ref().unwrap()).unwrap();
        let mut resolver = ExternalResolver::new().unwrap();
        resolver.charge_candidates(vec![candidate], 0).unwrap();
        assert!(resolver.retained_metadata >= constructor_bytes.len());
    }

    #[tokio::test]
    async fn sourcify_source_discovery_uses_selected_endpoint_on_dev_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/private", listener.local_addr().unwrap());
        let response = json!({
            "stdJsonInput": input(),
            "compilation": { "compilerVersion": "0.8.30+commit.73712a01" }
        })
        .to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let bytes_read = socket.read(&mut request).await.unwrap();
            let request = std::str::from_utf8(&request[..bytes_read]).unwrap();
            assert!(request.starts_with("GET /private/v2/contract/31337/"), "{request}");
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let source = ExternalResolver::new()
            .unwrap()
            .resolve_sourcify(Chain::from(31337), Address::ZERO, Some(&endpoint))
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert_eq!(source.provider, SourceProvider::Sourcify { endpoint: endpoint.clone() });
        assert_ne!(source.provider, SourceProvider::Sourcify { endpoint: SOURCIFY_URL.into() });
    }

    #[tokio::test]
    async fn sourcify_cache_identity_includes_selected_endpoint() {
        let mut resolver = ExternalResolver::new().unwrap();
        for _ in 0..2 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/private", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                socket
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            });
            assert!(
                resolver
                    .resolve_sourcify(Chain::from(31337), Address::ZERO, Some(&endpoint))
                    .await
                    .unwrap()
                    .is_none()
            );
            server.await.unwrap();
        }
        assert_eq!(resolver.fetch_cache.len(), 2);
    }

    #[test]
    fn remote_versions_and_cached_errors_are_bounded_and_accounted() {
        let long_version = format!("0.8.30+{}", "a".repeat(MAX_COMPILER_VERSION));
        assert!(Version::parse(&long_version).is_ok());
        assert!(parse_compiler_version(&long_version).is_err());

        let mut resolver = ExternalResolver::new().unwrap();
        let endpoint = "https://explorer.invalid/api";
        let key = FetchKey {
            chain: 1,
            provider: SourceProvider::Etherscan { endpoint: endpoint.into() },
            address: Address::ZERO,
        };
        let error = resolver
            .cache_source_result(key, Err("x".repeat(MAX_CACHED_ERROR_CHARS * 2)))
            .unwrap_err();
        assert_eq!(error.chars().count(), MAX_CACHED_ERROR_CHARS + 1);
        assert_eq!(resolver.retained_metadata, endpoint.len() + error.len());

        let mut resolver = ExternalResolver::new().unwrap();
        resolver.retained_metadata = MAX_RETAINED_METADATA;
        let key = FetchKey {
            chain: 1,
            provider: SourceProvider::Sourcify { endpoint: SOURCIFY_URL.into() },
            address: Address::ZERO,
        };
        let source = ExternalSource {
            input: Arc::new(input()),
            version: Version::new(0, 8, 30),
            provider: key.provider.clone(),
        };
        assert!(resolver.cache_source_result(key, Ok(Some(source))).is_err());
        assert_eq!(resolver.retained_source_input, 0);
        assert_eq!(resolver.retained_metadata, MAX_RETAINED_METADATA);
    }

    #[tokio::test]
    async fn etherscan_source_discovery_does_not_follow_redirects() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", source.local_addr().unwrap());
        let location = format!("http://{}", target.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = source.accept().await.unwrap();
            let mut request = [0; 4096];
            let bytes_read = socket.read(&mut request).await.unwrap();
            assert!(bytes_read > 0);
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let error = ExternalResolver::new()
            .unwrap()
            .resolve_etherscan(Chain::mainnet(), Address::ZERO, Some(&endpoint), None)
            .await
            .unwrap_err();
        server.await.unwrap();
        assert!(error.contains("HTTP 307"), "unexpected discovery error: {error}");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), target.accept()).await.is_err(),
            "external source discovery followed the redirect"
        );
    }

    #[test]
    fn etherscan_parser_preserves_raw_standard_json() {
        let expected = json!({
            "language": "Solidity",
            "sources": { "A.sol": {
                "content": "contract A {}",
                "urls": ["dweb:/ipfs/source"],
                "keccak256": "0x1234"
            } },
            "settings": {},
            "unknown": { "preserved": true }
        });
        for source in [expected.clone(), Value::String(format!("{{{expected}}}"))] {
            let response = json!({
                "status": "1",
                "message": "OK",
                "result": [{ "SourceCode": source, "CompilerVersion": "v0.8.30+commit.73712a01" }]
            });
            let (parsed, version) =
                parse_etherscan_response(&serde_json::to_vec(&response).unwrap()).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(version, Version::parse("0.8.30+commit.73712a01").unwrap());
        }

        let flat = json!({
            "status": "1", "result": [{
                "SourceCode": "contract A {}", "CompilerVersion": "v0.8.30+commit.73712a01"
            }]
        });
        assert!(parse_etherscan_response(&serde_json::to_vec(&flat).unwrap()).is_err());
    }

    #[test]
    fn fingerprint_ignores_object_key_order() {
        let first = input();
        let second: Value = serde_json::from_str(
            r#"{"unknown":{"preserved":true},"settings":{"outputSelection":{"old":[]},"optimizer":{"enabled":true}},"sources":{"A.sol":{"custom":1,"content":"contract A {}"}},"language":"Solidity"}"#,
        )
        .unwrap();
        assert_eq!(fingerprint(&first).unwrap(), fingerprint(&second).unwrap());

        let source_a = ExternalSource {
            input: Arc::new(first),
            version: Version::new(1, 2, 3),
            provider: SourceProvider::Sourcify { endpoint: SOURCIFY_URL.into() },
        };
        let source_b = ExternalSource {
            provider: SourceProvider::Etherscan { endpoint: "other".into() },
            ..source_a.clone()
        };
        assert_eq!(fingerprint(&source_a.input).unwrap(), fingerprint(&source_b.input).unwrap());
    }

    #[test]
    fn compiler_version_requires_commit_but_allows_platform_suffix() {
        let requested = Version::parse("0.8.30+commit.73712a01").unwrap();
        assert!(compiler_matches(
            &requested,
            &Version::parse("0.8.30+commit.73712a01.Linux.gcc").unwrap()
        ));
        assert!(!compiler_matches(&requested, &Version::parse("0.8.30+commit.deadbeef").unwrap()));
        assert!(!compiler_matches(&Version::new(0, 8, 30), &requested));
    }

    #[test]
    fn parses_real_solc_version_output() {
        let output = b"solc, the solidity compiler commandline interface\nVersion: 0.8.30+commit.73712a01.Darwin.appleclang\n";
        assert_eq!(
            parse_solc_version_output(output).unwrap(),
            Version::parse("0.8.30+commit.73712a01.Darwin.appleclang").unwrap()
        );
        let linux = b"Version: 0.8.30+commit.73712a01.Linux.g++\n";
        assert_eq!(
            parse_solc_version_output(linux).unwrap(),
            Version::parse("0.8.30+commit.73712a01.Linux.gcc").unwrap()
        );
        assert!(parse_solc_version_output(b"Version: forged").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_process_times_out_and_caps_output() {
        let mut sleep = Command::new("sh");
        sleep.args(["-c", "sleep 2"]);
        let started = tokio::time::Instant::now();
        let error =
            run_bounded_command(sleep, None, 16, 16, Duration::from_millis(20)).await.unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(500));

        // Output closes before the process exits, so the output-task branch wins first. Waiting
        // for the process must remain subject to the original deadline.
        let mut closed_output = Command::new("sh");
        closed_output.args(["-c", "exec 1>&- 2>&-; sleep 2"]);
        assert!(
            run_bounded_command(closed_output, None, 16, 16, Duration::from_millis(20))
                .await
                .unwrap_err()
                .contains("timed out")
        );

        let mut output = Command::new("sh");
        output.args(["-c", "printf 12345"]);
        assert!(
            run_bounded_command(output, None, 4, 16, Duration::from_secs(1))
                .await
                .unwrap_err()
                .contains("output limit")
        );

        // The shell exits immediately, but its child retains the output pipe. The same absolute
        // deadline must still cover draining output after `child.wait()` completes.
        let mut inherited_output = Command::new("sh");
        inherited_output.args(["-c", "sleep 2 &"]);
        assert!(
            run_bounded_command(inherited_output, None, 16, 16, Duration::from_millis(20))
                .await
                .unwrap_err()
                .contains("timed out")
        );

        // The direct child exits while a descendant retains stdin without reading it. Awaiting a
        // blocked writer must also remain subject to the same deadline.
        let mut inherited_input = Command::new("sh");
        inherited_input.args(["-c", "exec 3<&0; sleep 2 >/dev/null 2>&1 &"]);
        assert!(
            run_bounded_command(
                inherited_input,
                Some(vec![0; 1024 * 1024]),
                16,
                16,
                Duration::from_millis(20)
            )
            .await
            .unwrap_err()
            .contains("timed out")
        );
    }

    #[test]
    fn constructor_matching_is_canonical() {
        let constructor: JsonAbi = serde_json::from_value(json!([{
            "type": "constructor", "inputs": [{"name":"n", "type":"uint256"}]
        }]))
        .unwrap();
        let candidate = candidate("A.sol:A", constructor);
        let mut observed = vec![0x60, 0x00];
        observed.extend([0; 31]);
        observed.push(7);
        let MatchResult::Unique(found) =
            match_candidates(&observed, std::slice::from_ref(&candidate))
        else {
            panic!("expected match")
        };
        assert_eq!(found.constructor_args.len(), 32);
        assert!(matches!(match_candidates(&[0x60, 0x00, 7], &[candidate]), MatchResult::None));
    }

    #[test]
    fn no_constructor_and_zero_inputs_require_empty_suffix() {
        let none = candidate("A.sol:A", JsonAbi::default());
        assert!(matches!(
            match_candidates(&[0x60, 0x00], std::slice::from_ref(&none)),
            MatchResult::Unique(_)
        ));
        assert!(matches!(match_candidates(&[0x60, 0x00, 0], &[none]), MatchResult::None));

        let zero: JsonAbi =
            serde_json::from_value(json!([{"type":"constructor","inputs":[]}])).unwrap();
        assert!(matches!(
            match_candidates(&[0x60, 0x00], &[candidate("B.sol:B", zero)]),
            MatchResult::Unique(_)
        ));
    }

    #[test]
    fn ambiguity_collapses_equivalent_deployments_from_different_inputs() {
        let a = candidate("A.sol:A", JsonAbi::default());
        let first_input = a.input.clone();
        let mut duplicate = a.clone();
        duplicate.fingerprint = "different-provider-input".into();
        duplicate.input = Arc::new(json!({ "providerSpecific": true }));
        duplicate.version = Version::parse("0.8.30+commit.73712a01.Linux.gcc").unwrap();
        let MatchResult::Unique(matched) = match_candidates(&[0x60, 0x00], &[a.clone(), duplicate])
        else {
            panic!("equivalent deployments should be deduplicated")
        };
        assert!(Arc::ptr_eq(&matched.input, &first_input));
        let mut other_commit = a.clone();
        other_commit.version = Version::parse("0.8.30+commit.deadbeef").unwrap();
        assert!(matches!(
            match_candidates(&[0x60, 0x00], &[a.clone(), other_commit]),
            MatchResult::Ambiguous(_)
        ));

        let constructor: JsonAbi = serde_json::from_value(json!([{
            "type": "constructor", "inputs": [{"name":"n", "type":"uint256"}]
        }]))
        .unwrap();
        let prefixed = candidate("A.sol:A", constructor);
        let mut observed = prefixed.creation_bytecode.to_vec();
        observed.extend([0; 32]);
        let exact = Candidate {
            constructor: None,
            creation_bytecode: Bytes::copy_from_slice(&observed),
            ..prefixed.clone()
        };
        assert!(matches!(
            match_candidates(&observed, &[prefixed, exact]),
            MatchResult::Ambiguous(_)
        ));

        let b = candidate("B.sol:B", JsonAbi::default());
        let MatchResult::Ambiguous(matches) = match_candidates(&[0x60, 0x00], &[a, b]) else {
            panic!("expected ambiguity")
        };
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn unresolved_library_links_are_flagged_without_dropping_other_candidates() {
        let placeholder = format!("__${}$__", "0".repeat(34));
        let output = serde_json::from_value(json!({
            "contracts": {
                "A.sol": {
                    "Linked": {
                        "abi": [],
                        "evm": { "bytecode": {
                            "object": "6000",
                            "linkReferences": {
                                "Lib.sol": { "Lib": [{ "start": 0, "length": 20 }] }
                            }
                        }}
                    },
                    "Placeholder": {
                        "abi": [],
                        "evm": { "bytecode": { "object": placeholder } }
                    },
                    "Plain": {
                        "abi": [],
                        "evm": { "bytecode": { "object": "6001" } }
                    }
                }
            }
        }))
        .unwrap();
        let source = ExternalSource {
            input: Arc::new(input()),
            version: Version::new(0, 8, 30),
            provider: SourceProvider::Sourcify { endpoint: SOURCIFY_URL.into() },
        };
        let (candidates, has_unresolved_links) = candidates_from_output(&source, output).unwrap();
        assert!(has_unresolved_links);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].fqn, "A.sol:Plain");
    }

    #[test]
    fn capped_chunk_aggregation_rejects_limit_plus_one() {
        let mut output = Vec::new();
        assert!(append_capped(&mut output, b"123", 4));
        assert!(!append_capped(&mut output, b"45", 4));
        assert_eq!(output, b"123");
    }

    #[test]
    fn empty_creation_bytecode_never_matches() {
        let mut empty = candidate("A.sol:A", JsonAbi::default());
        empty.creation_bytecode = Bytes::new();
        assert!(matches!(match_candidates(&[], &[empty]), MatchResult::None));
    }

    #[test]
    fn candidates_share_source_input() {
        let first = candidate("A.sol:A", JsonAbi::default());
        let second = Candidate { fqn: "A.sol:B".into(), ..first.clone() };
        assert!(Arc::ptr_eq(&first.input, &second.input));
    }

    #[test]
    fn remote_diagnostics_are_bounded_and_sanitized() {
        let message = format!("secret\n{}", "x".repeat(300));
        let clean = sanitize_remote(message.as_bytes());
        assert!(!clean.contains('\n'));
        assert_eq!(clean.chars().count(), 160);
    }
}
