//! External compiler adapter protocol and process transport.

use alloy_primitives::hex;
use eyre::{Context, ContextCompat, Result, bail, ensure};
use foundry_compilers::{
    ArtifactFile, ArtifactOutput, Artifacts, ConfigurableArtifacts, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, Severity, contract::Contract},
    compilers::{Compiler, Language, multi::MultiCompilerLanguage},
    contracts::{VersionedContract, VersionedContracts},
};
use foundry_config::{Config, DenyLevel, ExternalCompiler};
use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread::JoinHandle,
};

const PROTOCOL_VERSION: &str = "1.0";
const MAX_PROTOCOL_LINE_BYTES: u64 = 16 * 1024 * 1024;
const EXTERNAL_CACHE_DIR: &str = "external-compilers";
const EXTERNAL_ARTIFACT_DIR: &str = ".external";

/// External artifacts and the unit inventory used to publish them.
pub(crate) struct ExternalCompilation<'a> {
    config: &'a Config,
    artifacts: Artifacts<ConfigurableContractArtifact>,
    contracts: VersionedContracts<Contract>,
    write_outputs: bool,
    complete_discovery: bool,
    active_units: BTreeMap<String, BTreeSet<String>>,
    pending_cache: Vec<PendingCache>,
}

impl<'a> ExternalCompilation<'a> {
    /// Runs adapters, staging their output until the built-in compilation succeeds.
    pub(crate) fn compile(
        config: &'a Config,
        selected_paths: &[PathBuf],
        write_outputs: bool,
    ) -> Result<Self> {
        let mut compilation = Self {
            config,
            artifacts: Artifacts::default(),
            contracts: VersionedContracts::default(),
            write_outputs,
            complete_discovery: selected_paths.is_empty(),
            active_units: BTreeMap::new(),
            pending_cache: Vec::new(),
        };
        let mut adapter_keys = BTreeSet::new();
        for adapter in &config.external_compilers {
            ensure!(
                adapter_keys.insert(adapter.id.to_ascii_lowercase()),
                "duplicate or case-insensitive external compiler adapter ID `{}`",
                adapter.id
            );
            ensure_portable_child(&config.out.join(EXTERNAL_ARTIFACT_DIR), &adapter.id, None)?;
            ensure_portable_child(&config.cache_path.join(EXTERNAL_CACHE_DIR), &adapter.id, None)?;
            AdapterClient::new(config, adapter, selected_paths)?.compile(&mut compilation)?;
        }
        Ok(compilation)
    }

    /// Merges validated external artifacts into a built-in compilation result.
    pub(crate) fn merge<C>(self, output: &mut ProjectCompileOutput<C>) -> Result<()>
    where
        C: Compiler<CompilerContract = Contract>,
    {
        for (source, contracts) in &self.artifacts {
            for name in contracts.keys() {
                ensure!(
                    output.find(source, name).is_none(),
                    "external compiler artifact conflicts with built-in artifact {}:{name}",
                    source.display()
                );
            }
        }

        if self.write_outputs {
            let output_root = self.config.out.join(EXTERNAL_ARTIFACT_DIR);
            let cache_root = self.config.cache_path.join(EXTERNAL_CACHE_DIR);
            let active_adapters = self.active_units.keys().cloned().collect();
            if self.config.cache {
                retire_children(&cache_root, &active_adapters, None)?;
                if self.complete_discovery {
                    for (adapter, units) in &self.active_units {
                        retire_children(&cache_root.join(adapter), units, Some("json"))?;
                    }
                }
                for cache in &self.pending_cache {
                    write_cache(&cache.path, &cache.contents)?;
                }
            }
            retire_children(&output_root, &active_adapters, None)?;
            for (adapter, units) in &self.active_units {
                let adapter_root = output_root.join(adapter);
                if self.complete_discovery {
                    retire_children(&adapter_root, units, None)?;
                }
                for unit in units {
                    let unit_root = adapter_root.join(unit);
                    if unit_root.exists() {
                        fs::remove_dir_all(unit_root)?;
                    }
                }
            }
            self.artifacts.write_all()?;
        }

        if self.artifacts.is_empty() {
            return Ok(());
        }
        let mut merged = output.compiled_artifacts().clone();
        for (source, contracts) in self.artifacts {
            merged.as_mut().entry(source).or_default().extend(contracts);
        }
        for (source, contracts) in self.contracts {
            output.output_mut().contracts.as_mut().entry(source).or_default().extend(contracts);
        }
        output.set_compiled_artifacts(merged);
        Ok(())
    }
}

struct AdapterClient<'a> {
    config: &'a Config,
    cache_root: PathBuf,
    adapter: &'a ExternalCompiler,
    selected_paths: &'a [PathBuf],
    command: PathBuf,
}

impl<'a> AdapterClient<'a> {
    fn new(
        config: &'a Config,
        adapter: &'a ExternalCompiler,
        selected_paths: &'a [PathBuf],
    ) -> Result<Self> {
        validate_id("adapter", &adapter.id)?;
        ensure!(!adapter.roots.is_empty(), "external compiler `{}` has no roots", adapter.id);
        let command = resolve_file(&config.root, &adapter.command).wrap_err_with(|| {
            format!("failed to resolve external compiler adapter `{}`", adapter.id)
        })?;
        Ok(Self {
            config,
            cache_root: config.cache_path.join(EXTERNAL_CACHE_DIR).join(&adapter.id),
            adapter,
            selected_paths,
            command,
        })
    }

    fn compile(&self, output: &mut ExternalCompilation<'_>) -> Result<()> {
        let mut process =
            AdapterProcess::spawn(&self.config.root, &self.command, &self.adapter.args)
                .wrap_err_with(|| {
                    format!("failed to start external compiler `{}`", self.adapter.id)
                })?;
        let result = self.compile_units(output, &mut process);
        process.finish(result)
    }

    fn compile_units(
        &self,
        output: &mut ExternalCompilation<'_>,
        process: &mut AdapterProcess,
    ) -> Result<()> {
        let initialized: InitializeResult = process.request(
            "initialize",
            json!({
                "protocols": [PROTOCOL_VERSION],
                "host": {"name": "forge", "version": env!("CARGO_PKG_VERSION")},
                "target": "evm",
            }),
        )?;
        ensure!(
            initialized.protocol == PROTOCOL_VERSION,
            "external compiler `{}` selected unsupported protocol `{}`",
            self.adapter.id,
            initialized.protocol
        );

        let roots = self
            .adapter
            .roots
            .iter()
            .map(|path| normalize_root_path(path).map(|path| self.config.root.join(path)))
            .collect::<Result<Vec<_>>>()?;
        let discovery: DiscoverResult = process.request(
            "discover",
            json!({
                "roots": roots,
                "settings": self.adapter.settings,
                "selected_paths": self.selected_paths,
            }),
        )?;

        let mut active_units = BTreeSet::new();
        let mut unit_keys = BTreeSet::new();
        for unit in discovery.units {
            validate_id("build unit", &unit.id)?;
            ensure!(
                active_units.insert(unit.id.clone()),
                "external compiler `{}` returned duplicate unit ID `{}`",
                self.adapter.id,
                unit.id
            );
            ensure!(
                !unit.compiler.name.is_empty(),
                "external compiler `{}` unit `{}` returned an empty compiler name",
                self.adapter.id,
                unit.id
            );
            ensure!(
                unit_keys.insert(unit.id.to_ascii_lowercase()),
                "external compiler `{}` unit ID `{}` collides on case-insensitive filesystems",
                self.adapter.id,
                unit.id
            );
            ensure_portable_child(&self.cache_root, &unit.id, Some("json"))?;
            ensure_portable_child(
                &self.config.out.join(EXTERNAL_ARTIFACT_DIR).join(&self.adapter.id),
                &unit.id,
                None,
            )?;
            ensure!(
                unit.capabilities.contains("build/1"),
                "external compiler `{}` unit `{}` does not support `build/1`",
                self.adapter.id,
                unit.id
            );
            let fingerprint = self.fingerprint(&unit)?;
            let cache_path = self.cache_root.join(format!("{}.json", unit.id));
            let result = if self.config.cache && !self.config.force && unit.cacheable {
                read_cache(&cache_path, &fingerprint)?
            } else {
                None
            };
            let (result, fresh) = match result {
                Some(result) => (result, false),
                None => {
                    let result: CompileResult = process
                        .request("compile", json!({"unit": unit.id, "fingerprint": fingerprint}))?;
                    (result, true)
                }
            };
            emit_diagnostics(&self.adapter.id, &unit.id, &result.diagnostics, self.config.deny)?;
            let cache = (output.write_outputs && fresh && self.config.cache && unit.cacheable)
                .then(|| {
                    serde_json::to_vec(&CacheEntry {
                        fingerprint: fingerprint.clone(),
                        result: &result,
                    })
                })
                .transpose()?;
            self.add_artifacts(output, &unit, &fingerprint, result, fresh)?;
            if let Some(contents) = cache {
                output.pending_cache.push(PendingCache { path: cache_path, contents });
            }
        }

        output.active_units.insert(self.adapter.id.clone(), active_units);
        Ok(())
    }

    fn fingerprint(&self, unit: &DiscoveredUnit) -> Result<String> {
        let mut hasher = Sha256::new();
        hash_part(&mut hasher, PROTOCOL_VERSION.as_bytes());
        hash_part(&mut hasher, self.command.as_os_str().as_encoded_bytes());
        hash_part(&mut hasher, &fs::read(&self.command)?);
        hash_part(&mut hasher, &serde_json::to_vec(&self.adapter.args)?);
        hash_part(&mut hasher, &serde_json::to_vec(&self.adapter.settings)?);
        hash_part(&mut hasher, &serde_json::to_vec(unit)?);

        let mut inputs = unit
            .inputs
            .iter()
            .map(|path| resolve_file(&self.config.root, path))
            .collect::<Result<Vec<_>>>()?;
        inputs.sort();
        inputs.dedup();
        for input in inputs {
            hash_part(&mut hasher, input.as_os_str().as_encoded_bytes());
            hash_part(&mut hasher, &fs::read(&input)?);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    fn add_artifacts(
        &self,
        output: &mut ExternalCompilation<'_>,
        unit: &DiscoveredUnit,
        fingerprint: &str,
        result: CompileResult,
        fresh: bool,
    ) -> Result<()> {
        let version = Version::parse(&unit.compiler.version).wrap_err_with(|| {
            format!(
                "external compiler `{}` unit `{}` returned non-SemVer compiler version `{}`",
                self.adapter.id, unit.id, unit.compiler.version
            )
        })?;
        let unit_out =
            self.config.out.join(EXTERNAL_ARTIFACT_DIR).join(&self.adapter.id).join(&unit.id);
        let mut artifact_paths = BTreeSet::new();
        for artifact in result.artifacts {
            let name = artifact.name.clone();
            validate_id("contract", &name)?;
            ensure!(
                !name.contains(['.', '-']),
                "external compiler contract name must contain only ASCII letters, digits, or underscores: {name}"
            );
            let source = validate_relative_path("source unit", &artifact.source)?.to_path_buf();
            let source_path = resolve_file(&self.config.root, &source)?;
            ensure!(
                output
                    .artifacts
                    .get(&source_path)
                    .and_then(|existing| existing.get(&name))
                    .is_none(),
                "external compiler `{}` returned duplicate artifact {}:{}",
                self.adapter.id,
                source.display(),
                name
            );
            let artifact_path = unit_out.join(&source).join(format!("{name}.json"));
            let relative_artifact_path = source.join(format!("{name}.json"));
            ensure!(
                artifact_paths
                    .insert(relative_artifact_path.to_string_lossy().to_ascii_lowercase()),
                "external compiler `{}` unit `{}` artifact path `{}` collides on case-insensitive filesystems",
                self.adapter.id,
                unit.id,
                relative_artifact_path.display()
            );
            let (contract, compiler_contract) = artifact.into_foundry_outputs()?;
            let build_id = format!(
                "external:{}:{}:{}:{fingerprint}",
                self.adapter.id,
                unit.id,
                if unit.capabilities.contains("forge-tests/1") { "forge-tests" } else { "build" }
            );
            if fresh {
                output
                    .contracts
                    .as_mut()
                    .entry(source_path.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_default()
                    .push(VersionedContract {
                        contract: compiler_contract,
                        version: version.clone(),
                        build_id: build_id.clone(),
                        profile: self.config.profile.to_string(),
                    });
            }
            output
                .artifacts
                .as_mut()
                .entry(source_path)
                .or_default()
                .entry(name)
                .or_default()
                .push(ArtifactFile {
                    artifact: contract,
                    file: artifact_path,
                    version: version.clone(),
                    build_id,
                    profile: self.config.profile.to_string(),
                });
        }
        Ok(())
    }
}

struct AdapterProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
    request_id: u64,
}

impl AdapterProcess {
    fn spawn(root: &Path, command: &Path, args: &[String]) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().context("adapter stdin was not piped")?;
        let stdout = child.stdout.take().context("adapter stdout was not piped")?;
        let mut child_stderr = child.stderr.take().context("adapter stderr was not piped")?;
        let stderr = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 8192];
            loop {
                let read = child_stderr.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                let remaining = MAX_PROTOCOL_LINE_BYTES.saturating_sub(bytes.len() as u64) as usize;
                bytes.extend_from_slice(&chunk[..read.min(remaining)]);
            }
            Ok(bytes)
        });
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr: Some(stderr),
            request_id: 0,
        })
    }

    fn request<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &'static str,
        params: P,
    ) -> Result<R> {
        self.request_id += 1;
        let request = Request { id: self.request_id, method, params };
        let stdin = self.stdin.as_mut().context("adapter stdin is closed")?;
        serde_json::to_writer(&mut *stdin, &request)?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;

        let mut line = String::new();
        self.stdout
            .by_ref()
            .take(MAX_PROTOCOL_LINE_BYTES + 1)
            .read_line(&mut line)
            .wrap_err_with(|| format!("failed reading `{method}` response"))?;
        ensure!(!line.is_empty(), "adapter exited before responding to `{method}`");
        ensure!(
            line.len() as u64 <= MAX_PROTOCOL_LINE_BYTES,
            "adapter `{method}` response exceeds {MAX_PROTOCOL_LINE_BYTES} bytes"
        );
        let response: Response<R> = serde_json::from_str(&line)
            .wrap_err_with(|| format!("invalid adapter response to `{method}`"))?;
        ensure!(response.id == self.request_id, "adapter response ID does not match request");
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => bail!("adapter error {}: {}", error.code, error.message),
            _ => bail!("adapter response must contain exactly one of `result` or `error`"),
        }
    }

    fn finish(mut self, result: Result<()>) -> Result<()> {
        drop(self.stdin.take());
        if result.is_err() {
            let _ = self.child.kill();
        }
        let status = self.child.wait()?;
        let stderr = self
            .stderr
            .take()
            .context("adapter stderr reader missing")?
            .join()
            .map_err(|_| eyre::eyre!("adapter stderr reader panicked"))??;
        if stderr.is_empty() {
            result?;
        } else {
            result.wrap_err_with(|| {
                format!("adapter stderr: {}", String::from_utf8_lossy(&stderr).trim_end())
            })?;
        }
        ensure!(
            status.success(),
            "adapter exited with {status}: {}",
            String::from_utf8_lossy(&stderr)
        );
        Ok(())
    }
}

impl Drop for AdapterProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[derive(Serialize)]
struct Request<P> {
    id: u64,
    method: &'static str,
    params: P,
}

#[derive(Deserialize)]
struct Response<R> {
    id: u64,
    result: Option<R>,
    error: Option<ProtocolError>,
}

#[derive(Deserialize)]
struct ProtocolError {
    code: String,
    message: String,
}

#[derive(Deserialize)]
struct InitializeResult {
    protocol: String,
}

#[derive(Deserialize)]
struct DiscoverResult {
    units: Vec<DiscoveredUnit>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredUnit {
    id: String,
    compiler: CompilerIdentity,
    inputs: Vec<PathBuf>,
    #[serde(default)]
    capabilities: BTreeSet<String>,
    #[serde(default)]
    effective_settings: Value,
    #[serde(default)]
    cacheable: bool,
}

#[derive(Deserialize, Serialize)]
struct CompilerIdentity {
    name: String,
    version: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompileResult {
    #[serde(default)]
    diagnostics: Vec<ExternalDiagnostic>,
    #[serde(default)]
    artifacts: Vec<ExternalArtifact>,
}

#[derive(Deserialize, Serialize)]
struct ExternalDiagnostic {
    severity: Severity,
    message: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    source: Option<PathBuf>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalArtifact {
    source: PathBuf,
    name: String,
    contract: Contract,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    source_id: Option<u32>,
}

impl ExternalArtifact {
    fn into_foundry_outputs(mut self) -> Result<(ConfigurableContractArtifact, Contract)> {
        let method_identifiers = self
            .contract
            .abi
            .get_or_insert_with(Default::default)
            .functions()
            .map(|function| (function.signature(), hex::encode(function.selector())))
            .collect::<BTreeMap<_, _>>();
        if let Some(evm) = &mut self.contract.evm {
            ensure!(
                evm.bytecode.is_some() || evm.deployed_bytecode.is_none(),
                "external artifact {} has runtime bytecode without creation bytecode",
                self.name
            );
            for bytecode in evm.bytecode.iter().chain(
                evm.deployed_bytecode.iter().filter_map(|deployed| deployed.bytecode.as_ref()),
            ) {
                ensure!(
                    bytecode.object.is_bytecode() && bytecode.link_references.is_empty(),
                    "external artifact {} must provide fully linked bytecode",
                    self.name
                );
            }
            evm.method_identifiers = method_identifiers.clone();
        }
        let mut artifact = ConfigurableArtifacts::default().contract_to_artifact(
            &self.source,
            &self.name,
            self.contract.clone(),
            None,
        );
        artifact.raw_metadata = self.metadata.as_ref().map(serde_json::to_string).transpose()?;
        artifact.id = self.source_id;
        artifact.method_identifiers = Some(method_identifiers);
        Ok((artifact, self.contract))
    }
}

#[derive(Deserialize, Serialize)]
struct CacheEntry<R> {
    fingerprint: String,
    result: R,
}

struct PendingCache {
    path: PathBuf,
    contents: Vec<u8>,
}

/// Returns whether an artifact can be selected as a Forge test contract.
pub fn external_artifact_is_test_eligible(build_id: &str) -> bool {
    !is_external_artifact(build_id)
        || build_id.split(':').nth(3).is_some_and(|role| role == "forge-tests")
}

/// Returns whether an artifact was produced by an external compiler adapter.
pub fn is_external_artifact(build_id: &str) -> bool {
    build_id.starts_with("external:")
}

/// Returns whether a path belongs to a compiler built into Foundry.
pub fn is_builtin_compiler_source(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|extension| {
        MultiCompilerLanguage::FILE_EXTENSIONS
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

fn emit_diagnostics(
    adapter: &str,
    unit: &str,
    diagnostics: &[ExternalDiagnostic],
    deny: DenyLevel,
) -> Result<()> {
    let mut errors = Vec::new();
    for diagnostic in diagnostics {
        let code = diagnostic.code.as_deref().map(|code| format!(" [{code}]")).unwrap_or_default();
        let source = diagnostic
            .source
            .as_ref()
            .map(|source| format!(" {}", source.display()))
            .unwrap_or_default();
        let message = format!(
            "external compiler `{adapter}` unit `{unit}`{code}{source}: {}",
            diagnostic.message
        );
        match diagnostic.severity {
            Severity::Error => errors.push(message),
            Severity::Warning if deny.warnings() => errors.push(message),
            Severity::Warning => sh_warn!("{message}")?,
            Severity::Info => tracing::info!("{message}"),
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

fn read_cache(path: &Path, fingerprint: &str) -> Result<Option<CompileResult>> {
    let Ok(file) = File::open(path) else { return Ok(None) };
    let entry: CacheEntry<Value> = serde_json::from_reader(file)
        .wrap_err_with(|| format!("failed to read external compiler cache {}", path.display()))?;
    if entry.fingerprint != fingerprint {
        return Ok(None);
    }
    Ok(Some(serde_json::from_value(entry.result)?))
}

fn write_cache(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("external compiler cache path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    temp.as_file_mut().flush()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn storage_entries(root: &Path) -> Result<Option<fs::ReadDir>> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            ensure!(
                !metadata.is_symlink(),
                "external compiler storage path is a symlink: {}",
                root.display()
            );
            Ok(Some(fs::read_dir(root)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn retire_children(root: &Path, active: &BTreeSet<String>, extension: Option<&str>) -> Result<()> {
    let Some(entries) = storage_entries(root)? else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = if let Some(extension) = extension {
            if path.extension() != Some(OsStr::new(extension)) {
                continue;
            }
            path.file_stem()
        } else {
            path.file_name()
        };
        if name.is_some_and(|name| !active.contains(name.to_string_lossy().as_ref())) {
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

fn ensure_portable_child(root: &Path, name: &str, extension: Option<&str>) -> Result<()> {
    let Some(entries) = storage_entries(root)? else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(candidate) = (if let Some(extension) = extension {
            (path.extension() == Some(OsStr::new(extension))).then(|| path.file_stem()).flatten()
        } else {
            path.file_name()
        }) else {
            continue;
        };
        let candidate = candidate.to_string_lossy();
        ensure!(
            candidate != name || !entry.file_type()?.is_symlink(),
            "external compiler storage path is a symlink: {}",
            path.display()
        );
        ensure!(
            candidate == name || !candidate.eq_ignore_ascii_case(name),
            "external compiler storage namespace `{name}` collides with existing `{candidate}` on case-insensitive filesystems"
        );
    }
    Ok(())
}

fn validate_id(kind: &str, value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "external compiler {kind} ID cannot be empty");
    ensure!(
        !matches!(value, "." | "..")
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "invalid external compiler {kind} ID `{value}`"
    );
    Ok(())
}

fn validate_relative_path<'a>(kind: &str, path: &'a Path) -> Result<&'a Path> {
    ensure!(!path.as_os_str().is_empty(), "external compiler {kind} cannot be empty");
    ensure!(path.is_relative(), "external compiler {kind} must be relative: {}", path.display());
    ensure!(
        path.components().all(|component| matches!(component, Component::Normal(_))),
        "external compiler {kind} contains an unsafe component: {}",
        path.display()
    );
    Ok(path)
}

fn normalize_root_path(path: &Path) -> Result<&Path> {
    if path == Path::new(".") { Ok(Path::new("")) } else { validate_relative_path("root", path) }
}

fn resolve_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
    let path = dunce::canonicalize(&path)
        .wrap_err_with(|| format!("failed to canonicalize {}", path.display()))?;
    ensure!(path.is_file(), "external compiler input is not a file: {}", path.display());
    Ok(path)
}

fn hash_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn rejects_unsafe_protocol_paths_and_ids() {
        assert!(validate_relative_path("source", Path::new("src/main.fe")).is_ok());
        assert!(validate_relative_path("source", Path::new("../outside.fe")).is_err());
        assert!(validate_relative_path("source", Path::new("/outside.fe")).is_err());
        assert_eq!(normalize_root_path(Path::new(".")).unwrap(), Path::new(""));
        assert!(validate_id("unit", "app-1").is_ok());
        assert!(validate_id("unit", ".").is_err());
        assert!(validate_id("unit", "..").is_err());
        assert!(validate_id("unit", "../app").is_err());
    }

    #[test]
    fn rejects_case_insensitive_storage_collisions() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("App")).unwrap();
        assert!(ensure_portable_child(dir.path(), "app", None).is_err());
        ensure_portable_child(dir.path(), "App", None).unwrap();

        fs::write(dir.path().join("Build.json"), "{}").unwrap();
        assert!(ensure_portable_child(dir.path(), "build", Some("json")).is_err());
        ensure_portable_child(dir.path(), "Build", Some("json")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_linked_storage() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let sentinel = target.join("keep");
        fs::write(&sentinel, "unchanged").unwrap();
        let link = dir.path().join("linked");
        symlink(&target, &link).unwrap();

        assert!(retire_children(&link, &Default::default(), None).is_err());
        assert!(ensure_portable_child(dir.path(), "linked", None).is_err());
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "unchanged");
    }

    #[test]
    fn rejects_unlinked_bytecode() {
        for field in ["bytecode", "deployedBytecode"] {
            let mut value = serde_json::json!({
                "source": "native/src/lib.fe",
                "name": "Token",
                "contract": {"evm": {"bytecode": {"object": "00"}}}
            });
            value["contract"]["evm"][field] = serde_json::json!({
                "object": "0000000000000000000000000000000000000000",
                "linkReferences": {"native/src/lib.fe": {"Library": [{"start": 0, "length": 20}]}}
            });
            let artifact: ExternalArtifact = serde_json::from_value(value).unwrap();
            assert_eq!(
                artifact.into_foundry_outputs().unwrap_err().to_string(),
                "external artifact Token must provide fully linked bytecode"
            );
        }
    }
}
