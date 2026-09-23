//! External compiler adapter protocol and process transport.

use alloy_json_abi::JsonAbi;
use alloy_primitives::{Bytes, hex};
use eyre::{Context, ContextCompat, Result, bail, ensure};
use foundry_compilers::{
    ArtifactFile, Artifacts, ProjectCompileOutput,
    artifacts::{
        BytecodeObject, CompactBytecode, CompactDeployedBytecode, ConfigurableContractArtifact,
        Offsets, contract::Contract,
    },
    compilers::{Compiler, Language, multi::MultiCompilerLanguage},
};
use foundry_config::{Config, ExternalCompiler};
use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
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

/// The Foundry workflow consuming external compiler output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalCompilerWorkflow {
    Build,
    Test,
    Script,
    Create,
    Inspect,
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
pub(crate) fn is_builtin_compiler_source(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|extension| {
        MultiCompilerLanguage::FILE_EXTENSIONS
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

/// External artifacts and the unit inventory used to publish them.
pub(crate) struct ExternalCompilation {
    artifacts: Artifacts<ConfigurableContractArtifact>,
    output_root: PathBuf,
    cache_root: PathBuf,
    cache_enabled: bool,
    write_outputs: bool,
    complete_discovery: bool,
    active_units: BTreeMap<String, BTreeSet<String>>,
    pending_cache: Vec<PendingCache>,
}

/// Runs configured external compiler adapters and returns their normalized artifacts.
pub(crate) fn compile_external(
    config: &Config,
    workflow: ExternalCompilerWorkflow,
    selected_paths: &[PathBuf],
    write_outputs: bool,
) -> Result<ExternalCompilation> {
    let mut external = Artifacts::default();
    let mut active_units = BTreeMap::new();
    let mut pending_cache = Vec::new();
    for adapter in &config.external_compilers {
        ensure!(
            !active_units.contains_key(&adapter.id),
            "duplicate external compiler adapter ID `{}`",
            adapter.id
        );
        let (artifacts, units, adapter_cache) =
            AdapterClient::new(config, adapter, workflow, selected_paths)?
                .compile(write_outputs)?;
        for (source, contracts) in artifacts {
            for name in contracts.keys() {
                ensure!(
                    external.get(&source).and_then(|existing| existing.get(name)).is_none(),
                    "external compiler artifact collision for {}:{name}",
                    source.display()
                );
            }
            external.as_mut().entry(source).or_default().extend(contracts);
        }
        active_units.insert(adapter.id.clone(), units);
        pending_cache.extend(adapter_cache);
    }

    Ok(ExternalCompilation {
        artifacts: external,
        output_root: config.out.join(EXTERNAL_ARTIFACT_DIR),
        cache_root: config.cache_path.join(EXTERNAL_CACHE_DIR),
        cache_enabled: config.cache,
        write_outputs,
        complete_discovery: selected_paths.is_empty(),
        active_units,
        pending_cache,
    })
}

/// Merges validated external artifacts into a built-in compilation result.
pub(crate) fn merge_external<C>(
    output: &mut ProjectCompileOutput<C>,
    external: ExternalCompilation,
) -> Result<()>
where
    C: Compiler<CompilerContract = Contract>,
{
    let mut merged = output.compiled_artifacts().clone();
    for (source, contracts) in &external.artifacts {
        for name in contracts.keys() {
            ensure!(
                output.find(source, name).is_none(),
                "external compiler artifact conflicts with built-in artifact {}:{name}",
                source.display()
            );
        }
    }

    if external.write_outputs {
        let active_adapters = external.active_units.keys().cloned().collect();
        if external.cache_enabled {
            retire_children(&external.cache_root, &active_adapters, None)?;
            if external.complete_discovery {
                for (adapter, units) in &external.active_units {
                    retire_children(&external.cache_root.join(adapter), units, Some("json"))?;
                }
            }
            for cache in &external.pending_cache {
                write_cache(&cache.path, &cache.contents)?;
            }
        }
        retire_children(&external.output_root, &active_adapters, None)?;
        for (adapter, units) in &external.active_units {
            let adapter_root = external.output_root.join(adapter);
            if external.complete_discovery {
                retire_children(&adapter_root, units, None)?;
            }
            for unit in units {
                let unit_root = adapter_root.join(unit);
                if unit_root.exists() {
                    fs::remove_dir_all(unit_root)?;
                }
            }
        }
        external.artifacts.write_all()?;
    }

    for (source, contracts) in external.artifacts {
        merged.as_mut().entry(source).or_default().extend(contracts);
    }
    output.set_compiled_artifacts(merged);
    Ok(())
}

struct AdapterClient<'a> {
    root: &'a Path,
    out: &'a Path,
    cache_root: PathBuf,
    cache: bool,
    force: bool,
    profile: String,
    adapter: &'a ExternalCompiler,
    workflow: ExternalCompilerWorkflow,
    selected_paths: &'a [PathBuf],
    command: PathBuf,
}

impl<'a> AdapterClient<'a> {
    fn new(
        config: &'a Config,
        adapter: &'a ExternalCompiler,
        workflow: ExternalCompilerWorkflow,
        selected_paths: &'a [PathBuf],
    ) -> Result<Self> {
        validate_id("adapter", &adapter.id)?;
        ensure!(!adapter.roots.is_empty(), "external compiler `{}` has no roots", adapter.id);
        let command = resolve_file(&config.root, &adapter.command).wrap_err_with(|| {
            format!("failed to resolve external compiler adapter `{}`", adapter.id)
        })?;
        Ok(Self {
            root: &config.root,
            out: &config.out,
            cache_root: config.cache_path.join(EXTERNAL_CACHE_DIR).join(&adapter.id),
            cache: config.cache,
            force: config.force,
            profile: config.profile.to_string(),
            adapter,
            workflow,
            selected_paths,
            command,
        })
    }

    fn compile(
        &self,
        write_outputs: bool,
    ) -> Result<(Artifacts<ConfigurableContractArtifact>, BTreeSet<String>, Vec<PendingCache>)>
    {
        let mut process = AdapterProcess::spawn(self.root, &self.command, &self.adapter.args)
            .wrap_err_with(|| format!("failed to start external compiler `{}`", self.adapter.id))?;
        let initialized: InitializeResult = process.request(
            "initialize",
            InitializeParams {
                protocols: [PROTOCOL_VERSION],
                host: HostIdentity { name: "forge", version: env!("CARGO_PKG_VERSION") },
                target: "evm",
            },
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
            .map(|path| normalize_root_path(path).map(|path| self.root.join(path)))
            .collect::<Result<Vec<_>>>()?;
        let discovery: DiscoverResult = process.request(
            "discover",
            DiscoverParams {
                roots,
                settings: &self.adapter.settings,
                workflow: self.workflow,
                selected_paths: self.selected_paths,
            },
        )?;

        let mut artifacts = Artifacts::default();
        let mut active_units = BTreeSet::new();
        let mut pending_cache = Vec::new();
        for unit in discovery.units {
            validate_id("build unit", &unit.id)?;
            ensure!(
                !unit.compiler.name.is_empty(),
                "external compiler `{}` unit `{}` returned an empty compiler name",
                self.adapter.id,
                unit.id
            );
            ensure!(
                unit.capabilities.contains("build/1"),
                "external compiler `{}` unit `{}` does not support `build/1`",
                self.adapter.id,
                unit.id
            );
            ensure!(
                active_units.insert(unit.id.clone()),
                "external compiler `{}` returned duplicate unit ID `{}`",
                self.adapter.id,
                unit.id
            );
            let fingerprint = self.fingerprint(&unit)?;
            let cache_path = self.cache_root.join(format!("{}.json", unit.id));
            let result = if self.cache && !self.force && unit.cacheable {
                read_cache(&cache_path, &fingerprint)?
            } else {
                None
            };
            let (result, fresh) = match result {
                Some(result) => (result, false),
                None => {
                    let result: CompileResult = process.request(
                        "compile",
                        CompileParams {
                            unit: &unit.id,
                            fingerprint: &fingerprint,
                            workflow: self.workflow,
                        },
                    )?;
                    (result, true)
                }
            };
            emit_diagnostics(&self.adapter.id, &unit.id, &result.diagnostics)?;
            let cache = (write_outputs && fresh && self.cache && unit.cacheable)
                .then(|| serialize_cache(&fingerprint, &result))
                .transpose()?;
            self.add_artifacts(&mut artifacts, &unit, &fingerprint, result)?;
            if let Some(contents) = cache {
                pending_cache.push(PendingCache { path: cache_path, contents });
            }
        }

        process.finish()?;
        Ok((artifacts, active_units, pending_cache))
    }

    fn fingerprint(&self, unit: &DiscoveredUnit) -> Result<String> {
        let mut hasher = Sha256::new();
        hash_part(&mut hasher, PROTOCOL_VERSION.as_bytes());
        hash_part(&mut hasher, self.command.as_os_str().as_encoded_bytes());
        hash_file(&mut hasher, &self.command)?;
        hash_part(&mut hasher, &serde_json::to_vec(&self.adapter.args)?);
        hash_part(&mut hasher, &serde_json::to_vec(&self.adapter.settings)?);
        hash_part(&mut hasher, &serde_json::to_vec(&self.workflow)?);
        hash_part(&mut hasher, &serde_json::to_vec(unit)?);

        let mut inputs = unit
            .inputs
            .iter()
            .map(|path| resolve_file(self.root, path))
            .collect::<Result<Vec<_>>>()?;
        inputs.sort();
        inputs.dedup();
        for input in inputs {
            hash_part(&mut hasher, input.as_os_str().as_encoded_bytes());
            hash_file(&mut hasher, &input)?;
        }
        Ok(hex::encode(hasher.finalize()))
    }

    fn add_artifacts(
        &self,
        output: &mut Artifacts<ConfigurableContractArtifact>,
        unit: &DiscoveredUnit,
        fingerprint: &str,
        result: CompileResult,
    ) -> Result<()> {
        let version = Version::parse(&unit.compiler.version).wrap_err_with(|| {
            format!(
                "external compiler `{}` unit `{}` returned non-SemVer compiler version `{}`",
                self.adapter.id, unit.id, unit.compiler.version
            )
        })?;
        let unit_out = self.out.join(EXTERNAL_ARTIFACT_DIR).join(&self.adapter.id).join(&unit.id);
        for artifact in result.artifacts {
            let name = artifact.name.clone();
            validate_id("contract", &name)?;
            let source = validate_relative_path("source unit", &artifact.source)?;
            let source_path = self.root.join(source);
            ensure!(
                output.get(&source_path).and_then(|existing| existing.get(&name)).is_none(),
                "external compiler `{}` returned duplicate artifact {}:{}",
                self.adapter.id,
                source.display(),
                name
            );
            let artifact_path = unit_out.join(source).join(format!("{name}.json"));
            let contract = artifact.into_foundry_artifact()?;
            output.as_mut().entry(source_path).or_default().entry(name).or_default().push(
                ArtifactFile {
                    artifact: contract,
                    file: artifact_path,
                    version: version.clone(),
                    build_id: format!(
                        "external:{}:{}:{}:{fingerprint}",
                        self.adapter.id,
                        unit.id,
                        if unit.capabilities.contains("forge-tests/1") {
                            "forge-tests"
                        } else {
                            "build"
                        }
                    ),
                    profile: self.profile.clone(),
                },
            );
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

    fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        let stderr = self
            .stderr
            .take()
            .context("adapter stderr reader missing")?
            .join()
            .map_err(|_| eyre::eyre!("adapter stderr reader panicked"))??;
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

#[derive(Serialize)]
struct InitializeParams<'a> {
    protocols: [&'a str; 1],
    host: HostIdentity<'a>,
    target: &'a str,
}

#[derive(Serialize)]
struct HostIdentity<'a> {
    name: &'a str,
    version: &'a str,
}

#[derive(Deserialize)]
struct InitializeResult {
    protocol: String,
}

#[derive(Serialize)]
struct DiscoverParams<'a> {
    roots: Vec<PathBuf>,
    settings: &'a Value,
    workflow: ExternalCompilerWorkflow,
    selected_paths: &'a [PathBuf],
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

#[derive(Serialize)]
struct CompileParams<'a> {
    unit: &'a str,
    fingerprint: &'a str,
    workflow: ExternalCompilerWorkflow,
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
    severity: DiagnosticSeverity,
    message: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    source: Option<PathBuf>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalArtifact {
    source: PathBuf,
    name: String,
    #[serde(default)]
    abi: JsonAbi,
    #[serde(default)]
    bytecode: Option<Bytes>,
    #[serde(default)]
    deployed_bytecode: Option<Bytes>,
    #[serde(default)]
    source_map: Option<String>,
    #[serde(default)]
    deployed_source_map: Option<String>,
    #[serde(default)]
    link_references: BTreeMap<String, BTreeMap<String, Vec<Offsets>>>,
    #[serde(default)]
    deployed_link_references: BTreeMap<String, BTreeMap<String, Vec<Offsets>>>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    source_id: Option<u32>,
}

impl ExternalArtifact {
    fn into_foundry_artifact(self) -> Result<ConfigurableContractArtifact> {
        ensure!(
            self.bytecode.is_some() || self.deployed_bytecode.is_none(),
            "external artifact {} has runtime bytecode without creation bytecode",
            self.name
        );
        ensure!(
            self.link_references.is_empty() && self.deployed_link_references.is_empty(),
            "external artifact {} must provide fully linked bytecode",
            self.name
        );
        let bytecode = self.bytecode.map(|bytes| CompactBytecode {
            object: BytecodeObject::Bytecode(bytes),
            source_map: self.source_map,
            link_references: Default::default(),
        });
        let deployed_bytecode = self.deployed_bytecode.map(|bytes| CompactDeployedBytecode {
            bytecode: Some(CompactBytecode {
                object: BytecodeObject::Bytecode(bytes),
                source_map: self.deployed_source_map,
                link_references: Default::default(),
            }),
            immutable_references: Default::default(),
        });
        let method_identifiers = self
            .abi
            .functions()
            .map(|function| (function.signature(), hex::encode(function.selector())))
            .collect();
        Ok(ConfigurableContractArtifact {
            abi: Some(self.abi),
            bytecode,
            deployed_bytecode,
            method_identifiers: Some(method_identifiers),
            raw_metadata: self.metadata.as_ref().map(serde_json::to_string).transpose()?,
            id: self.source_id,
            ..Default::default()
        })
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

fn emit_diagnostics(adapter: &str, unit: &str, diagnostics: &[ExternalDiagnostic]) -> Result<()> {
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
            DiagnosticSeverity::Error => errors.push(message),
            DiagnosticSeverity::Warning => sh_warn!("{message}")?,
            DiagnosticSeverity::Info => tracing::info!("{message}"),
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

fn read_cache(path: &Path, fingerprint: &str) -> Result<Option<CompileResult>> {
    let Ok(file) = File::open(path) else { return Ok(None) };
    let entry: CacheEntry<CompileResult> = serde_json::from_reader(file)
        .wrap_err_with(|| format!("failed to read external compiler cache {}", path.display()))?;
    Ok((entry.fingerprint == fingerprint).then_some(entry.result))
}

fn serialize_cache(fingerprint: &str, result: &CompileResult) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&CacheEntry { fingerprint: fingerprint.to_owned(), result })?)
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

fn retire_children(root: &Path, active: &BTreeSet<String>, extension: Option<&str>) -> Result<()> {
    let Ok(entries) = fs::read_dir(root) else { return Ok(()) };
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
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
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

fn hash_file(hasher: &mut Sha256, path: &Path) -> Result<()> {
    hash_part(hasher, &fs::read(path)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ExternalArtifact, external_artifact_is_test_eligible, is_builtin_compiler_source,
        normalize_root_path, validate_id, validate_relative_path,
    };
    use std::path::Path;

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
    fn external_test_eligibility_is_explicit() {
        assert!(external_artifact_is_test_eligible("solc-build-id"));
        assert!(external_artifact_is_test_eligible("external:adapter:unit:forge-tests:hash"));
        assert!(!external_artifact_is_test_eligible("external:adapter:unit:build:hash"));
    }

    #[test]
    fn recognizes_builtin_source_extensions() {
        assert!(is_builtin_compiler_source(Path::new("src/Contract.sol")));
        assert!(is_builtin_compiler_source(Path::new("src/contract.VY")));
        assert!(!is_builtin_compiler_source(Path::new("native/src/lib.fe")));
    }

    #[test]
    fn derives_method_identifiers_from_abi() {
        let artifact: ExternalArtifact = serde_json::from_value(serde_json::json!({
            "source": "native/src/lib.fe",
            "name": "Token",
            "abi": [{
                "type": "function",
                "name": "balanceOf",
                "inputs": [{"name": "account", "type": "address"}],
                "outputs": [{"name": "", "type": "uint256"}],
                "stateMutability": "view"
            }]
        }))
        .unwrap();

        let artifact = artifact.into_foundry_artifact().unwrap();
        assert_eq!(artifact.method_identifiers.unwrap()["balanceOf(address)"], "70a08231");
    }

    #[test]
    fn rejects_unlinked_bytecode() {
        for field in ["linkReferences", "deployedLinkReferences"] {
            let mut value = serde_json::json!({
                "source": "native/src/lib.fe",
                "name": "Token",
                "bytecode": "0x0000000000000000000000000000000000000000"
            });
            value[field] = serde_json::json!({
                "native/src/lib.fe": {"Library": [{"start": 0, "length": 20}]}
            });
            let artifact: ExternalArtifact = serde_json::from_value(value).unwrap();
            assert!(
                artifact
                    .into_foundry_artifact()
                    .unwrap_err()
                    .to_string()
                    .contains("must provide fully linked bytecode")
            );
        }
    }
}
