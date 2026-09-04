//! Opt-in, occurrence-bound capture for authoritative fresh Z3 queries.

use super::{
    SolverCommand, SolverProcessOutcome, solver_exit_error, solver_wait_duration,
    trace::{self, EncodedQueryTrace, TraceOccurrence},
};
use crate::SymbolicError;
use serde::Serialize;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::{
    ffi::OsStrExt,
    fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
    process::CommandExt,
};
use std::{
    ffi::{CString, OsStr},
    fs::{self, DirBuilder, OpenOptions},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use wait_timeout::ChildExt;

const CAPTURE_DIRECTORY_ENV: &str = "FOUNDRY_INTERNAL_SYMBOLIC_FRESH_Z3_CAPTURE_DIR";

const CAPTURE_SCHEMA: &str = "foundry:symbolic-fresh-z3-capture@v2";
const CAPTURE_SCHEMA_VERSION: u32 = 2;
const SOLVER_WORKING_DIRECTORY: &str = "/";
const SOLVER_PATH: &str = "/usr/bin:/bin";
// The current real-query corpus is orders of magnitude smaller. These bounds admit large models
// while preventing a broken or hostile subprocess from consuming unbounded memory or disk.
const MAX_STDIN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STDOUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STDERR_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ROOT_ENTRIES: usize = 100_000;
// One staging directory, five artifact files, and one temporary publication claim.
const BUNDLE_TRANSIENT_ENTRIES: usize = 7;
const IO_CANCEL_CHECK_INTERVAL: Duration = Duration::from_millis(1);
const PROCESS_REAP_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct FreshZ3Capture {
    root: PathBuf,
    limits: CaptureLimits,
}

#[derive(Clone, Copy)]
struct CaptureLimits {
    stdin: u64,
    stdout: u64,
    stderr: u64,
    total: u64,
    #[cfg(test)]
    force_process_group_kill_failure: bool,
    #[cfg(test)]
    timeout_floor: u32,
}

impl FreshZ3Capture {
    pub(super) fn from_env() -> Option<Self> {
        std::env::var_os(CAPTURE_DIRECTORY_ENV)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map(Self::new)
    }

    const fn new(root: PathBuf) -> Self {
        Self {
            root,
            limits: CaptureLimits {
                stdin: MAX_STDIN_BYTES,
                stdout: MAX_STDOUT_BYTES,
                stderr: MAX_STDERR_BYTES,
                total: MAX_TOTAL_BYTES,
                #[cfg(test)]
                force_process_group_kill_failure: false,
                #[cfg(test)]
                timeout_floor: 0,
            },
        }
    }

    pub(super) fn directory(&self) -> &Path {
        &self.root
    }

    pub(super) fn validate_command<'a>(
        &self,
        commands: &'a [SolverCommand],
        timeout: Option<u32>,
    ) -> Result<&'a SolverCommand, SymbolicError> {
        ensure_process_group_support()?;
        if let Some(trace_directory) = trace::configured_trace_directory() {
            return Err(capture_error(format!(
                "fresh Z3 capture {} cannot be combined with FOUNDRY_SYMBOLIC_QUERY_TRACE_DIR {}",
                self.root.display(),
                trace_directory.display()
            )));
        }
        let [command] = commands else {
            return Err(capture_error(
                "fresh Z3 capture requires exactly one configured solver command",
            ));
        };
        if !Path::new(&command.program).is_absolute()
            || Path::new(&command.program).file_name() != Some(OsStr::new("z3"))
            || command.args != ["-in", "-smt2"]
        {
            return Err(capture_error(
                "fresh Z3 capture requires an absolute canonical `z3 -in -smt2` command",
            ));
        }
        if timeout.is_none_or(|timeout| timeout == 0) {
            return Err(capture_error(
                "fresh Z3 capture requires a positive symbolic solver timeout",
            ));
        }
        Ok(command)
    }

    pub(super) fn run(
        &self,
        command: &SolverCommand,
        smt: &[u8],
        timeout: u32,
        occurrence: &TraceOccurrence,
    ) -> Result<CapturedFreshZ3Run, SymbolicError> {
        #[cfg(test)]
        let timeout = timeout.max(self.limits.timeout_floor);
        let input_length = u64::try_from(smt.len()).unwrap_or(u64::MAX);
        if input_length > self.limits.stdin {
            return Err(capture_error(format!(
                "fresh Z3 capture input exceeds {} bytes",
                self.limits.stdin
            )));
        }

        let mut bundle = PendingCaptureBundle::create(&self.root, occurrence, self.limits.total)?;
        bundle.stdin = Some(bundle.write_artifact("stdin.smt2", "stdin.smt2", smt)?);
        let executable = inspect_solver_executable(&command.program)?;

        let started = Instant::now();
        let run =
            run_fresh_process(command, &executable.canonical_path, smt, timeout, self.limits)?;
        let solver_elapsed = started.elapsed();
        verify_solver_executable(&executable)?;
        let mut stdout = bundle.write_artifact("stdout.bin", "stdout.bin", &run.stdout.bytes)?;
        stdout.truncated = run.stdout.truncated;
        stdout.reached_eof = Some(run.stdout.reached_eof);
        let mut stderr = bundle.write_artifact("stderr.bin", "stderr.bin", &run.stderr.bytes)?;
        stderr.truncated = run.stderr.truncated;
        stderr.reached_eof = Some(run.stderr.reached_eof);
        bundle.stdout = Some(stdout);
        bundle.stderr = Some(stderr);
        bundle.stdin_bytes_written = run.stdin_bytes_written;
        bundle.termination = Some(run.termination);
        bundle.executable = Some(executable);
        bundle.args = Some(command.args.clone());
        bundle.timeout_ms = Some(u64::from(timeout).saturating_mul(1000));
        bundle.solver_elapsed = solver_elapsed;
        Ok(CapturedFreshZ3Run { outcome: run.outcome, solver_elapsed, bundle })
    }

    pub(super) fn commit(
        &self,
        run: PendingCaptureBundle,
        trace: EncodedQueryTrace,
    ) -> Result<(), SymbolicError> {
        self.commit_inner(run, trace, |_| Ok(()), |_| Ok(()))
    }

    fn commit_inner(
        &self,
        mut run: PendingCaptureBundle,
        trace: EncodedQueryTrace,
        before_final_verify: impl FnOnce(&Path) -> Result<(), SymbolicError>,
        after_publish: impl FnOnce(&Path) -> Result<(), SymbolicError>,
    ) -> Result<(), SymbolicError> {
        if run.occurrence_id != trace.occurrence.stem() {
            return Err(capture_error("fresh Z3 capture occurrence identity mismatch"));
        }
        let query_trace = run.write_artifact("query.json", "query.json", &trace.bytes)?;
        let stdin = run
            .stdin
            .take()
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its stdin receipt"))?;
        let stdout = run
            .stdout
            .take()
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its stdout receipt"))?;
        let stderr = run
            .stderr
            .take()
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its stderr receipt"))?;
        let executable = run
            .executable
            .as_ref()
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its solver executable"))?;
        let args = run
            .args
            .as_deref()
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its solver arguments"))?;
        let timeout_ms = run
            .timeout_ms
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its solver timeout"))?;
        let termination = run
            .termination
            .as_ref()
            .ok_or_else(|| capture_error("fresh Z3 capture is missing its termination receipt"))?;
        verify_private_directory_identity(&run.root, run.root_identity)?;
        verify_bundle(
            &run.staging,
            run.staging_identity,
            [&query_trace, &stdin, &stdout, &stderr],
        )?;
        let manifest = CaptureManifest {
            schema: CAPTURE_SCHEMA,
            schema_version: CAPTURE_SCHEMA_VERSION,
            occurrence_id: &run.occurrence_id,
            query_trace: &query_trace,
            solver: SolverReceipt {
                executable,
                args,
                timeout_ms,
                working_directory: SOLVER_WORKING_DIRECTORY,
                environment: SolverEnvironmentReceipt { clear_inherited: true, path: SOLVER_PATH },
            },
            stdin: &stdin,
            stdout: &stdout,
            stderr: &stderr,
            stdin_bytes_written: run.stdin_bytes_written,
            termination,
            solver_wall_time_ns: run.solver_elapsed.as_nanos().try_into().unwrap_or(u64::MAX),
            classified_outcome: trace.outcome,
        };
        let mut manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| {
            capture_error(format!("failed to encode fresh Z3 capture manifest: {error}"))
        })?;
        manifest_bytes.push(b'\n');
        let manifest =
            run.write_artifact("capture.manifest.json", "capture.manifest.json", &manifest_bytes)?;
        sync_directory(&run.staging)?;
        // Occurrence ids are process-unique, and every capture writer must acquire this exclusive
        // claim before checking or publishing the matching destination. This closes the
        // check/rename race between concurrent capture writers without making a partial bundle
        // visible beneath the capture root.
        let mut claim = PublicationClaim::acquire(&run.root, &run.occurrence_id)?;
        ensure_destination_absent(&run.final_path)?;
        before_final_verify(&run.staging)?;
        let usage = capture_root_usage(&run.root)?;
        if usage.bytes > self.limits.total {
            return Err(capture_error(format!(
                "fresh Z3 capture root {} exceeds its {}-byte aggregate budget ({} bytes present)",
                run.root.display(),
                self.limits.total,
                usage.bytes
            )));
        }
        verify_private_directory_identity(&run.root, run.root_identity)?;
        verify_bundle(
            &run.staging,
            run.staging_identity,
            [&query_trace, &stdin, &stdout, &stderr, &manifest],
        )?;
        publish_directory_no_replace(&run.staging, &run.final_path)?;
        // Once the bundle is visible, retain the claim if a durability step fails. A later writer
        // then fails closed instead of guessing whether the prior publication completed.
        claim.preserve_on_drop();
        run.published = true;
        after_publish(&run.final_path)?;
        verify_private_directory_identity(&run.root, run.root_identity)?;
        verify_bundle(
            &run.final_path,
            run.staging_identity,
            [&query_trace, &stdin, &stdout, &stderr, &manifest],
        )?;
        verify_private_directory_identity(&run.root, run.root_identity)?;
        sync_directory(&run.root)?;
        claim.finish()?;
        verify_private_directory_identity(&run.root, run.root_identity)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) const fn for_test(root: PathBuf, stdin: u64, stdout: u64, stderr: u64) -> Self {
        Self {
            root,
            limits: CaptureLimits {
                stdin,
                stdout,
                stderr,
                total: MAX_TOTAL_BYTES,
                force_process_group_kill_failure: false,
                timeout_floor: 10,
            },
        }
    }

    #[cfg(test)]
    pub(super) const fn for_test_with_total(
        root: PathBuf,
        stdin: u64,
        stdout: u64,
        stderr: u64,
        total: u64,
    ) -> Self {
        Self {
            root,
            limits: CaptureLimits {
                stdin,
                stdout,
                stderr,
                total,
                force_process_group_kill_failure: false,
                timeout_floor: 10,
            },
        }
    }

    #[cfg(test)]
    const fn with_forced_process_group_kill_failure(mut self) -> Self {
        self.limits.force_process_group_kill_failure = true;
        self
    }

    #[cfg(test)]
    const fn with_exact_test_timeout(mut self) -> Self {
        self.limits.timeout_floor = 0;
        self
    }
}

pub(super) fn capture_directory_is_configured() -> bool {
    std::env::var_os(CAPTURE_DIRECTORY_ENV).is_some_and(|path| !path.is_empty())
}

pub(super) struct CapturedFreshZ3Run {
    pub(super) outcome: SolverProcessOutcome,
    pub(super) solver_elapsed: Duration,
    pub(super) bundle: PendingCaptureBundle,
}

pub(super) struct PendingCaptureBundle {
    root: PathBuf,
    root_identity: FileIdentity,
    _budget_lock: RootBudgetLock,
    staging: PathBuf,
    staging_identity: FileIdentity,
    final_path: PathBuf,
    occurrence_id: String,
    stdin: Option<ArtifactReceipt>,
    stdout: Option<ArtifactReceipt>,
    stderr: Option<ArtifactReceipt>,
    stdin_bytes_written: u64,
    termination: Option<TerminationReceipt>,
    executable: Option<SolverExecutableReceipt>,
    args: Option<Vec<String>>,
    timeout_ms: Option<u64>,
    solver_elapsed: Duration,
    available_bytes: u64,
    written_bytes: u64,
    published: bool,
}

impl PendingCaptureBundle {
    fn create(
        root: &Path,
        occurrence: &TraceOccurrence,
        total_budget: u64,
    ) -> Result<Self, SymbolicError> {
        let root = prepare_private_root(root)?;
        let root_identity = private_directory_identity(&root)?;
        let budget_lock = RootBudgetLock::acquire(&root)?;
        verify_private_directory_identity(&root, root_identity)?;
        let usage = capture_root_usage(&root)?;
        if usage.bytes > total_budget {
            return Err(capture_error(format!(
                "fresh Z3 capture root {} already exceeds its {total_budget}-byte aggregate budget ({} bytes present)",
                root.display(),
                usage.bytes
            )));
        }
        if usage.entries.saturating_add(BUNDLE_TRANSIENT_ENTRIES) > MAX_ROOT_ENTRIES {
            return Err(capture_error(format!(
                "fresh Z3 capture root lacks room for a {BUNDLE_TRANSIENT_ENTRIES}-entry bundle under its {MAX_ROOT_ENTRIES}-entry audit limit"
            )));
        }
        let occurrence_id = occurrence.stem().to_owned();
        let staging = root.join(format!(".{occurrence_id}.tmp"));
        let final_path = root.join(&occurrence_id);
        create_private_directory(&staging)?;
        let staging_identity = private_directory_identity(&staging)?;
        Ok(Self {
            root,
            root_identity,
            _budget_lock: budget_lock,
            staging,
            staging_identity,
            final_path,
            occurrence_id,
            stdin: None,
            stdout: None,
            stderr: None,
            stdin_bytes_written: 0,
            termination: None,
            executable: None,
            args: None,
            timeout_ms: None,
            solver_elapsed: Duration::ZERO,
            available_bytes: total_budget - usage.bytes,
            written_bytes: 0,
            published: false,
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.staging.join(name)
    }

    fn write_artifact(
        &mut self,
        name: &str,
        relative_path: &'static str,
        bytes: &[u8],
    ) -> Result<ArtifactReceipt, SymbolicError> {
        let bytes_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let next = self
            .written_bytes
            .checked_add(bytes_len)
            .ok_or_else(|| capture_error("fresh Z3 capture bundle byte count overflowed u64"))?;
        if next > self.available_bytes {
            return Err(capture_error(format!(
                "fresh Z3 capture root {} would exceed its aggregate budget while writing {relative_path} ({} bytes available, {next} requested by this bundle)",
                self.root.display(),
                self.available_bytes
            )));
        }
        let receipt = write_artifact(self.path(name), relative_path, bytes)?;
        self.written_bytes = next;
        Ok(receipt)
    }
}

impl Drop for PendingCaptureBundle {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.staging);
        }
    }
}

#[cfg(unix)]
fn prepare_private_root(root: &Path) -> Result<PathBuf, SymbolicError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => validate_private_directory(root, &metadata)?,
        Err(error) if error.kind() == ErrorKind::NotFound => create_private_directory(root)?,
        Err(error) => {
            return Err(capture_error(format!(
                "failed to inspect fresh Z3 capture root {}: {error}",
                root.display()
            )));
        }
    }
    let original = fs::symlink_metadata(root).map_err(|error| {
        capture_error(format!(
            "failed to inspect fresh Z3 capture root {}: {error}",
            root.display()
        ))
    })?;
    validate_private_directory(root, &original)?;
    let canonical = fs::canonicalize(root).map_err(|error| {
        capture_error(format!(
            "failed to resolve fresh Z3 capture root {}: {error}",
            root.display()
        ))
    })?;
    let resolved = fs::symlink_metadata(&canonical).map_err(|error| {
        capture_error(format!(
            "failed to inspect resolved fresh Z3 capture root {}: {error}",
            canonical.display()
        ))
    })?;
    if FileIdentity::from_metadata(&original) != FileIdentity::from_metadata(&resolved) {
        return Err(capture_error(format!(
            "fresh Z3 capture root identity changed while resolving {}",
            root.display()
        )));
    }
    Ok(canonical)
}

#[cfg(not(unix))]
fn prepare_private_root(_root: &Path) -> Result<PathBuf, SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix private-directory semantics"))
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), SymbolicError> {
    let mut builder = DirBuilder::new();
    builder.mode(0o700).create(path).map_err(|error| {
        capture_error(format!(
            "failed to create private fresh Z3 capture directory {}: {error}",
            path.display()
        ))
    })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        capture_error(format!("failed to set private permissions on {}: {error}", path.display()))
    })?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        capture_error(format!(
            "failed to inspect private fresh Z3 capture directory {}: {error}",
            path.display()
        ))
    })?;
    validate_private_directory(path, &metadata)
}

#[cfg(not(unix))]
fn create_private_directory(_path: &Path) -> Result<(), SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix private-directory semantics"))
}

#[cfg(unix)]
fn validate_private_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), SymbolicError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(capture_error(format!(
            "fresh Z3 capture directory must not be a symlink: {}",
            path.display()
        )));
    }
    // SAFETY: geteuid(2) has no preconditions or failure mode.
    let owner = unsafe { libc::geteuid() };
    let mode = metadata.permissions().mode() & 0o777;
    if metadata.uid() != owner || mode != 0o700 {
        return Err(capture_error(format!(
            "fresh Z3 capture directory {} must be owned by uid {owner} with mode 0700; found uid {} mode {mode:04o}",
            path.display(),
            metadata.uid()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_directory(_path: &Path, _metadata: &fs::Metadata) -> Result<(), SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix private-directory semantics"))
}

#[cfg(unix)]
fn validate_private_file(path: &Path, metadata: &fs::Metadata) -> Result<(), SymbolicError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(capture_error(format!(
            "fresh Z3 capture artifact must be a regular file: {}",
            path.display()
        )));
    }
    // SAFETY: geteuid(2) has no preconditions or failure mode.
    let owner = unsafe { libc::geteuid() };
    let mode = metadata.permissions().mode() & 0o777;
    if metadata.uid() != owner || mode != 0o600 || metadata.nlink() != 1 {
        return Err(capture_error(format!(
            "fresh Z3 capture artifact {} must be singly linked, owned by uid {owner}, and mode 0600; found uid {} mode {mode:04o} links {}",
            path.display(),
            metadata.uid(),
            metadata.nlink()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file(_path: &Path, _metadata: &fs::Metadata) -> Result<(), SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix private-file semantics"))
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self { device: metadata.dev(), inode: metadata.ino() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct SolverExecutableReceipt {
    requested_path: String,
    canonical_path: String,
    bytes: u64,
    sha256: String,
    identity: SolverExecutableIdentity,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct SolverExecutableIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
    size_bytes: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(unix)]
impl SolverExecutableIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            links: metadata.nlink(),
            size_bytes: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct SolverExecutableIdentity;

#[cfg(unix)]
fn inspect_solver_executable(program: &str) -> Result<SolverExecutableReceipt, SymbolicError> {
    let requested = Path::new(program);
    if !requested.is_absolute() {
        return Err(capture_error(format!(
            "fresh Z3 capture solver path must be absolute: {program}"
        )));
    }
    let canonical = fs::canonicalize(requested).map_err(|error| {
        capture_error(format!("failed to resolve fresh Z3 executable {program}: {error}"))
    })?;
    let canonical_path = canonical
        .to_str()
        .ok_or_else(|| {
            capture_error(format!(
                "fresh Z3 executable has a non-UTF-8 canonical path: {}",
                canonical.display()
            ))
        })?
        .to_owned();
    let mut file = File::open(&canonical).map_err(|error| {
        capture_error(format!(
            "failed to open fresh Z3 executable {}: {error}",
            canonical.display()
        ))
    })?;
    let before_metadata = file.metadata().map_err(|error| {
        capture_error(format!(
            "failed to inspect fresh Z3 executable {}: {error}",
            canonical.display()
        ))
    })?;
    if !before_metadata.is_file() || before_metadata.file_type().is_symlink() {
        return Err(capture_error(format!(
            "fresh Z3 executable must resolve to a regular file: {}",
            canonical.display()
        )));
    }
    if before_metadata.mode() & 0o111 == 0 {
        return Err(capture_error(format!(
            "fresh Z3 executable is not executable: {}",
            canonical.display()
        )));
    }
    let identity = SolverExecutableIdentity::from_metadata(&before_metadata);
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            capture_error(format!(
                "failed to hash fresh Z3 executable {}: {error}",
                canonical.display()
            ))
        })?;
        if count == 0 {
            break;
        }
        bytes = bytes.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        digest.update(&buffer[..count]);
    }
    let after_metadata = file.metadata().map_err(|error| {
        capture_error(format!(
            "failed to re-inspect fresh Z3 executable {}: {error}",
            canonical.display()
        ))
    })?;
    if SolverExecutableIdentity::from_metadata(&after_metadata) != identity
        || bytes != identity.size_bytes
    {
        return Err(capture_error(format!(
            "fresh Z3 executable changed while it was hashed: {}",
            canonical.display()
        )));
    }
    let canonical_after = fs::canonicalize(requested).map_err(|error| {
        capture_error(format!("failed to re-resolve fresh Z3 executable {program}: {error}"))
    })?;
    let path_metadata = fs::symlink_metadata(&canonical).map_err(|error| {
        capture_error(format!(
            "failed to re-inspect fresh Z3 executable path {}: {error}",
            canonical.display()
        ))
    })?;
    if canonical_after != canonical
        || path_metadata.file_type().is_symlink()
        || SolverExecutableIdentity::from_metadata(&path_metadata) != identity
    {
        return Err(capture_error(format!(
            "fresh Z3 executable path or identity changed while it was hashed: {program}"
        )));
    }
    Ok(SolverExecutableReceipt {
        requested_path: program.to_owned(),
        canonical_path,
        bytes,
        sha256: hex_digest(digest.finalize().into()),
        identity,
    })
}

#[cfg(not(unix))]
fn inspect_solver_executable(_program: &str) -> Result<SolverExecutableReceipt, SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix executable identity semantics"))
}

fn verify_solver_executable(expected: &SolverExecutableReceipt) -> Result<(), SymbolicError> {
    let actual = inspect_solver_executable(&expected.requested_path)?;
    if actual != *expected {
        return Err(capture_error(format!(
            "fresh Z3 executable changed during the solver invocation: {}",
            expected.requested_path
        )));
    }
    Ok(())
}

fn private_directory_identity(path: &Path) -> Result<FileIdentity, SymbolicError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        capture_error(format!(
            "failed to inspect private fresh Z3 capture directory {}: {error}",
            path.display()
        ))
    })?;
    validate_private_directory(path, &metadata)?;
    Ok(FileIdentity::from_metadata(&metadata))
}

fn verify_private_directory_identity(
    path: &Path,
    expected: FileIdentity,
) -> Result<(), SymbolicError> {
    let actual = private_directory_identity(path)?;
    if actual != expected {
        return Err(capture_error(format!(
            "private fresh Z3 capture directory identity changed: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity;

#[cfg(not(unix))]
impl FileIdentity {
    const fn from_metadata(_metadata: &fs::Metadata) -> Self {
        Self
    }
}

struct RootBudgetLock {
    #[cfg(unix)]
    _file: File,
}

impl RootBudgetLock {
    #[cfg(unix)]
    fn acquire(root: &Path) -> Result<Self, SymbolicError> {
        let root_metadata = fs::symlink_metadata(root).map_err(|error| {
            capture_error(format!(
                "failed to inspect fresh Z3 capture root {}: {error}",
                root.display()
            ))
        })?;
        validate_private_directory(root, &root_metadata)?;
        let path = root.join(".capture-budget.lock");
        let file = open_private_control_file(&path)?;
        loop {
            // SAFETY: flock(2) only observes the valid descriptor owned by `file`.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                return Err(capture_error(format!(
                    "failed to lock fresh Z3 capture budget {}: {error}",
                    path.display()
                )));
            }
        }
    }

    #[cfg(not(unix))]
    fn acquire(_root: &Path) -> Result<Self, SymbolicError> {
        Err(capture_error("fresh Z3 capture requires Unix root-budget locking"))
    }
}

#[cfg(unix)]
fn open_private_control_file(path: &Path) -> Result<File, SymbolicError> {
    let file =
        match OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(path) {
            Ok(file) => {
                file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|error| {
                    capture_error(format!(
                        "failed to set private permissions on {}: {error}",
                        path.display()
                    ))
                })?;
                file.sync_all().map_err(|error| {
                    capture_error(format!("failed to sync {}: {error}", path.display()))
                })?;
                if let Some(parent) = path.parent() {
                    sync_directory(parent)?;
                }
                file
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let before = fs::symlink_metadata(path).map_err(|error| {
                    capture_error(format!(
                        "failed to inspect fresh Z3 capture control file {}: {error}",
                        path.display()
                    ))
                })?;
                validate_private_file(path, &before)?;
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(path)
                    .map_err(|error| {
                        capture_error(format!(
                            "failed to open fresh Z3 capture control file {}: {error}",
                            path.display()
                        ))
                    })?;
                let after = file.metadata().map_err(|error| {
                    capture_error(format!(
                        "failed to inspect opened fresh Z3 capture control file {}: {error}",
                        path.display()
                    ))
                })?;
                if FileIdentity::from_metadata(&before) != FileIdentity::from_metadata(&after) {
                    return Err(capture_error(format!(
                        "fresh Z3 capture control file identity changed while opening {}",
                        path.display()
                    )));
                }
                file
            }
            Err(error) => {
                return Err(capture_error(format!(
                    "failed to create fresh Z3 capture control file {}: {error}",
                    path.display()
                )));
            }
        };
    let metadata = file.metadata().map_err(|error| {
        capture_error(format!(
            "failed to inspect fresh Z3 capture control file {}: {error}",
            path.display()
        ))
    })?;
    validate_private_file(path, &metadata)?;
    if metadata.len() != 0 {
        return Err(capture_error(format!(
            "fresh Z3 capture control file must be empty: {}",
            path.display()
        )));
    }
    Ok(file)
}

#[derive(Clone, Copy)]
struct CaptureRootUsage {
    entries: usize,
    bytes: u64,
}

fn capture_root_usage(root: &Path) -> Result<CaptureRootUsage, SymbolicError> {
    let mut entries = 0_usize;
    let mut total = 0_u64;
    for entry in fs::read_dir(root).map_err(|error| {
        capture_error(format!(
            "failed to enumerate fresh Z3 capture root {}: {error}",
            root.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            capture_error(format!(
                "failed to enumerate fresh Z3 capture root {}: {error}",
                root.display()
            ))
        })?;
        let path = entry.path();
        add_capture_entry_bytes(&path, &mut entries, &mut total, true)?;
    }
    Ok(CaptureRootUsage { entries, bytes: total })
}

fn add_capture_entry_bytes(
    path: &Path,
    entries: &mut usize,
    total: &mut u64,
    allow_directory: bool,
) -> Result<(), SymbolicError> {
    *entries = entries.saturating_add(1);
    if *entries > MAX_ROOT_ENTRIES {
        return Err(capture_error(format!(
            "fresh Z3 capture root exceeds its {MAX_ROOT_ENTRIES}-entry audit limit"
        )));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        capture_error(format!(
            "failed to inspect fresh Z3 capture entry {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(capture_error(format!(
            "fresh Z3 capture root contains a symlink: {}",
            path.display()
        )));
    }
    if metadata.is_file() {
        validate_private_file(path, &metadata)?;
        *total = total
            .checked_add(metadata.len())
            .ok_or_else(|| capture_error("fresh Z3 capture root byte count overflowed u64"))?;
        return Ok(());
    }
    if !allow_directory || !metadata.is_dir() {
        return Err(capture_error(format!(
            "fresh Z3 capture root contains an unsupported entry: {}",
            path.display()
        )));
    }
    validate_private_directory(path, &metadata)?;
    for entry in fs::read_dir(path).map_err(|error| {
        capture_error(format!(
            "failed to enumerate fresh Z3 capture bundle {}: {error}",
            path.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            capture_error(format!(
                "failed to enumerate fresh Z3 capture bundle {}: {error}",
                path.display()
            ))
        })?;
        add_capture_entry_bytes(&entry.path(), entries, total, false)?;
    }
    Ok(())
}

struct PublicationClaim {
    root: PathBuf,
    path: PathBuf,
    remove_on_drop: bool,
}

impl PublicationClaim {
    fn acquire(root: &Path, occurrence_id: &str) -> Result<Self, SymbolicError> {
        let path = root.join(format!(".{occurrence_id}.publish"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&path).map_err(|error| {
            capture_error(format!(
                "failed to claim fresh Z3 capture occurrence {}: {error}",
                path.display()
            ))
        })?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|error| {
            capture_error(format!(
                "failed to set private permissions on fresh Z3 capture claim {}: {error}",
                path.display()
            ))
        })?;
        let metadata = file.metadata().map_err(|error| {
            capture_error(format!(
                "failed to inspect fresh Z3 capture claim {}: {error}",
                path.display()
            ))
        })?;
        validate_private_file(&path, &metadata)?;
        let claim = Self { root: root.to_path_buf(), path, remove_on_drop: true };
        file.sync_all().map_err(|error| {
            capture_error(format!(
                "failed to sync fresh Z3 capture publication claim {}: {error}",
                claim.path.display()
            ))
        })?;
        sync_directory(root)?;
        Ok(claim)
    }

    const fn preserve_on_drop(&mut self) {
        self.remove_on_drop = false;
    }

    fn finish(mut self) -> Result<(), SymbolicError> {
        fs::remove_file(&self.path).map_err(|error| {
            capture_error(format!(
                "failed to remove fresh Z3 capture publication claim {}: {error}",
                self.path.display()
            ))
        })?;
        self.remove_on_drop = false;
        sync_directory(&self.root)
    }
}

impl Drop for PublicationClaim {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn ensure_destination_absent(path: &Path) -> Result<(), SymbolicError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(capture_error(format!(
            "fresh Z3 capture occurrence already exists: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(capture_error(format!(
            "failed to inspect fresh Z3 capture destination {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn publish_directory_no_replace(source: &Path, destination: &Path) -> Result<(), SymbolicError> {
    let source_bytes = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        capture_error(format!("fresh Z3 capture source contains NUL: {}", source.display()))
    })?;
    let destination_bytes = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        capture_error(format!(
            "fresh Z3 capture destination contains NUL: {}",
            destination.display()
        ))
    })?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: both C strings remain live for the syscall, and AT_FDCWD selects their absolute
    // paths. Calling the syscall directly preserves Foundry's pre-glibc-2.28 runtime floor while
    // RENAME_NOREPLACE makes destination creation one atomic operation.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source_bytes.as_ptr(),
            libc::AT_FDCWD,
            destination_bytes.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_vendor = "apple")]
    // SAFETY: both C strings remain live for the syscall, and AT_FDCWD selects their absolute
    // paths. RENAME_EXCL makes destination creation one atomic operation.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source_bytes.as_ptr(),
            libc::AT_FDCWD,
            destination_bytes.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::AlreadyExists {
        return Err(capture_error(format!(
            "fresh Z3 capture occurrence already exists: {}",
            destination.display()
        )));
    }
    Err(capture_error(format!(
        "failed to atomically publish fresh Z3 capture {}: {error}",
        destination.display()
    )))
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn publish_directory_no_replace(_source: &Path, _destination: &Path) -> Result<(), SymbolicError> {
    Err(capture_error("fresh Z3 capture requires atomic no-replace directory publication"))
}

struct FreshProcessRun {
    outcome: SolverProcessOutcome,
    stdout: CapturedStream,
    stderr: CapturedStream,
    stdin_bytes_written: u64,
    termination: TerminationReceipt,
}

struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
    reached_eof: bool,
}

struct FreshChild {
    child: Child,
    reaped: bool,
    #[cfg(unix)]
    process_group: Option<i32>,
    #[cfg(test)]
    force_process_group_kill_failure: bool,
}

impl FreshChild {
    fn new(child: Child) -> Self {
        #[cfg(unix)]
        let process_group = i32::try_from(child.id()).ok();
        Self {
            child,
            reaped: false,
            #[cfg(unix)]
            process_group,
            #[cfg(test)]
            force_process_group_kill_failure: false,
        }
    }

    const fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    fn wait_timeout(&mut self, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.wait_timeout(timeout)?;
        if status.is_some() {
            self.reaped = true;
        }
        Ok(status)
    }

    fn terminate_and_reap(&mut self) -> (Option<ExitStatus>, Option<String>) {
        let group_error = self.terminate_process_group().err();
        let kill_error = self.child.kill().err();
        match self.wait_timeout(PROCESS_REAP_TIMEOUT) {
            Ok(Some(status)) => (Some(status), group_error),
            Ok(None) => {
                let mut error = format!(
                    "fresh Z3 process did not exit within {} ms after termination",
                    PROCESS_REAP_TIMEOUT.as_millis()
                );
                if let Some(group_error) = group_error {
                    error.push_str(&format!("; {group_error}"));
                }
                if let Some(kill_error) = kill_error {
                    error.push_str(&format!("; failed to kill process: {kill_error}"));
                }
                (None, Some(error))
            }
            Err(wait_error) => {
                let mut error = format!("failed to reap fresh Z3 process: {wait_error}");
                if let Some(group_error) = group_error {
                    error.push_str(&format!("; {group_error}"));
                }
                if let Some(kill_error) = kill_error {
                    error.push_str(&format!("; failed to kill process: {kill_error}"));
                }
                (None, Some(error))
            }
        }
    }

    #[cfg(unix)]
    fn terminate_process_group(&self) -> Result<(), String> {
        if self.reaped {
            return Err("refusing to signal a fresh Z3 process group after its leader was reaped"
                .to_string());
        }
        #[cfg(test)]
        if self.force_process_group_kill_failure {
            return Err("injected fresh Z3 process-group termination failure".to_string());
        }
        let Some(process_group) = self.process_group else {
            return Err("fresh Z3 process id does not fit a Unix process-group id".to_string());
        };
        // SAFETY: the child was placed in a new process group whose id is its positive process
        // id. Negating that id asks kill(2) to signal only that group.
        if unsafe { libc::kill(-process_group, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!("failed to kill fresh Z3 process group {process_group}: {error}"))
        }
    }

    #[cfg(not(unix))]
    const fn terminate_process_group(&self) -> Result<(), String> {
        Ok(())
    }
}

impl Drop for FreshChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.terminate_process_group();
            let _ = self.child.kill();
            let _ = self.wait_timeout(PROCESS_REAP_TIMEOUT);
        }
    }
}

struct IoCancellation {
    cancelled: Arc<AtomicBool>,
}

impl IoCancellation {
    fn new() -> Self {
        Self { cancelled: Arc::new(AtomicBool::new(false)) }
    }

    fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Drop for IoCancellation {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct ProcessStop {
    kind: &'static str,
    error: Option<String>,
}

impl ProcessStop {
    const fn new(kind: &'static str) -> Self {
        Self { kind, error: None }
    }

    fn with_error(kind: &'static str, error: impl Into<String>) -> Self {
        Self { kind, error: Some(error.into()) }
    }

    fn add_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        if let Some(existing) = &mut self.error {
            existing.push_str("; ");
            existing.push_str(&error);
        } else {
            self.error = Some(error);
        }
    }
}

fn run_fresh_process(
    command: &SolverCommand,
    executable: &str,
    smt: &[u8],
    timeout: u32,
    limits: CaptureLimits,
) -> Result<FreshProcessRun, SymbolicError> {
    let mut process = Command::new(executable);
    process
        .args(&command.args)
        .env_clear()
        .env("PATH", SOLVER_PATH)
        .current_dir(SOLVER_WORKING_DIRECTORY)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut process);
    let child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(FreshProcessRun {
                outcome: SolverProcessOutcome::Error(format!(
                    "failed to spawn `{}`: {error}",
                    command.display
                )),
                stdout: CapturedStream { bytes: Vec::new(), truncated: false, reached_eof: true },
                stderr: CapturedStream { bytes: Vec::new(), truncated: false, reached_eof: true },
                stdin_bytes_written: 0,
                termination: TerminationReceipt::error("spawn_error", error.to_string()),
            });
        }
    };

    let mut child = FreshChild::new(child);
    #[cfg(test)]
    {
        child.force_process_group_kill_failure = limits.force_process_group_kill_failure;
    }
    let stdin = child
        .child_mut()
        .stdin
        .take()
        .ok_or_else(|| capture_error("fresh Z3 process is missing its piped stdin"))?;
    let stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| capture_error("fresh Z3 process is missing its piped stdout"))?;
    let stderr = child
        .child_mut()
        .stderr
        .take()
        .ok_or_else(|| capture_error("fresh Z3 process is missing its piped stderr"))?;
    set_pipe_nonblocking(&stdin, "stdin")?;
    set_pipe_nonblocking(&stdout, "stdout")?;
    set_pipe_nonblocking(&stderr, "stderr")?;
    let output_limit = Arc::new(AtomicBool::new(false));
    let stream_error = Arc::new(AtomicBool::new(false));
    let stdout_eof = Arc::new(AtomicBool::new(false));
    let stderr_eof = Arc::new(AtomicBool::new(false));
    let stdin_failed = Arc::new(AtomicBool::new(false));
    let stdin_written = AtomicU64::new(0);
    let started = Instant::now();
    let timeout = Duration::from_secs(timeout.into());

    let (stdin_result, stdout_result, stderr_result, status, stop_reason) =
        std::thread::scope(|scope| {
            // This guard cancels every nonblocking pipe loop before scoped-thread teardown even if
            // the controller panics. Scoped joins therefore never depend on a descendant closing
            // inherited solver pipes.
            let io_cancellation = IoCancellation::new();
            let stdin_stop = Arc::clone(&stdin_failed);
            let stdin_written = &stdin_written;
            let stdin_cancelled = io_cancellation.flag();
            let stdin_thread = scope.spawn(move || {
                let result = write_solver_stdin(stdin, smt, stdin_written, &stdin_cancelled);
                if result.is_err() {
                    stdin_stop.store(true, Ordering::Release);
                }
                result
            });
            let stdout_limit = Arc::clone(&output_limit);
            let stdout_error = Arc::clone(&stream_error);
            let stdout_eof_thread = Arc::clone(&stdout_eof);
            let stdout_cancelled = io_cancellation.flag();
            let stdout_thread = scope.spawn(move || {
                capture_stream(
                    stdout,
                    "stdout",
                    limits.stdout,
                    &stdout_limit,
                    &stdout_error,
                    &stdout_cancelled,
                    &stdout_eof_thread,
                )
            });
            let stderr_limit = Arc::clone(&output_limit);
            let stderr_error = Arc::clone(&stream_error);
            let stderr_eof_thread = Arc::clone(&stderr_eof);
            let stderr_cancelled = io_cancellation.flag();
            let stderr_thread = scope.spawn(move || {
                capture_stream(
                    stderr,
                    "stderr",
                    limits.stderr,
                    &stderr_limit,
                    &stderr_error,
                    &stderr_cancelled,
                    &stderr_eof_thread,
                )
            });

            let mut status = None;
            let stop_reason = loop {
                if output_limit.load(Ordering::Acquire) {
                    break Some(ProcessStop::new("output_limit"));
                }
                if stream_error.load(Ordering::Acquire) {
                    break Some(ProcessStop::new("capture_error"));
                }
                if stdin_failed.load(Ordering::Acquire) {
                    break Some(ProcessStop::new("stdin_error"));
                }
                let Some(wait) = solver_wait_duration(started.elapsed(), Some(timeout)) else {
                    break Some(ProcessStop::new("timed_out"));
                };
                match child.wait_timeout(wait) {
                    Ok(Some(exit)) => {
                        status = Some(exit);
                        break None;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        break Some(ProcessStop::with_error(
                            "wait_error",
                            format!("failed to wait for fresh Z3 process: {error}"),
                        ));
                    }
                }
            };
            let mut stop_reason = stop_reason;
            if let Some(reason) = &mut stop_reason {
                let (exit, error) = child.terminate_and_reap();
                status = exit;
                if let Some(error) = error {
                    reason.add_error(error);
                }
                // Keep the readers live while process-group termination closes every owned pipe.
                // Cancelling first can silently omit bytes already buffered by the kernel.
                wait_for_stream_eof(&stdout_eof, &stderr_eof);
            } else {
                // `wait_timeout` reaped the process-group leader. Do not signal its old numeric
                // process-group id: the kernel may already have reused it for an unrelated group.
                // A canonical Z3 process closes both pipes when it exits. Inherited pipes that do
                // not reach EOF are attested as incomplete and rejected below.
                wait_for_stream_eof(&stdout_eof, &stderr_eof);
                if !stdout_eof.load(Ordering::Acquire) || !stderr_eof.load(Ordering::Acquire) {
                    stop_reason = Some(ProcessStop::with_error(
                        "cleanup_error",
                        "fresh Z3 capture pipes did not reach EOF after the solver exited",
                    ));
                }
            }
            io_cancellation.cancel();

            (stdin_thread.join(), stdout_thread.join(), stderr_thread.join(), status, stop_reason)
        });

    let stdin_bytes_written = stdin_written.load(Ordering::Acquire);
    let stdin_error = flatten_thread_result(stdin_result, "stdin writer").err();
    let stdout = flatten_thread_result(stdout_result, "stdout capture").map_err(capture_error)?;
    let stderr = flatten_thread_result(stderr_result, "stderr capture").map_err(capture_error)?;
    let stdout_text = String::from_utf8_lossy(&stdout.bytes).into_owned();
    let stderr_text = String::from_utf8_lossy(&stderr.bytes).into_owned();

    let mut stop_reason = stop_reason;
    if (stdout.truncated || stderr.truncated)
        && stop_reason.as_ref().is_none_or(|reason| reason.kind == "cleanup_error")
    {
        stop_reason = Some(ProcessStop::new("output_limit"));
    }
    if stop_reason.is_none() && (!stdout.reached_eof || !stderr.reached_eof) {
        stop_reason = Some(ProcessStop::with_error(
            "cleanup_error",
            "fresh Z3 capture pipes did not reach EOF after process-group cleanup",
        ));
    }
    if stop_reason.is_none() && stdin_bytes_written != u64::try_from(smt.len()).unwrap_or(u64::MAX)
    {
        stop_reason = Some(ProcessStop::with_error(
            "stdin_error",
            format!(
                "fresh Z3 process exited after reading {stdin_bytes_written} of {} stdin bytes",
                smt.len()
            ),
        ));
    }
    if let Some(error) = stdin_error.as_deref() {
        if let Some(reason) = &mut stop_reason {
            if reason.kind == "stdin_error" {
                reason.add_error(error);
            }
        } else {
            stop_reason = Some(ProcessStop::with_error("stdin_error", error));
        }
    }
    if let Some(reason) = stop_reason {
        let outcome = if reason.kind == "timed_out" && reason.error.is_none() {
            SolverProcessOutcome::Unknown
        } else {
            let detail =
                reason.error.as_deref().map(|error| format!(": {error}")).unwrap_or_default();
            SolverProcessOutcome::Error(format!(
                "fresh Z3 capture stopped: {}{detail}",
                reason.kind
            ))
        };
        return Ok(FreshProcessRun {
            outcome,
            stdout,
            stderr,
            stdin_bytes_written,
            termination: TerminationReceipt::stopped(reason, status),
        });
    }
    let Some(status) = status else {
        return Ok(FreshProcessRun {
            outcome: SolverProcessOutcome::Error(
                "fresh Z3 process completed without an exit status".to_string(),
            ),
            stdout,
            stderr,
            stdin_bytes_written,
            termination: TerminationReceipt::error(
                "wait_error",
                "missing process exit status".to_string(),
            ),
        });
    };
    let termination = TerminationReceipt::exited(&status);
    let outcome = if status.success() {
        SolverProcessOutcome::Output(stdout_text)
    } else {
        SolverProcessOutcome::Error(solver_exit_error(command, status, &stdout_text, &stderr_text))
    };
    Ok(FreshProcessRun { outcome, stdout, stderr, stdin_bytes_written, termination })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
const fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
const fn ensure_process_group_support() -> Result<(), SymbolicError> {
    Ok(())
}

#[cfg(not(unix))]
fn ensure_process_group_support() -> Result<(), SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix process-group isolation"))
}

#[cfg(unix)]
fn set_pipe_nonblocking(pipe: &impl AsRawFd, label: &str) -> Result<(), SymbolicError> {
    let descriptor = pipe.as_raw_fd();
    // SAFETY: both fcntl(2) invocations operate on the live pipe descriptor borrowed above.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(capture_error(format!(
            "failed to inspect fresh Z3 {label} pipe flags: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: the descriptor remains live and the existing status flags are preserved.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(capture_error(format!(
            "failed to make fresh Z3 {label} pipe nonblocking: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_pipe_nonblocking<T>(_pipe: &T, _label: &str) -> Result<(), SymbolicError> {
    Err(capture_error("fresh Z3 capture requires Unix nonblocking-pipe semantics"))
}

fn write_solver_stdin(
    mut stdin: std::process::ChildStdin,
    mut input: &[u8],
    written: &AtomicU64,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    while !input.is_empty() {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let count = match stdin.write(input) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(IO_CANCEL_CHECK_INTERVAL);
                continue;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("failed to write fresh Z3 stdin: {error}")),
        };
        if count == 0 {
            return Err("failed to write fresh Z3 stdin: zero-byte write".to_string());
        }
        written.fetch_add(u64::try_from(count).unwrap_or(u64::MAX), Ordering::Release);
        input = &input[count..];
    }
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        match stdin.flush() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(IO_CANCEL_CHECK_INTERVAL);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("failed to flush fresh Z3 stdin: {error}")),
        }
    }
}

fn capture_stream(
    mut source: impl Read,
    label: &'static str,
    limit: u64,
    output_limit: &AtomicBool,
    stream_error: &AtomicBool,
    cancelled: &AtomicBool,
    eof: &AtomicBool,
) -> Result<CapturedStream, String> {
    let result = (|| {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        let mut reached_eof = false;
        loop {
            let count = match source.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(IO_CANCEL_CHECK_INTERVAL);
                    continue;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("failed to read {label}: {error}")),
            };
            if count == 0 {
                reached_eof = true;
                eof.store(true, Ordering::Release);
                break;
            }
            let count_u64 = u64::try_from(count).unwrap_or(u64::MAX);
            let retained_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let remaining = limit.saturating_sub(retained_bytes);
            let retained = usize::try_from(remaining.min(count_u64)).unwrap_or(count);
            if retained > 0 {
                bytes
                    .try_reserve(retained)
                    .map_err(|_| format!("failed to reserve bounded {label} capture"))?;
                bytes.extend_from_slice(&buffer[..retained]);
            }
            if retained < count {
                truncated = true;
                output_limit.store(true, Ordering::Release);
                break;
            }
        }
        Ok(CapturedStream { bytes, truncated, reached_eof })
    })();
    if result.is_err() {
        stream_error.store(true, Ordering::Release);
    }
    result
}

fn wait_for_stream_eof(stdout_eof: &AtomicBool, stderr_eof: &AtomicBool) {
    let started = Instant::now();
    while !(stdout_eof.load(Ordering::Acquire) && stderr_eof.load(Ordering::Acquire))
        && started.elapsed() < PROCESS_REAP_TIMEOUT
    {
        std::thread::sleep(IO_CANCEL_CHECK_INTERVAL);
    }
}

fn flatten_thread_result<T>(
    result: std::thread::Result<Result<T, String>>,
    label: &str,
) -> Result<T, String> {
    result.map_err(|_| format!("{label} thread panicked"))?
}

fn write_artifact(
    path: PathBuf,
    relative_path: &'static str,
    bytes: &[u8],
) -> Result<ArtifactReceipt, SymbolicError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|error| capture_error(format!("failed to create {}: {error}", path.display())))?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|error| {
        capture_error(format!("failed to set private permissions on {}: {error}", path.display()))
    })?;
    file.write_all(bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
        .map_err(|error| capture_error(format!("failed to write {}: {error}", path.display())))?;
    let metadata = file.metadata().map_err(|error| {
        capture_error(format!("failed to inspect written artifact {}: {error}", path.display()))
    })?;
    validate_private_file(&path, &metadata)?;
    Ok(ArtifactReceipt {
        path: relative_path,
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        sha256: hex_digest(Sha256::digest(bytes).into()),
        truncated: false,
        reached_eof: None,
        identity: FileIdentity::from_metadata(&metadata),
    })
}

fn verify_artifacts<const N: usize>(
    directory: &Path,
    artifacts: [&ArtifactReceipt; N],
) -> Result<(), SymbolicError> {
    for artifact in artifacts {
        verify_artifact(directory, artifact)?;
    }
    Ok(())
}

fn verify_bundle<const N: usize>(
    directory: &Path,
    expected_identity: FileIdentity,
    artifacts: [&ArtifactReceipt; N],
) -> Result<(), SymbolicError> {
    verify_private_directory_identity(directory, expected_identity)?;
    let mut expected = artifacts.iter().map(|artifact| artifact.path).collect::<Vec<_>>();
    expected.sort_unstable();
    let mut actual = fs::read_dir(directory)
        .map_err(|error| {
            capture_error(format!(
                "failed to enumerate fresh Z3 capture bundle {}: {error}",
                directory.display()
            ))
        })?
        .map(|entry| {
            entry
                .map_err(|error| {
                    capture_error(format!(
                        "failed to enumerate fresh Z3 capture bundle {}: {error}",
                        directory.display()
                    ))
                })?
                .file_name()
                .into_string()
                .map_err(|_| {
                    capture_error(format!(
                        "fresh Z3 capture bundle contains a non-UTF-8 entry: {}",
                        directory.display()
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    actual.sort_unstable();
    if actual.iter().map(String::as_str).collect::<Vec<_>>() != expected {
        return Err(capture_error(format!(
            "fresh Z3 capture bundle {} does not contain exactly its attested artifacts",
            directory.display()
        )));
    }
    verify_artifacts(directory, artifacts)?;
    verify_private_directory_identity(directory, expected_identity)
}

fn verify_artifact(directory: &Path, artifact: &ArtifactReceipt) -> Result<(), SymbolicError> {
    let path = directory.join(artifact.path);
    let before = fs::symlink_metadata(&path).map_err(|error| {
        capture_error(format!("failed to inspect capture artifact {}: {error}", path.display()))
    })?;
    validate_private_file(&path, &before)?;
    if FileIdentity::from_metadata(&before) != artifact.identity || before.len() != artifact.bytes {
        return Err(capture_error(format!(
            "fresh Z3 capture artifact identity or size changed before verification: {}",
            path.display()
        )));
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(&path).map_err(|error| {
        capture_error(format!("failed to open capture artifact {}: {error}", path.display()))
    })?;
    let opened = file.metadata().map_err(|error| {
        capture_error(format!("failed to inspect opened artifact {}: {error}", path.display()))
    })?;
    validate_private_file(&path, &opened)?;
    if FileIdentity::from_metadata(&opened) != artifact.identity || opened.len() != artifact.bytes {
        return Err(capture_error(format!(
            "fresh Z3 capture artifact identity or size changed while opening: {}",
            path.display()
        )));
    }

    let read_limit = artifact.bytes.checked_add(1).ok_or_else(|| {
        capture_error(format!("fresh Z3 capture artifact size overflow: {}", path.display()))
    })?;
    let mut contents = Vec::new();
    Read::by_ref(&mut file).take(read_limit).read_to_end(&mut contents).map_err(|error| {
        capture_error(format!("failed to verify capture artifact {}: {error}", path.display()))
    })?;
    if u64::try_from(contents.len()).unwrap_or(u64::MAX) != artifact.bytes
        || hex_digest(Sha256::digest(&contents).into()) != artifact.sha256
    {
        return Err(capture_error(format!(
            "fresh Z3 capture artifact contents changed before publication: {}",
            path.display()
        )));
    }

    let after = fs::symlink_metadata(&path).map_err(|error| {
        capture_error(format!("failed to re-inspect capture artifact {}: {error}", path.display()))
    })?;
    validate_private_file(&path, &after)?;
    if FileIdentity::from_metadata(&after) != artifact.identity || after.len() != artifact.bytes {
        return Err(capture_error(format!(
            "fresh Z3 capture artifact identity or size changed after verification: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), SymbolicError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| capture_error(format!("failed to sync {}: {error}", path.display())))
}

#[cfg(not(unix))]
const fn sync_directory(_path: &Path) -> Result<(), SymbolicError> {
    Ok(())
}

#[derive(Serialize)]
struct CaptureManifest<'a> {
    schema: &'static str,
    schema_version: u32,
    occurrence_id: &'a str,
    query_trace: &'a ArtifactReceipt,
    solver: SolverReceipt<'a>,
    stdin: &'a ArtifactReceipt,
    stdout: &'a ArtifactReceipt,
    stderr: &'a ArtifactReceipt,
    stdin_bytes_written: u64,
    termination: &'a TerminationReceipt,
    solver_wall_time_ns: u64,
    classified_outcome: &'static str,
}

#[derive(Serialize)]
struct SolverReceipt<'a> {
    executable: &'a SolverExecutableReceipt,
    args: &'a [String],
    timeout_ms: u64,
    working_directory: &'static str,
    environment: SolverEnvironmentReceipt,
}

#[derive(Serialize)]
struct SolverEnvironmentReceipt {
    clear_inherited: bool,
    path: &'static str,
}

#[derive(Serialize)]
struct ArtifactReceipt {
    path: &'static str,
    bytes: u64,
    sha256: String,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reached_eof: Option<bool>,
    #[serde(skip)]
    identity: FileIdentity,
}

#[derive(Serialize)]
struct TerminationReceipt {
    kind: &'static str,
    exit_code: Option<i32>,
    signal: Option<i32>,
    error: Option<String>,
}

impl TerminationReceipt {
    fn exited(status: &ExitStatus) -> Self {
        Self {
            kind: if status.code().is_some() { "exited" } else { "signaled" },
            exit_code: status.code(),
            signal: exit_signal(status),
            error: None,
        }
    }

    fn stopped(reason: ProcessStop, status: Option<ExitStatus>) -> Self {
        Self {
            kind: reason.kind,
            exit_code: status.as_ref().and_then(ExitStatus::code),
            signal: status.as_ref().and_then(exit_signal),
            error: reason.error,
        }
    }

    const fn error(kind: &'static str, error: String) -> Self {
        Self { kind, exit_code: None, signal: None, error: Some(error) }
    }
}

#[cfg(unix)]
fn exit_signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
const fn exit_signal(_status: &ExitStatus) -> Option<i32> {
    None
}

fn hex_digest(digest: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn capture_error(message: impl Into<String>) -> SymbolicError {
    SymbolicError::Solver(message.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory {
        parent: PathBuf,
        root: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let parent = std::env::temp_dir().join(format!(
                "foundry-fresh-z3-capture-{name}-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir(&parent).unwrap();
            let root = parent.join("captures");
            Self { parent, root }
        }

        fn fake_z3(&self, script: &str) -> SolverCommand {
            let path = self.parent.join("z3");
            fs::write(&path, script).unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions).unwrap();
            SolverCommand::new(
                vec![path.display().to_string(), "-in".to_string(), "-smt2".to_string()],
                true,
            )
            .unwrap()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.parent);
        }
    }

    fn encoded_trace(occurrence: TraceOccurrence, outcome: &'static str) -> EncodedQueryTrace {
        EncodedQueryTrace {
            occurrence,
            bytes: format!("{{\"baseline\":{{\"outcome\":\"{outcome}\"}}}}\n").into_bytes(),
            outcome,
        }
    }

    fn read_manifest(directory: &TestDirectory, occurrence: &str) -> Value {
        serde_json::from_slice(
            &fs::read(directory.root.join(occurrence).join("capture.manifest.json")).unwrap(),
        )
        .unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn publishes_exact_occurrence_bound_streams() {
        let directory = TestDirectory::new("exact");
        let command = directory
            .fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\nprintf 'warning\\n' >&2\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        capture.validate_command(std::slice::from_ref(&command), Some(2)).unwrap();
        let occurrence = TraceOccurrence::for_test("backend-test-exact");
        let input = b"(set-logic QF_BV)\n(check-sat)\n";

        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, input, 2, &occurrence).unwrap();
        assert!(bundle.staging.starts_with(fs::canonicalize(&directory.root).unwrap()));
        assert_eq!(mode(&directory.root), 0o700);
        assert_eq!(mode(&bundle.staging), 0o700);
        assert_eq!(mode(&bundle.path("stdin.smt2")), 0o600);
        assert_eq!(outcome.into_result().unwrap(), "sat\n");
        capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap();

        let published = directory.root.join("backend-test-exact");
        assert_eq!(mode(&published), 0o700);
        for artifact in
            ["stdin.smt2", "stdout.bin", "stderr.bin", "query.json", "capture.manifest.json"]
        {
            assert_eq!(mode(&published.join(artifact)), 0o600, "{artifact}");
        }
        assert_eq!(fs::read(published.join("stdin.smt2")).unwrap(), input);
        assert_eq!(fs::read(published.join("stdout.bin")).unwrap(), b"sat\n");
        assert_eq!(fs::read(published.join("stderr.bin")).unwrap(), b"warning\n");
        let manifest = read_manifest(&directory, "backend-test-exact");
        assert_eq!(manifest["schema"], CAPTURE_SCHEMA);
        assert_eq!(manifest["schema_version"], CAPTURE_SCHEMA_VERSION);
        assert_eq!(manifest["occurrence_id"], "backend-test-exact");
        assert_eq!(manifest["stdin_bytes_written"], input.len());
        assert_eq!(manifest["termination"]["kind"], "exited");
        assert_eq!(manifest["termination"]["exit_code"], 0);
        assert_eq!(manifest["classified_outcome"], "sat");
        assert_eq!(manifest["stdout"]["reached_eof"], true);
        assert_eq!(manifest["stderr"]["reached_eof"], true);
        assert_eq!(manifest["solver"]["executable"]["requested_path"], command.program());
        assert_eq!(
            manifest["solver"]["executable"]["canonical_path"],
            fs::canonicalize(command.program()).unwrap().to_str().unwrap()
        );
        assert_eq!(
            manifest["solver"]["executable"]["sha256"],
            hex_digest(Sha256::digest(fs::read(command.program()).unwrap()).into())
        );
        assert_eq!(manifest["solver"]["working_directory"], SOLVER_WORKING_DIRECTORY);
        assert_eq!(manifest["solver"]["environment"]["clear_inherited"], true);
        assert_eq!(manifest["solver"]["environment"]["path"], SOLVER_PATH);
        assert_eq!(manifest["stdout"]["sha256"], hex_digest(Sha256::digest(b"sat\n").into()));
        assert_eq!(
            manifest["query_trace"]["sha256"],
            hex_digest(Sha256::digest(fs::read(published.join("query.json")).unwrap()).into())
        );
    }

    #[test]
    fn rejects_non_private_or_symlink_capture_roots() {
        let unsafe_directory = TestDirectory::new("unsafe-root");
        fs::create_dir(&unsafe_directory.root).unwrap();
        fs::set_permissions(&unsafe_directory.root, fs::Permissions::from_mode(0o755)).unwrap();
        let command = unsafe_directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(unsafe_directory.root.clone(), 1024, 1024, 1024);
        let error = match capture.run(
            &command,
            b"(check-sat)\n",
            2,
            &TraceOccurrence::for_test("backend-test-unsafe-root"),
        ) {
            Ok(_) => panic!("unsafe capture root was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("mode 0700"));

        let symlink_directory = TestDirectory::new("symlink-root");
        let target = symlink_directory.parent.join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&target, &symlink_directory.root).unwrap();
        let command = symlink_directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(symlink_directory.root.clone(), 1024, 1024, 1024);
        let error = match capture.run(
            &command,
            b"(check-sat)\n",
            2,
            &TraceOccurrence::for_test("backend-test-symlink-root"),
        ) {
            Ok(_) => panic!("symlink capture root was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("must not be a symlink"));
    }

    #[test]
    fn aggregate_budget_includes_prior_instances_and_published_bundles() {
        let directory = TestDirectory::new("aggregate-budget");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let first_capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let first_occurrence = TraceOccurrence::for_test("backend-test-budget-first");
        let CapturedFreshZ3Run { bundle, .. } =
            first_capture.run(&command, b"(check-sat)\n", 2, &first_occurrence).unwrap();
        first_capture.commit(bundle, encoded_trace(first_occurrence, "sat")).unwrap();

        let existing_bytes = capture_root_usage(&directory.root).unwrap().bytes;
        assert!(existing_bytes > 0);
        let second_capture = FreshZ3Capture::for_test_with_total(
            directory.root.clone(),
            1024,
            1024,
            1024,
            existing_bytes,
        );
        let second_occurrence = TraceOccurrence::for_test("backend-test-budget-second");
        let error = match second_capture.run(&command, b"(check-sat)\n", 2, &second_occurrence) {
            Ok(_) => panic!("over-budget staging write was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("aggregate budget"));
        assert!(directory.root.join("backend-test-budget-first").is_dir());
        assert!(!directory.root.join("backend-test-budget-second").exists());
    }

    #[test]
    fn aggregate_budget_counts_stale_staging_artifacts_before_running() {
        let directory = TestDirectory::new("stale-staging-budget");
        let root = prepare_private_root(&directory.root).unwrap();
        let stale = root.join(".stale.tmp");
        create_private_directory(&stale).unwrap();
        write_artifact(stale.join("stale.bin"), "stale.bin", b"1234567890").unwrap();
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test_with_total(root, 1024, 1024, 1024, 10);
        let occurrence = TraceOccurrence::for_test("backend-test-stale-staging-budget");

        let error = match capture.run(&command, b"(check-sat)\n", 2, &occurrence) {
            Ok(_) => panic!("stale staging bytes were omitted from the root budget"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("aggregate budget"));
    }

    #[test]
    fn query_trace_cannot_bypass_aggregate_budget() {
        let directory = TestDirectory::new("query-trace-budget");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let input = b"(check-sat)\n";
        let total = u64::try_from(input.len() + b"sat\n".len()).unwrap();
        let capture =
            FreshZ3Capture::for_test_with_total(directory.root.clone(), 1024, 1024, 1024, total);
        let occurrence = TraceOccurrence::for_test("backend-test-query-trace-budget");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, input, 2, &occurrence).unwrap();

        let error = capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap_err();
        assert!(error.to_string().contains("query.json"));
        assert!(!directory.root.join("backend-test-query-trace-budget").exists());
        assert!(!directory.root.join(".backend-test-query-trace-budget.tmp").exists());
    }

    #[test]
    fn artifact_mutation_is_rejected_before_manifest_publication() {
        let directory = TestDirectory::new("artifact-mutation");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-artifact-mutation");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        fs::write(bundle.path("stdin.smt2"), b"(check-xxx)\n").unwrap();

        let error = capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap_err();
        assert!(error.to_string().contains("contents changed"));
        assert!(!directory.root.join("backend-test-artifact-mutation").exists());
    }

    #[test]
    fn artifact_replacement_is_rejected_immediately_before_rename() {
        let directory = TestDirectory::new("artifact-replacement");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-artifact-replacement");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();

        let error = capture
            .commit_inner(
                bundle,
                encoded_trace(occurrence, "sat"),
                |staging| {
                    let bytes = fs::read(staging.join("stdout.bin")).map_err(|error| {
                        capture_error(format!("failed to read test artifact: {error}"))
                    })?;
                    let replacement = staging.join("replacement.bin");
                    let _ = write_artifact(replacement.clone(), "stdout.bin", &bytes)?;
                    fs::rename(replacement, staging.join("stdout.bin")).map_err(|error| {
                        capture_error(format!("failed to replace test artifact: {error}"))
                    })
                },
                |_| Ok(()),
            )
            .unwrap_err();
        assert!(error.to_string().contains("identity or size changed"));
        assert!(!directory.root.join("backend-test-artifact-replacement").exists());
    }

    #[test]
    fn atomic_publication_does_not_replace_a_racing_destination() {
        let directory = TestDirectory::new("atomic-collision");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-atomic-collision");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        let final_path = bundle.final_path.clone();

        let error = capture
            .commit_inner(
                bundle,
                encoded_trace(occurrence, "sat"),
                |_| {
                    fs::create_dir(&final_path).map_err(|error| {
                        capture_error(format!("failed to create racing destination: {error}"))
                    })?;
                    fs::set_permissions(&final_path, fs::Permissions::from_mode(0o700)).map_err(
                        |error| {
                            capture_error(format!("failed to secure racing destination: {error}"))
                        },
                    )
                },
                |_| Ok(()),
            )
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert_eq!(fs::read_dir(&final_path).unwrap().count(), 0);
    }

    #[test]
    fn post_publication_mutation_is_detected_before_success() {
        let directory = TestDirectory::new("post-publication-mutation");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-post-publication-mutation");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();

        let error = capture
            .commit_inner(
                bundle,
                encoded_trace(occurrence, "sat"),
                |_| Ok(()),
                |published| {
                    fs::write(published.join("stdout.bin"), b"bad\n").map_err(|error| {
                        capture_error(format!("failed to mutate published artifact: {error}"))
                    })
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("contents changed"));
        assert!(directory.root.join(".backend-test-post-publication-mutation.publish").is_file());
    }

    #[test]
    fn timeout_kills_reaps_and_seals_partial_output() {
        let directory = TestDirectory::new("timeout");
        let command = directory
            .fake_z3("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' \"$$\"\n/bin/sleep 5 &\nwait\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024)
            .with_exact_test_timeout();
        let occurrence = TraceOccurrence::for_test("backend-test-timeout");

        let started = Instant::now();
        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 1, &occurrence).unwrap();
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(matches!(outcome, SolverProcessOutcome::Unknown));
        capture.commit(bundle, encoded_trace(occurrence, "unknown")).unwrap();

        let solver_pid = String::from_utf8(
            fs::read(directory.root.join("backend-test-timeout/stdout.bin")).unwrap(),
        )
        .unwrap();
        let solver_pid = solver_pid.trim();
        assert!(
            !Command::new("/bin/kill")
                .args(["-0", solver_pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let manifest = read_manifest(&directory, "backend-test-timeout");
        assert_eq!(manifest["termination"]["kind"], "timed_out");
        assert_eq!(manifest["stdout"]["truncated"], false);
        assert_eq!(manifest["stdout"]["reached_eof"], true);
        assert_eq!(manifest["stderr"]["reached_eof"], true);
    }

    #[test]
    fn closed_stdin_stops_promptly_and_is_not_a_timeout() {
        let directory = TestDirectory::new("stdin-error");
        let command = directory.fake_z3("#!/bin/sh\nexec 0<&-\nexec /bin/sleep 5\n");
        let input = vec![b'x'; 1024 * 1024];
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 2 * 1024 * 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-stdin-error");

        let started = Instant::now();
        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, &input, 5, &occurrence).unwrap();
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(matches!(outcome, SolverProcessOutcome::Error(_)));
        capture.commit(bundle, encoded_trace(occurrence, "error")).unwrap();

        let manifest = read_manifest(&directory, "backend-test-stdin-error");
        assert_eq!(manifest["termination"]["kind"], "stdin_error");
        assert!(manifest["termination"]["error"].as_str().unwrap().contains("stdin"));
        assert!(
            manifest["stdin_bytes_written"].as_u64().unwrap() < u64::try_from(input.len()).unwrap()
        );
    }

    #[test]
    fn reaped_wrapper_with_inherited_pipes_is_rejected_without_group_signal() {
        let directory = TestDirectory::new("exited-wrapper");
        let command = directory.fake_z3(
            "#!/bin/sh\ncat >/dev/null\n/bin/sleep 30 &\nprintf '%s\\n' \"$!\"\nprintf 'sat\\n'\nexit 0\n",
        );
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-exited-wrapper");

        let started = Instant::now();
        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        let descendant_pid = String::from_utf8(fs::read(bundle.path("stdout.bin")).unwrap())
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        let _ = Command::new("/bin/kill")
            .args(["-KILL", descendant_pid.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(matches!(outcome, SolverProcessOutcome::Error(_)));
        capture.commit(bundle, encoded_trace(occurrence, "error")).unwrap();

        let manifest = read_manifest(&directory, "backend-test-exited-wrapper");
        assert_eq!(manifest["termination"]["kind"], "cleanup_error");
        assert_eq!(manifest["termination"]["exit_code"], 0);
        assert!(manifest["termination"]["error"].as_str().unwrap().contains("did not reach EOF"));
        assert_eq!(manifest["stdout"]["truncated"], false);
        assert_eq!(manifest["stdout"]["reached_eof"], false);
    }

    #[test]
    fn normal_exit_never_signals_a_reaped_process_group() {
        let directory = TestDirectory::new("normal-exit-no-group-signal");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024)
            .with_forced_process_group_kill_failure();
        let occurrence = TraceOccurrence::for_test("backend-test-normal-exit-no-group-signal");

        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        assert_eq!(outcome.into_result().unwrap(), "sat\n");
        capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap();

        let manifest = read_manifest(&directory, "backend-test-normal-exit-no-group-signal");
        assert_eq!(manifest["termination"]["kind"], "exited");
        assert_eq!(manifest["stdout"]["reached_eof"], true);
        assert_eq!(manifest["stderr"]["reached_eof"], true);
    }

    #[test]
    fn escaped_descendant_holding_pipes_is_a_cleanup_error() {
        let directory = TestDirectory::new("escaped-descendant");
        let command = directory.fake_z3(concat!(
            "#!/usr/bin/env python3\n",
            "import os\n",
            "import sys\n",
            "import time\n",
            "sys.stdin.buffer.read()\n",
            "ready_read, ready_write = os.pipe()\n",
            "child = os.fork()\n",
            "if child == 0:\n",
            "    os.close(ready_read)\n",
            "    os.setsid()\n",
            "    os.write(ready_write, b'1')\n",
            "    os.close(ready_write)\n",
            "    time.sleep(30)\n",
            "    os._exit(0)\n",
            "os.close(ready_write)\n",
            "os.read(ready_read, 1)\n",
            "os.close(ready_read)\n",
            "sys.stdout.write(f'{child}\\nsat\\n')\n",
            "sys.stdout.flush()\n",
        ));
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-escaped-descendant");

        let started = Instant::now();
        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        let descendant_pid = String::from_utf8(fs::read(bundle.path("stdout.bin")).unwrap())
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        let _ = Command::new("/bin/kill")
            .args(["-KILL", descendant_pid.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(matches!(outcome, SolverProcessOutcome::Error(_)));
        capture.commit(bundle, encoded_trace(occurrence, "error")).unwrap();

        let manifest = read_manifest(&directory, "backend-test-escaped-descendant");
        assert_eq!(manifest["termination"]["kind"], "cleanup_error");
        assert!(manifest["termination"]["error"].as_str().unwrap().contains("did not reach EOF"));
        assert_eq!(manifest["stdout"]["truncated"], false);
        assert_eq!(manifest["stdout"]["reached_eof"], false);
    }

    #[test]
    fn process_group_kill_failure_cannot_strand_pipe_joins() {
        let directory = TestDirectory::new("kill-failure");
        let descendant_pid = directory.parent.join("descendant.pid");
        let command = directory.fake_z3(&format!(
            "#!/bin/sh\n/bin/sleep 30 &\nprintf '%s\\n' \"$!\" > \"{}\"\nprintf 'sat\\n'\nwait\n",
            descendant_pid.display()
        ));
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024)
            .with_forced_process_group_kill_failure()
            .with_exact_test_timeout();
        let occurrence = TraceOccurrence::for_test("backend-test-kill-failure");

        let started = Instant::now();
        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(matches!(outcome, SolverProcessOutcome::Error(_)));

        if let Ok(descendant_pid) = fs::read_to_string(descendant_pid) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", descendant_pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        capture.commit(bundle, encoded_trace(occurrence, "error")).unwrap();
        let manifest = read_manifest(&directory, "backend-test-kill-failure");
        assert_eq!(manifest["termination"]["kind"], "timed_out");
        assert!(
            manifest["termination"]["error"]
                .as_str()
                .unwrap()
                .contains("injected fresh Z3 process-group termination failure")
        );
        assert_eq!(manifest["stdout"]["truncated"], false);
    }

    #[test]
    fn output_limit_is_bounded_and_not_classified_as_success() {
        let directory = TestDirectory::new("limit");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf '12345'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 4, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-limit");

        let CapturedFreshZ3Run { outcome, bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        assert!(matches!(outcome, SolverProcessOutcome::Error(_)));
        capture.commit(bundle, encoded_trace(occurrence, "error")).unwrap();

        assert_eq!(
            fs::read(directory.root.join("backend-test-limit/stdout.bin")).unwrap(),
            b"1234"
        );
        let manifest = read_manifest(&directory, "backend-test-limit");
        assert_eq!(manifest["termination"]["kind"], "output_limit");
        assert_eq!(manifest["stdout"]["bytes"], 4);
        assert_eq!(manifest["stdout"]["truncated"], true);
    }

    #[test]
    fn existing_occurrence_is_not_overwritten() {
        let directory = TestDirectory::new("collision");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-collision");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        let final_path = directory.root.join("backend-test-collision");
        fs::create_dir(&final_path).unwrap();
        fs::write(final_path.join("sentinel"), b"keep").unwrap();

        let error = capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert_eq!(fs::read(final_path.join("sentinel")).unwrap(), b"keep");
    }

    #[test]
    fn empty_existing_occurrence_is_not_overwritten() {
        let directory = TestDirectory::new("empty-collision");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-empty-collision");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        let final_path = directory.root.join("backend-test-empty-collision");
        fs::create_dir(&final_path).unwrap();

        let error = capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert_eq!(fs::read_dir(final_path).unwrap().count(), 0);
    }

    #[test]
    fn publication_claim_is_exclusive() {
        let directory = TestDirectory::new("claimed");
        let command = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-claimed");
        let CapturedFreshZ3Run { bundle, .. } =
            capture.run(&command, b"(check-sat)\n", 2, &occurrence).unwrap();
        let claim = directory.root.join(".backend-test-claimed.publish");
        fs::write(&claim, b"prior writer").unwrap();

        let error = capture.commit(bundle, encoded_trace(occurrence, "sat")).unwrap_err();
        assert!(error.to_string().contains("claim"));
        assert_eq!(fs::read(claim).unwrap(), b"prior writer");
        assert!(!directory.root.join("backend-test-claimed").exists());
    }

    #[test]
    fn missing_executable_fails_before_capture_without_partial_bundle() {
        let directory = TestDirectory::new("spawn-error");
        let missing_program = directory.parent.join("missing").join("z3");
        let command = SolverCommand::new(
            vec![missing_program.display().to_string(), "-in".to_string(), "-smt2".to_string()],
            true,
        )
        .unwrap();
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        capture.validate_command(std::slice::from_ref(&command), Some(2)).unwrap();
        let occurrence = TraceOccurrence::for_test("backend-test-spawn-error");

        let error = match capture.run(&command, b"(check-sat)\n", 2, &occurrence) {
            Ok(_) => panic!("missing solver executable was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("failed to resolve fresh Z3 executable"));
        assert!(!directory.root.join("backend-test-spawn-error").exists());
        assert!(!directory.root.join(".backend-test-spawn-error.tmp").exists());
    }

    #[test]
    fn executable_mutation_during_query_is_rejected() {
        let directory = TestDirectory::new("executable-mutation");
        let command = directory.fake_z3(
            "#!/bin/sh\ncat >/dev/null\nprintf '# changed\\n' >> \"$0\"\nprintf 'sat\\n'\n",
        );
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let occurrence = TraceOccurrence::for_test("backend-test-executable-mutation");

        let error = match capture.run(&command, b"(check-sat)\n", 2, &occurrence) {
            Ok(_) => panic!("mutated solver executable was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("changed during the solver invocation"));
        assert!(!directory.root.join("backend-test-executable-mutation").exists());
        assert!(!directory.root.join(".backend-test-executable-mutation.tmp").exists());
    }

    #[test]
    fn rejects_ambiguous_command_or_unbounded_timeout() {
        let directory = TestDirectory::new("validation");
        let capture = FreshZ3Capture::for_test(directory.root.clone(), 1024, 1024, 1024);
        let z3 = SolverCommand::new(
            vec!["z3".to_string(), "-in".to_string(), "-smt2".to_string()],
            true,
        )
        .unwrap();
        let custom = SolverCommand::new(vec!["custom".to_string()], false).unwrap();
        let explicit_z3 = directory.fake_z3("#!/bin/sh\ncat >/dev/null\nprintf 'sat\\n'\n");
        let explicit_z3 = SolverCommand::new(
            vec![explicit_z3.program().to_owned(), "-in".to_string(), "-smt2".to_string()],
            false,
        )
        .unwrap();

        assert!(capture.validate_command(std::slice::from_ref(&z3), None).is_err());
        assert!(capture.validate_command(std::slice::from_ref(&z3), Some(1)).is_err());
        assert!(capture.validate_command(&[z3.clone(), z3], Some(1)).is_err());
        assert!(capture.validate_command(&[custom], Some(1)).is_err());
        assert!(capture.validate_command(&[explicit_z3], Some(1)).is_ok());
    }
}
