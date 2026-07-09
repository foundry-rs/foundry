//! Run local Foundry scfuzzbench campaigns and collect deterministic artifacts.

use clap::{Parser, ValueEnum};
use eyre::{Context, Result};
use foundry_common::sh_println;
use once_cell::sync::Lazy;
use serde_json::json;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::time::Duration;
use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::Mutex,
};

const DEFAULT_SCFUZZBENCH_REPO: &str = "https://github.com/tempoxyz/scfuzzbench.git";
const DEFAULT_SCFUZZBENCH_REF: &str = "main";
const DEFAULT_FOUNDRY_REPO: &str = "https://github.com/foundry-rs/foundry.git";
const OUTPUT_MARKER: &str = ".foundry-scfuzzbench-output";
#[cfg(unix)]
const PROCESS_GROUP_GRACE: Duration = Duration::from_secs(2);

#[cfg(unix)]
type ProcessGroupId = libc::pid_t;
#[cfg(not(unix))]
type ProcessGroupId = u32;

static ACTIVE_PROCESS_GROUPS: Lazy<Mutex<HashSet<ProcessGroupId>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

const REQUIRED_DATA_ARTIFACTS: &[&str] = &[
    "REPORT.md",
    "events.csv",
    "summary.csv",
    "cumulative.csv",
    "throughput_samples.csv",
    "throughput_summary.csv",
    "progress_metrics_samples.csv",
    "progress_metrics_summary.csv",
    "showmap_campaign_manifest.json",
    "differential_coverage_relscores.csv",
    "differential_coverage_relcov.csv",
    "runner_resource_summary.csv",
    "runner_resource_timeseries.csv",
    "runner_resource_usage.md",
    "broken_invariants.csv",
    "broken_invariants.md",
];

/// Run a local Foundry scfuzzbench campaign and collect deterministic artifacts.
#[derive(Parser, Debug)]
#[clap(
    name = "foundry-scfuzzbench",
    about = "Run Foundry scfuzzbench campaigns and collect analysis artifacts"
)]
struct Cli {
    /// scfuzzbench repository to clone.
    #[clap(long, default_value = DEFAULT_SCFUZZBENCH_REPO)]
    scfuzzbench_repo: String,

    /// scfuzzbench branch, tag, or commit to pin.
    #[clap(long, default_value = DEFAULT_SCFUZZBENCH_REF)]
    scfuzzbench_ref: String,

    /// Target benchmark repository to run scfuzzbench against.
    #[clap(long)]
    target_repo: String,

    /// Target benchmark branch, tag, or commit to pin.
    #[clap(long)]
    target_ref: String,

    /// scfuzzbench benchmark type.
    #[clap(long, value_enum)]
    benchmark_type: BenchmarkType,

    /// Campaign timeout in seconds.
    #[clap(long)]
    timeout_seconds: u64,

    /// Number of Foundry worker threads.
    #[clap(long)]
    workers: Option<u64>,

    /// Deterministic output directory for work files and final artifacts.
    #[clap(long)]
    output_dir: PathBuf,

    /// Path to the forge binary to benchmark. Mutually exclusive with --foundry-ref.
    #[clap(long, conflicts_with = "foundry_ref")]
    foundry_bin: Option<PathBuf>,

    /// Foundry branch, tag, or commit to build and benchmark. Mutually exclusive with
    /// --foundry-bin.
    #[clap(long, conflicts_with = "foundry_bin")]
    foundry_ref: Option<String>,

    /// Foundry repository to clone when --foundry-ref is used.
    #[clap(long, default_value = DEFAULT_FOUNDRY_REPO)]
    foundry_repo: String,

    /// Extra arguments passed to scfuzzbench as --foundry-test-args.
    #[clap(long)]
    foundry_test_args: Option<String>,

    /// Target-repository-relative properties path passed as SCFUZZBENCH_PROPERTIES_PATH.
    /// Required for optimization mode.
    #[clap(long)]
    properties_path: Option<PathBuf>,

    /// Remove --output-dir before running if it already exists.
    #[clap(long)]
    force: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BenchmarkType {
    Property,
    Optimization,
}

impl BenchmarkType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Property => "property",
            Self::Optimization => "optimization",
        }
    }
}

struct Dirs {
    work: PathBuf,
    raw: PathBuf,
    data: PathBuf,
    images: PathBuf,
    artifacts: PathBuf,
    home: PathBuf,
    tools_bin: PathBuf,
    scfuzzbench: PathBuf,
    target_pin: PathBuf,
    scfuzz_root: PathBuf,
    scfuzz_work: PathBuf,
    scfuzz_logs: PathBuf,
    foundry_fuzz_corpus: PathBuf,
    foundry_invariant_corpus: PathBuf,
    unzipped: PathBuf,
    analysis_logs: PathBuf,
}

impl Dirs {
    fn new(output: PathBuf) -> Self {
        let work = output.join("work");
        Self {
            raw: output.join("raw"),
            data: output.join("data"),
            images: output.join("images"),
            artifacts: output.join("artifacts"),
            home: work.join("home"),
            tools_bin: work.join("bin"),
            scfuzzbench: work.join("scfuzzbench"),
            target_pin: work.join("target-pin"),
            scfuzz_root: work.join("scfuzz-root"),
            scfuzz_work: work.join("scfuzz-work"),
            scfuzz_logs: work.join("scfuzz-logs"),
            foundry_fuzz_corpus: work.join("foundry-fuzz-corpus"),
            foundry_invariant_corpus: work.join("foundry-invariant-corpus"),
            unzipped: work.join("unzipped"),
            analysis_logs: work.join("analysis-logs"),
            work,
        }
    }

    fn create(&self) -> Result<()> {
        for dir in [
            &self.work,
            &self.raw,
            &self.data,
            &self.images,
            &self.artifacts,
            &self.home,
            &self.tools_bin,
            &self.scfuzz_root,
            &self.scfuzz_work,
            &self.scfuzz_logs,
            &self.foundry_fuzz_corpus,
            &self.foundry_invariant_corpus,
            &self.unzipped,
            &self.analysis_logs,
        ] {
            fs::create_dir_all(dir)
                .wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        }
        Ok(())
    }
}

struct RunEnv {
    path: OsString,
    home: PathBuf,
}

impl RunEnv {
    fn apply(&self, command: &mut Command) {
        command.env("PATH", &self.path).env("HOME", &self.home);
    }
}

fn apply_foundry_corpus_env(command: &mut Command, dirs: &Dirs) {
    command
        .env("FOUNDRY_FUZZ_CORPUS_DIR", &dirs.foundry_fuzz_corpus)
        .env("FOUNDRY_INVARIANT_CORPUS_DIR", &dirs.foundry_invariant_corpus);
}

struct FoundrySelection {
    mode: &'static str,
    label: String,
    bin: PathBuf,
    repo: Option<String>,
    ref_name: Option<String>,
    commit: Option<String>,
    version_output: String,
    env: RunEnv,
}

struct RunMetadata<'a> {
    scfuzzbench_commit: &'a str,
    target_commit: &'a str,
    run_id: &'a str,
    campaign_exit_code: Option<i32>,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    ensure_supported_platform()?;
    install_termination_handler()?;
    let cli = Cli::parse();

    validate_options(&cli)?;
    preflight(&cli)?;
    prepare_output_dir(&cli.output_dir, cli.force)?;
    let dirs = Dirs::new(cli.output_dir.clone());
    dirs.create()?;
    install_date_shim(&dirs.tools_bin)?;
    install_timeout_shim(&dirs.tools_bin)?;
    install_sed_shim(&dirs.tools_bin)?;

    let _ = sh_println!("📦 Cloning scfuzzbench");
    let scfuzzbench_commit =
        clone_at(&cli.scfuzzbench_repo, &cli.scfuzzbench_ref, &dirs.scfuzzbench)
            .wrap_err("failed to clone scfuzzbench")?;

    let _ = sh_println!("📦 Resolving target repository pin");
    let target_commit = clone_at(&cli.target_repo, &cli.target_ref, &dirs.target_pin)
        .wrap_err("failed to clone target repository")?;

    let foundry = select_foundry(&cli, &dirs).wrap_err("failed to select Foundry binary")?;
    let _ = sh_println!("🔨 Foundry: {}", foundry.version_output.trim());

    let run_id = format!("foundry-scfuzzbench-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"));
    let campaign_status = run_campaign(&cli, &dirs, &foundry, &target_commit, &run_id)
        .wrap_err("failed to run scfuzzbench campaign")?;
    ensure_campaign_success(&campaign_status)?;

    validate_campaign_logs(&dirs)?;

    run_analysis(&cli, &dirs, &foundry, &run_id).wrap_err("failed to analyze campaign logs")?;
    validate_differential_coverage(&dirs)?;

    let run_metadata = RunMetadata {
        scfuzzbench_commit: &scfuzzbench_commit,
        target_commit: &target_commit,
        run_id: &run_id,
        campaign_exit_code: campaign_status.code(),
    };
    let mut missing = collect_artifacts(&dirs).wrap_err("failed to collect artifacts")?;
    let summary_path = write_llm_summary(&cli, &dirs, &foundry, &run_metadata, &missing)?;
    let manifest_path = write_manifest(&cli, &dirs, &foundry, &run_metadata)?;
    missing.retain(|path| path != "manifest.json" && path != "llm_summary.md");

    if !missing.is_empty() {
        eyre::bail!(
            "missing required scfuzzbench artifacts in {}: {}",
            dirs.artifacts.display(),
            missing.join(", ")
        );
    }

    let _ = sh_println!("✅ Artifacts written to {}", dirs.artifacts.display());
    let _ = sh_println!("   manifest: {}", manifest_path.display());
    let _ = sh_println!("   LLM summary: {}", summary_path.display());
    Ok(())
}

#[cfg(unix)]
const fn ensure_supported_platform() -> Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn ensure_supported_platform() -> Result<()> {
    eyre::bail!("foundry-scfuzzbench requires a Unix-like platform with bash process groups");
}

fn ensure_campaign_success(status: &ExitStatus) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    eyre::bail!(
        "scfuzzbench campaign failed ({status}); refusing to analyze incomplete campaign logs"
    );
}

fn validate_options(cli: &Cli) -> Result<()> {
    if matches!(cli.benchmark_type, BenchmarkType::Optimization) && cli.properties_path.is_none() {
        eyre::bail!("--properties-path is required for --benchmark-type optimization");
    }
    if let Some(properties_path) = &cli.properties_path {
        if properties_path.is_absolute() {
            eyre::bail!(
                "--properties-path must be relative to the target repository: {}",
                properties_path.display()
            );
        }
        if properties_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            eyre::bail!(
                "--properties-path must not escape the target repository: {}",
                properties_path.display()
            );
        }
    }
    Ok(())
}

fn preflight(cli: &Cli) -> Result<()> {
    for name in ["bash", "git", "make", "uv", "zip", "python3"] {
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {name} >/dev/null 2>&1"))
            .status()
            .wrap_err_with(|| format!("failed to check for {name}"))?;
        if !status.success() {
            eyre::bail!("required command `{name}` was not found in PATH");
        }
    }
    if cli.foundry_ref.is_some() && !command_exists("cargo")? {
        eyre::bail!("required command `cargo` was not found in PATH");
    }
    Ok(())
}

fn prepare_output_dir(output_dir: &Path, force: bool) -> Result<()> {
    if output_dir.exists() && fs::symlink_metadata(output_dir)?.file_type().is_symlink() {
        eyre::bail!("refusing to use symlink output directory {}", output_dir.display());
    }

    if output_dir.exists() {
        if !force && dir_has_entries(output_dir)? {
            eyre::bail!(
                "output directory {} already exists and is not empty; pass --force to remove it",
                output_dir.display()
            );
        }
        if force {
            if output_dir.parent().is_none() || output_dir == Path::new("/") {
                eyre::bail!("refusing to remove unsafe output directory {}", output_dir.display());
            }
            let marker = output_dir.join(OUTPUT_MARKER);
            if dir_has_entries(output_dir)? && !marker.exists() {
                eyre::bail!(
                    "refusing to remove {} because it is not marked as a foundry-scfuzzbench output directory",
                    output_dir.display()
                );
            }
            fs::remove_dir_all(output_dir)
                .wrap_err_with(|| format!("failed to remove {}", output_dir.display()))?;
        }
    }
    fs::create_dir_all(output_dir)
        .wrap_err_with(|| format!("failed to create {}", output_dir.display()))?;
    fs::write(output_dir.join(OUTPUT_MARKER), "foundry-scfuzzbench\n")?;
    Ok(())
}

fn command_exists(name: &str) -> Result<bool> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .status()
        .wrap_err_with(|| format!("failed to check for {name}"))?;
    Ok(status.success())
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .wrap_err_with(|| format!("failed to chmod {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn install_termination_handler() -> Result<()> {
    ctrlc::set_handler(|| {
        terminate_active_process_groups();
        std::process::exit(130);
    })
    .wrap_err("failed to install termination handler")
}

fn install_date_shim(tools_bin: &Path) -> Result<()> {
    let native_supports_iso_seconds = Command::new("date")
        .arg("-Is")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .wrap_err("failed to check native date -Is support")?
        .success();
    if native_supports_iso_seconds {
        return Ok(());
    }

    fs::create_dir_all(tools_bin)
        .wrap_err_with(|| format!("failed to create {}", tools_bin.display()))?;
    let shim = tools_bin.join("date");
    let content = r#"#!/usr/bin/env bash
if [[ "$#" -eq 1 && ( "$1" == "-Is" || "$1" == "-Iseconds" ) ]]; then
  exec python3 -c 'from datetime import datetime, timezone; print(datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds"))'
fi
exec /bin/date "$@"
"#;

    fs::write(&shim, content).wrap_err_with(|| format!("failed to write {}", shim.display()))?;
    make_executable(&shim)?;
    Ok(())
}

fn install_timeout_shim(tools_bin: &Path) -> Result<()> {
    if command_exists("timeout")? {
        return Ok(());
    }

    fs::create_dir_all(tools_bin)
        .wrap_err_with(|| format!("failed to create {}", tools_bin.display()))?;
    let shim = tools_bin.join("timeout");

    let content = if command_exists("gtimeout")? {
        r#"#!/usr/bin/env bash
exec gtimeout "$@"
"#
        .to_string()
    } else {
        r#"#!/usr/bin/env python3
import os
import signal
import subprocess
import sys
import time


def parse_seconds(value):
    if not value.endswith("s"):
        raise ValueError(f"unsupported duration {value!r}; expected seconds ending in 's'")
    return float(value[:-1])


def main(argv):
    if len(argv) < 5:
        print("timeout shim supports: timeout --signal=SIGINT --kill-after=<seconds>s <seconds>s <cmd...>", file=sys.stderr)
        return 125

    sigarg = argv[1]
    killarg = argv[2]
    duration_arg = argv[3]
    command = argv[4:]

    if sigarg != "--signal=SIGINT":
        print(f"unsupported timeout signal option: {sigarg}", file=sys.stderr)
        return 125
    if not killarg.startswith("--kill-after="):
        print(f"unsupported timeout kill-after option: {killarg}", file=sys.stderr)
        return 125

    try:
        duration = parse_seconds(duration_arg)
        grace = parse_seconds(killarg.split("=", 1)[1])
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 125

    proc = subprocess.Popen(command, start_new_session=True)
    try:
        return proc.wait(timeout=duration)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGINT)
        except ProcessLookupError:
            pass
        except PermissionError:
            proc.send_signal(signal.SIGINT)

        deadline = time.monotonic() + grace
        while time.monotonic() < deadline:
            code = proc.poll()
            if code is not None:
                return 124
            time.sleep(0.1)

        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            proc.kill()
        proc.wait()
        return 124


if __name__ == "__main__":
    sys.exit(main(sys.argv))
"#
        .to_string()
    };

    fs::write(&shim, content).wrap_err_with(|| format!("failed to write {}", shim.display()))?;
    make_executable(&shim)?;
    Ok(())
}

fn install_sed_shim(tools_bin: &Path) -> Result<()> {
    let native_supports_gnu_in_place = Command::new("sh")
        .arg("-c")
        .arg(
            r#"tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/foundry-scfuzzbench-sed.XXXXXX") || exit 1
trap 'rm -rf "$tmpdir"' EXIT
tmp="${tmpdir}/input"
printf 'foo\n' > "$tmp"
sed -i 's/foo/bar/' "$tmp" >/dev/null 2>&1 && grep -qx bar "$tmp"
"#,
        )
        .status()
        .wrap_err("failed to check native sed -i support")?
        .success();
    if native_supports_gnu_in_place {
        return Ok(());
    }

    fs::create_dir_all(tools_bin)
        .wrap_err_with(|| format!("failed to create {}", tools_bin.display()))?;
    let shim = tools_bin.join("sed");

    let content = if command_exists("gsed")? {
        r#"#!/usr/bin/env bash
exec gsed "$@"
"#
        .to_string()
    } else {
        r#"#!/usr/bin/env bash
native_sed=/usr/bin/sed
if [[ ! -x "${native_sed}" ]]; then
  native_sed=/bin/sed
fi

if "${native_sed}" --version >/dev/null 2>&1; then
  exec "${native_sed}" "$@"
fi

if [[ "${1:-}" == "-i" ]]; then
  shift
  exec "${native_sed}" -i '' "$@"
fi

exec "${native_sed}" "$@"
"#
        .to_string()
    };

    fs::write(&shim, content).wrap_err_with(|| format!("failed to write {}", shim.display()))?;
    make_executable(&shim)?;
    Ok(())
}

fn clone_at(repo: &str, git_ref: &str, dest: &Path) -> Result<String> {
    fs::create_dir_all(dest).wrap_err_with(|| format!("failed to create {}", dest.display()))?;

    let mut init = Command::new("git");
    init.arg("init").arg(dest);
    run_required(&mut init)?;

    let mut remote = Command::new("git");
    remote.current_dir(dest).args(["remote", "add", "origin", repo]);
    run_required(&mut remote)?;

    let mut fetch_command = Command::new("git");
    fetch_command.current_dir(dest).args(["fetch", "--depth", "1", "origin", git_ref]);
    let fetch = run_status(&mut fetch_command)
        .wrap_err_with(|| format!("failed to fetch {repo}@{git_ref}"))?;
    if !fetch.success() {
        let mut fetch_full = Command::new("git");
        fetch_full.current_dir(dest).args(["fetch", "origin", git_ref]);
        run_required(&mut fetch_full)?;
    }

    let mut checkout = Command::new("git");
    checkout.current_dir(dest).args(["checkout", "--detach", "FETCH_HEAD"]);
    run_required(&mut checkout)?;

    let mut rev_parse = Command::new("git");
    rev_parse.current_dir(dest).args(["rev-parse", "HEAD"]);
    output_text(&mut rev_parse).map(|s| s.trim().to_string())
}

fn select_foundry(cli: &Cli, dirs: &Dirs) -> Result<FoundrySelection> {
    if let Some(foundry_bin) = &cli.foundry_bin {
        let bin = foundry_bin
            .canonicalize()
            .wrap_err_with(|| format!("failed to canonicalize {}", foundry_bin.display()))?;
        if !bin.is_file() {
            eyre::bail!("--foundry-bin must point to a file: {}", bin.display());
        }
        if bin.file_name() != Some(OsStr::new("forge")) {
            eyre::bail!("--foundry-bin must point to a binary named `forge`: {}", bin.display());
        }
        let bin_dir = bin
            .parent()
            .ok_or_else(|| eyre::eyre!("{} has no parent directory", bin.display()))?
            .to_path_buf();
        let env = run_env(&dirs.tools_bin, Some(&bin_dir), &dirs.home)?;
        validate_selected_forge(&bin, &env)?;
        let version_output = forge_version(&env)?;
        return Ok(FoundrySelection {
            mode: "bin",
            label: "foundry-bin".to_string(),
            bin,
            repo: None,
            ref_name: None,
            commit: None,
            version_output,
            env,
        });
    }

    if let Some(foundry_ref) = &cli.foundry_ref {
        let foundry_checkout = dirs.work.join("foundry");
        let foundry_commit = clone_at(&cli.foundry_repo, foundry_ref, &foundry_checkout)?;

        let mut build = Command::new("cargo");
        build.current_dir(&foundry_checkout).args([
            "build",
            "--locked",
            "--profile",
            "dist",
            "--bin",
            "forge",
        ]);
        run_required(&mut build)?;

        let bin = foundry_checkout.join("target/dist/forge");
        let bin_dir = bin
            .parent()
            .ok_or_else(|| eyre::eyre!("{} has no parent directory", bin.display()))?
            .to_path_buf();
        let env = run_env(&dirs.tools_bin, Some(&bin_dir), &dirs.home)?;
        validate_selected_forge(&bin, &env)?;
        let version_output = forge_version(&env)?;
        let label = format!(
            "foundry-ref-{}-{}",
            sanitize_label(foundry_ref),
            foundry_commit.chars().take(12).collect::<String>()
        );
        return Ok(FoundrySelection {
            mode: "ref",
            label,
            bin: bin.canonicalize().unwrap_or(bin),
            repo: Some(cli.foundry_repo.clone()),
            ref_name: Some(foundry_ref.clone()),
            commit: Some(foundry_commit),
            version_output,
            env,
        });
    }

    let env = run_env(&dirs.tools_bin, None, &dirs.home)?;
    let mut which_forge = Command::new("sh");
    which_forge.arg("-c").arg("command -v forge");
    env.apply(&mut which_forge);
    let forge_path = output_text(&mut which_forge)?;
    let bin = PathBuf::from(forge_path.trim())
        .canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize {}", forge_path.trim()))?;
    validate_selected_forge(&bin, &env)?;
    let version_output = forge_version(&env)?;
    Ok(FoundrySelection {
        mode: "path",
        label: "foundry-path".to_string(),
        bin,
        repo: None,
        ref_name: None,
        commit: None,
        version_output,
        env,
    })
}

fn run_env(tools_bin: &Path, bin_dir: Option<&Path>, home: &Path) -> Result<RunEnv> {
    let mut paths = Vec::new();
    paths.push(tools_bin.to_path_buf());
    if let Some(bin_dir) = bin_dir {
        paths.push(bin_dir.to_path_buf());
    }
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    Ok(RunEnv { path: env::join_paths(paths)?, home: home.to_path_buf() })
}

fn validate_selected_forge(selected: &Path, env: &RunEnv) -> Result<()> {
    let selected = selected.canonicalize().wrap_err_with(|| {
        format!("failed to canonicalize selected forge {}", selected.display())
    })?;
    if !selected.is_file() {
        eyre::bail!("selected forge is not a file: {}", selected.display());
    }
    if selected.file_name() != Some(OsStr::new("forge")) {
        eyre::bail!("selected forge is not named `forge`: {}", selected.display());
    }

    let mut which_forge = Command::new("sh");
    which_forge.arg("-c").arg("command -v forge");
    env.apply(&mut which_forge);
    let resolved = output_text(&mut which_forge)?;
    let resolved = PathBuf::from(resolved.trim())
        .canonicalize()
        .wrap_err_with(|| format!("failed to canonicalize resolved forge {}", resolved.trim()))?;
    if resolved != selected {
        eyre::bail!(
            "selected forge {} does not match PATH-resolved forge {}",
            selected.display(),
            resolved.display()
        );
    }
    Ok(())
}

fn forge_version(env: &RunEnv) -> Result<String> {
    let mut command = Command::new("forge");
    env.apply(&mut command);
    command.arg("--version");
    output_text(&mut command)
}

fn run_campaign(
    cli: &Cli,
    dirs: &Dirs,
    foundry: &FoundrySelection,
    target_commit: &str,
    run_id: &str,
) -> Result<ExitStatus> {
    let _ = sh_println!("🚀 Running scfuzzbench campaign");
    let mut command = Command::new("bash");
    command
        .current_dir(&dirs.scfuzzbench)
        .arg("scripts/local-run.sh")
        .args(["-f", "foundry"])
        .args(["-r", &cli.target_repo])
        .args(["-b", target_commit])
        .args(["-t", &cli.timeout_seconds.to_string()])
        .args(["-T", cli.benchmark_type.as_str()]);

    if let Some(workers) = cli.workers {
        command.args(["-w", &workers.to_string()]);
        command.env("FOUNDRY_THREADS", workers.to_string());
    }
    if let Some(foundry_test_args) = cli.foundry_test_args.as_deref() {
        command.args(["--foundry-test-args", foundry_test_args]);
    }
    if let Some(properties_path) = &cli.properties_path {
        command.env("SCFUZZBENCH_PROPERTIES_PATH", properties_path);
    }

    foundry.env.apply(&mut command);
    apply_foundry_corpus_env(&mut command, dirs);
    command
        .env("SCFUZZBENCH_ROOT", &dirs.scfuzz_root)
        .env("SCFUZZBENCH_WORKDIR", &dirs.scfuzz_work)
        .env("SCFUZZBENCH_LOG_DIR", &dirs.scfuzz_logs)
        .env("SCFUZZBENCH_LOCAL_OUTPUT_DIR", &dirs.raw)
        .env("SCFUZZBENCH_RUN_ID", run_id)
        .env("SCFUZZBENCH_INSTANCE_ID", run_id)
        .env("SCFUZZBENCH_FUZZER_LABEL", &foundry.label)
        .env("FOUNDRY_LABEL", &foundry.label)
        .env("SCFUZZBENCH_FOUNDRY_SHOWMAP", "1")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    run_status(&mut command).wrap_err("failed to execute scripts/local-run.sh")
}

fn run_analysis(cli: &Cli, dirs: &Dirs, foundry: &FoundrySelection, run_id: &str) -> Result<()> {
    let _ = sh_println!("📊 Running scfuzzbench analysis");
    let prepared_logs = dirs.unzipped.join(&foundry.label).join("logs");
    fs::create_dir_all(&prepared_logs)
        .wrap_err_with(|| format!("failed to create {}", prepared_logs.display()))?;
    copy_analysis_logs(&dirs.scfuzz_logs, &prepared_logs)?;

    make(
        dirs,
        &[
            OsString::from("results-prepare"),
            make_var("UNZIPPED_DIR", &dirs.unzipped),
            make_var("ANALYSIS_LOGS_DIR", &dirs.analysis_logs),
        ],
    )?;
    make(
        dirs,
        &[
            OsString::from("results-analyze-filtered"),
            make_var("ANALYSIS_LOGS_DIR", &dirs.analysis_logs),
            make_var("ANALYSIS_OUT_DIR", &dirs.data),
            make_str_var("RUN_ID", run_id),
        ],
    )?;
    make(
        dirs,
        &[
            OsString::from("report-events-to-cumulative"),
            make_var("ANALYSIS_LOGS_DIR", &dirs.analysis_logs),
            make_var("ANALYSIS_OUT_DIR", &dirs.data),
            make_var("EVENTS_CSV", &dirs.data.join("events.csv")),
            make_var("CUMULATIVE_CSV", &dirs.data.join("cumulative.csv")),
            make_str_var("RUN_ID", run_id),
        ],
    )?;

    let report_budget = format!("{:.3}", cli.timeout_seconds as f64 / 3600.0);
    make(
        dirs,
        &[
            OsString::from("report-benchmark"),
            make_var("ANALYSIS_LOGS_DIR", &dirs.analysis_logs),
            make_var("ANALYSIS_OUT_DIR", &dirs.data),
            make_var("REPORT_CSV", &dirs.data.join("cumulative.csv")),
            make_var("REPORT_OUT_DIR", &dirs.data),
            make_var("IMAGES_OUT_DIR", &dirs.images),
            make_str_var("REPORT_BUDGET", &report_budget),
        ],
    )?;
    make(
        dirs,
        &[
            OsString::from("report-invariant-overlap"),
            make_var("ANALYSIS_LOGS_DIR", &dirs.analysis_logs),
            make_var("ANALYSIS_OUT_DIR", &dirs.data),
            make_var("EVENTS_CSV", &dirs.data.join("events.csv")),
            make_var("IMAGES_OUT_DIR", &dirs.images),
            make_str_var("REPORT_BUDGET", &report_budget),
        ],
    )?;
    make(
        dirs,
        &[
            OsString::from("report-runner-metrics"),
            make_var("ANALYSIS_LOGS_DIR", &dirs.analysis_logs),
            make_var("ANALYSIS_OUT_DIR", &dirs.data),
            make_var("IMAGES_OUT_DIR", &dirs.images),
            make_str_var("RUN_ID", run_id),
            make_str_var("REPORT_BUDGET", &report_budget),
        ],
    )?;
    Ok(())
}

fn validate_campaign_logs(dirs: &Dirs) -> Result<()> {
    let foundry_log = dirs.scfuzz_logs.join("foundry.log");
    ensure_non_empty_file(&foundry_log, "campaign foundry log")?;

    let commands_log = dirs.scfuzz_logs.join("runner_commands.log");
    ensure_non_empty_file(&commands_log, "campaign runner commands log")?;
    let commands = fs::read_to_string(&commands_log)
        .wrap_err_with(|| format!("failed to read {}", commands_log.display()))?;
    if !commands.contains("forge test --mc CryticToFoundry") {
        eyre::bail!(
            "{} did not contain expected Foundry campaign command `forge test --mc CryticToFoundry`",
            commands_log.display()
        );
    }
    Ok(())
}

fn validate_differential_coverage(dirs: &Dirs) -> Result<()> {
    let manifest_path = dirs.data.join("showmap_campaign_manifest.json");
    ensure_non_empty_file(&manifest_path, "showmap campaign manifest")?;
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path)
            .wrap_err_with(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .wrap_err_with(|| format!("failed to parse {}", manifest_path.display()))?;

    let raw_trials = manifest.get("raw_trials").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if raw_trials == 0 {
        eyre::bail!("{} has raw_trials=0", manifest_path.display());
    }

    let approaches = combined_approaches(&manifest_path, &manifest)?;
    let has_covered_trial = approaches.iter().any(|entry| {
        let trials = entry.get("trials").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let covered_edges =
            entry.get("covered_edges").and_then(serde_json::Value::as_u64).unwrap_or(0);
        trials > 0 && covered_edges > 0
    });
    if !has_covered_trial {
        eyre::bail!(
            "{} has no campaigns.combined approach with trials > 0 and covered_edges > 0",
            manifest_path.display()
        );
    }

    ensure_csv_has_data_row(&dirs.data.join("differential_coverage_relscores.csv"))?;

    let relcov = dirs.data.join("differential_coverage_relcov.csv");
    if approaches.len() > 1 {
        ensure_csv_has_data_row(&relcov)?;
    } else {
        ensure_non_empty_file(&relcov, "differential coverage CSV")?;
    }
    Ok(())
}

fn combined_approaches<'a>(
    manifest_path: &Path,
    manifest: &'a serde_json::Value,
) -> Result<Vec<&'a serde_json::Value>> {
    let combined = manifest
        .get("campaigns")
        .and_then(|campaigns| campaigns.get("combined"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            eyre::eyre!("{} does not contain campaigns.combined", manifest_path.display())
        })?;

    let approaches = match combined.get("approaches") {
        Some(approaches) => approaches.as_object().ok_or_else(|| {
            eyre::eyre!(
                "{} campaigns.combined.approaches is not an object",
                manifest_path.display()
            )
        })?,
        None => combined,
    };

    if approaches.is_empty() {
        eyre::bail!("{} has empty campaigns.combined approaches", manifest_path.display());
    }

    Ok(approaches.values().collect())
}

fn ensure_non_empty_file(path: &Path, label: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).wrap_err_with(|| format!("missing {label}: {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        eyre::bail!("{label} is empty or not a file: {}", path.display());
    }
    Ok(())
}

fn ensure_csv_has_data_row(path: &Path) -> Result<()> {
    ensure_non_empty_file(path, "differential coverage CSV")?;
    let contents =
        fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    let non_empty_lines = contents.lines().filter(|line| !line.trim().is_empty()).count();
    if non_empty_lines < 2 {
        eyre::bail!("{} has no data rows", path.display());
    }
    Ok(())
}

fn make(dirs: &Dirs, args: &[OsString]) -> Result<()> {
    let mut command = Command::new("make");
    command
        .current_dir(&dirs.scfuzzbench)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    run_required(&mut command)
}

fn make_var(name: &str, path: &Path) -> OsString {
    let mut value = OsString::from(name);
    value.push("=");
    value.push(path.as_os_str());
    value
}

fn make_str_var(name: &str, value: &str) -> OsString {
    OsString::from(format!("{name}={value}"))
}

fn collect_artifacts(dirs: &Dirs) -> Result<Vec<String>> {
    let _ = sh_println!("📁 Collecting deterministic artifact bundle");
    fs::create_dir_all(&dirs.artifacts)?;

    let mut missing = Vec::new();
    for artifact in REQUIRED_DATA_ARTIFACTS {
        let src = dirs.data.join(artifact);
        let dest = dirs.artifacts.join(artifact);
        if src.exists() {
            copy_path(&src, &dest)?;
        } else {
            missing.push((*artifact).to_string());
        }
    }

    copy_if_exists(
        &dirs.data.join("showmap_campaigns"),
        &dirs.artifacts.join("showmap_campaigns"),
    )?;
    copy_if_exists(&dirs.images, &dirs.artifacts.join("images"))?;
    collect_raw_archives(&dirs.raw, &dirs.artifacts.join("raw"))?;
    collect_lcov_outputs(dirs, &dirs.artifacts.join("lcov-diff"))?;

    Ok(missing)
}

fn collect_raw_archives(raw: &Path, dest: &Path) -> Result<()> {
    let logs = find_named(raw, "logs.zip")?;
    let corpus = find_named(raw, "corpus.zip")?;
    if logs.is_empty() && corpus.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    if let Some(path) = logs.first() {
        fs::copy(path, dest.join("logs.zip"))?;
    }
    if let Some(path) = corpus.first() {
        fs::copy(path, dest.join("corpus.zip"))?;
    }
    Ok(())
}

fn collect_lcov_outputs(dirs: &Dirs, dest: &Path) -> Result<()> {
    let mut matches = Vec::new();
    for root in [&dirs.raw, &dirs.data, &dirs.work] {
        find_lcov_like(root, &mut matches)?;
    }
    matches.sort();
    if matches.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for path in matches {
        if let Some(name) = path.file_name() {
            copy_path(&path, &dest.join(name))?;
        }
    }
    Ok(())
}

fn write_manifest(
    cli: &Cli,
    dirs: &Dirs,
    foundry: &FoundrySelection,
    metadata: &RunMetadata<'_>,
) -> Result<PathBuf> {
    let artifacts = list_relative_files(&dirs.artifacts)?;
    let manifest = json!({
        "scfuzzbench": {
            "repo": &cli.scfuzzbench_repo,
            "ref": &cli.scfuzzbench_ref,
            "commit": metadata.scfuzzbench_commit,
        },
        "target": {
            "repo": &cli.target_repo,
            "ref": &cli.target_ref,
            "commit": metadata.target_commit,
        },
        "foundry": {
            "mode": foundry.mode,
            "label": &foundry.label,
            "bin": foundry.bin.display().to_string(),
            "repo": foundry.repo.as_deref(),
            "ref": foundry.ref_name.as_deref(),
            "commit": foundry.commit.as_deref(),
            "version_output": foundry.version_output.trim(),
        },
        "campaign": {
            "benchmark_type": cli.benchmark_type.as_str(),
            "timeout_seconds": cli.timeout_seconds,
            "workers": cli.workers,
            "run_id": metadata.run_id,
            "exit_code": metadata.campaign_exit_code,
            "foundry_test_args": cli.foundry_test_args.as_deref(),
            "foundry_corpus": {
                "fuzz": dirs.foundry_fuzz_corpus.display().to_string(),
                "invariant": dirs.foundry_invariant_corpus.display().to_string(),
            },
            "properties_path": cli.properties_path.as_ref().map(|path| path.display().to_string()),
        },
        "artifacts": artifacts,
    });
    let path = dirs.artifacts.join("manifest.json");
    fs::write(&path, serde_json::to_string_pretty(&manifest)? + "\n")?;
    Ok(path)
}

fn write_llm_summary(
    cli: &Cli,
    dirs: &Dirs,
    foundry: &FoundrySelection,
    metadata: &RunMetadata<'_>,
    missing: &[String],
) -> Result<PathBuf> {
    let mut lines = vec![
        "# Foundry scfuzzbench summary".to_string(),
        String::new(),
        format!(
            "- scfuzzbench: `{}` @ `{}` (`{}`)",
            cli.scfuzzbench_repo, cli.scfuzzbench_ref, metadata.scfuzzbench_commit
        ),
        format!(
            "- target: `{}` @ `{}` (`{}`)",
            cli.target_repo, cli.target_ref, metadata.target_commit
        ),
        format!("- foundry: `{}` ({})", foundry.version_output.trim(), foundry.mode),
        format!("- benchmark type: `{}`", cli.benchmark_type.as_str()),
        format!("- timeout seconds: `{}`", cli.timeout_seconds),
        format!(
            "- workers: `{}`",
            cli.workers.map(|w| w.to_string()).unwrap_or_else(|| "default".to_string())
        ),
        format!("- fuzz corpus: `{}`", dirs.foundry_fuzz_corpus.display()),
        format!("- invariant corpus: `{}`", dirs.foundry_invariant_corpus.display()),
        format!("- run id: `{}`", metadata.run_id),
        format!(
            "- campaign exit code: `{}`",
            metadata
                .campaign_exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal/unknown".to_string())
        ),
        format!(
            "- required artifacts missing: `{}`",
            if missing.is_empty() { "none".to_string() } else { missing.join(", ") }
        ),
        String::new(),
        "## Primary artifacts".to_string(),
        String::new(),
        "- `REPORT.md`".to_string(),
        "- `events.csv`, `summary.csv`, `cumulative.csv`".to_string(),
        "- `showmap_campaign_manifest.json` and `showmap_campaigns/`".to_string(),
        "- `differential_coverage_relscores.csv` and `differential_coverage_relcov.csv`"
            .to_string(),
    ];

    let report = dirs.artifacts.join("REPORT.md");
    if report.exists() {
        let preview = fs::read_to_string(&report)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(12)
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !preview.is_empty() {
            lines.extend([String::new(), "## Report preview".to_string(), String::new()]);
            lines.extend(preview);
        }
    }

    let path = dirs.artifacts.join("llm_summary.md");
    fs::write(&path, lines.join("\n") + "\n")?;
    Ok(path)
}

fn run_required(command: &mut Command) -> Result<()> {
    let display = command_display(command);
    let status = run_status(command)?;
    if !status.success() {
        eyre::bail!("command failed ({status}): {display}");
    }
    Ok(())
}

fn run_status(command: &mut Command) -> Result<ExitStatus> {
    let display = command_display(command);
    let (mut child, mut guard) = spawn_guarded(command, &display)?;
    let status = child.wait().wrap_err_with(|| format!("failed to wait for {display}"))?;
    guard.finish();
    Ok(status)
}

fn output_text(command: &mut Command) -> Result<String> {
    let display = command_display(command);
    let output = guarded_output(command, &display)?;
    if !output.status.success() {
        eyre::bail!(
            "command failed ({}): {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            display,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn guarded_output(command: &mut Command, display: &str) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (child, mut guard) = spawn_guarded(command, display)?;
    let output =
        child.wait_with_output().wrap_err_with(|| format!("failed to wait for {display}"))?;
    guard.finish();
    Ok(output)
}

fn spawn_guarded(command: &mut Command, display: &str) -> Result<(Child, ActiveProcessGroup)> {
    configure_process_group(command);
    let child = command.spawn().wrap_err_with(|| format!("failed to execute {display}"))?;
    let guard = ActiveProcessGroup::new(child.id() as ProcessGroupId);
    Ok((child, guard))
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

struct ActiveProcessGroup {
    pgid: ProcessGroupId,
    finished: bool,
}

impl ActiveProcessGroup {
    fn new(pgid: ProcessGroupId) -> Self {
        ACTIVE_PROCESS_GROUPS.lock().expect("active process group lock poisoned").insert(pgid);
        Self { pgid, finished: false }
    }

    fn finish(&mut self) {
        terminate_process_group(self.pgid);
        self.finished = true;
        unregister_active_process_group(self.pgid);
    }
}

impl Drop for ActiveProcessGroup {
    fn drop(&mut self) {
        unregister_active_process_group(self.pgid);
        if !self.finished {
            terminate_process_group(self.pgid);
        }
    }
}

fn unregister_active_process_group(pgid: ProcessGroupId) {
    ACTIVE_PROCESS_GROUPS.lock().expect("active process group lock poisoned").remove(&pgid);
}

fn terminate_active_process_groups() {
    let pgids = ACTIVE_PROCESS_GROUPS
        .lock()
        .expect("active process group lock poisoned")
        .iter()
        .copied()
        .collect::<Vec<_>>();
    for pgid in pgids {
        terminate_process_group(pgid);
    }
}

#[cfg(unix)]
fn terminate_process_group(pgid: ProcessGroupId) {
    if matches!(signal_process_group(pgid, libc::SIGINT), Ok(true)) {
        std::thread::sleep(PROCESS_GROUP_GRACE);
        let _ = signal_process_group(pgid, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_pgid: ProcessGroupId) {}

#[cfg(unix)]
fn signal_process_group(pgid: ProcessGroupId, signal: libc::c_int) -> std::io::Result<bool> {
    // SAFETY: negative pid targets the process group created for the child process.
    let rc = unsafe { libc::kill(-pgid, signal) };
    if rc == 0 {
        Ok(true)
    } else {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) { Ok(false) } else { Err(err) }
    }
}

fn command_display(command: &Command) -> String {
    let mut parts = vec![command.get_program().to_string_lossy().to_string()];
    parts.extend(command.get_args().map(|arg| arg.to_string_lossy().to_string()));
    parts.join(" ")
}

fn dir_has_entries(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    Ok(fs::read_dir(path)?.next().is_some())
}

fn copy_if_exists(src: &Path, dest: &Path) -> Result<()> {
    if src.exists() {
        copy_path(src, dest)?;
    }
    Ok(())
}

fn copy_path(src: &Path, dest: &Path) -> Result<()> {
    if src.is_dir() {
        copy_dir(src, dest)
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)
            .wrap_err_with(|| format!("failed to copy {} to {}", src.display(), dest.display()))?;
        Ok(())
    }
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    copy_dir_contents(src, dest)
}

fn copy_dir_contents(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let mut entries = fs::read_dir(src)
        .wrap_err_with(|| format!("failed to read {}", src.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        copy_path(&src_path, &dest_path)?;
    }
    Ok(())
}

fn copy_analysis_logs(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let mut entries = fs::read_dir(src)
        .wrap_err_with(|| format!("failed to read {}", src.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dest.join(&file_name);
        if src_path.is_dir() {
            copy_analysis_logs(&src_path, &dest_path)?;
            continue;
        }
        if is_showmap_log(&file_name) {
            continue;
        }
        copy_path(&src_path, &dest_path)?;
    }
    Ok(())
}

fn is_showmap_log(file_name: &OsStr) -> bool {
    let name = file_name.to_string_lossy();
    name == "foundry_showmap.log" || name.ends_with("_showmap.log")
}

fn find_named(root: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    find_named_inner(root, OsStr::new(name), &mut out)?;
    out.sort();
    Ok(out)
}

fn find_named_inner(root: &Path, name: &OsStr, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name() == Some(name) {
            out.push(path.clone());
        }
        if path.is_dir() {
            find_named_inner(&path, name, out)?;
        }
    }
    Ok(())
}

fn find_lcov_like(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if is_lcov_like_name(&name) {
            out.push(path.clone());
        } else if path.is_dir() {
            find_lcov_like(&path, out)?;
        }
    }
    Ok(())
}

fn is_lcov_like_name(name: &str) -> bool {
    name.contains("coverage-diff")
        || name.contains("coverage_diff")
        || name.split(|ch: char| !ch.is_ascii_alphanumeric()).any(|part| part == "lcov")
}

fn list_relative_files(root: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    list_relative_files_inner(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn list_relative_files_inner(root: &Path, current: &Path, files: &mut Vec<String>) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            list_relative_files_inner(root, &path, files)?;
        } else {
            files.push(path.strip_prefix(root)?.display().to_string());
        }
    }
    Ok(())
}

fn sanitize_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_cli() -> Cli {
        Cli {
            scfuzzbench_repo: DEFAULT_SCFUZZBENCH_REPO.to_string(),
            scfuzzbench_ref: DEFAULT_SCFUZZBENCH_REF.to_string(),
            target_repo: "https://github.com/example/target.git".to_string(),
            target_ref: "main".to_string(),
            benchmark_type: BenchmarkType::Property,
            timeout_seconds: 60,
            workers: None,
            output_dir: PathBuf::from("out"),
            foundry_bin: None,
            foundry_ref: None,
            foundry_repo: DEFAULT_FOUNDRY_REPO.to_string(),
            foundry_test_args: None,
            properties_path: None,
            force: false,
        }
    }

    fn temp_dirs() -> (tempfile::TempDir, Dirs) {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let dirs = Dirs::new(temp.path().join("run"));
        dirs.create().expect("failed to create scfuzzbench dirs");
        (temp, dirs)
    }

    fn write_differential_coverage_inputs(dirs: &Dirs, manifest: serde_json::Value, relcov: &str) {
        fs::write(
            dirs.data.join("showmap_campaign_manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("failed to serialize manifest"),
        )
        .expect("failed to write manifest");
        fs::write(
            dirs.data.join("differential_coverage_relscores.csv"),
            "approach,score\nfoundry,1\n",
        )
        .expect("failed to write relscores");
        fs::write(dirs.data.join("differential_coverage_relcov.csv"), relcov)
            .expect("failed to write relcov");
    }

    #[test]
    fn applies_work_local_foundry_corpus_env() {
        let (_temp, dirs) = temp_dirs();
        let mut command = Command::new("true");

        apply_foundry_corpus_env(&mut command, &dirs);

        let envs = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(|value| value.to_owned())))
            .collect::<Vec<_>>();
        assert!(envs.iter().any(|(key, value)| {
            key == "FOUNDRY_FUZZ_CORPUS_DIR"
                && value.as_deref() == Some(dirs.foundry_fuzz_corpus.as_os_str())
        }));
        assert!(envs.iter().any(|(key, value)| {
            key == "FOUNDRY_INVARIANT_CORPUS_DIR"
                && value.as_deref() == Some(dirs.foundry_invariant_corpus.as_os_str())
        }));
    }

    #[test]
    fn validates_repo_relative_properties_path() {
        let mut cli = base_cli();
        cli.properties_path = Some(PathBuf::from("test/recon/Properties.sol"));
        validate_options(&cli).expect("repo-relative properties path should be valid");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_absolute_properties_path() {
        let mut cli = base_cli();
        cli.properties_path = Some(PathBuf::from("/tmp/Properties.sol"));
        let err = validate_options(&cli).expect_err("absolute properties path should fail");
        assert!(err.to_string().contains("must be relative"));
    }

    #[test]
    fn rejects_parent_dir_properties_path() {
        let mut cli = base_cli();
        cli.properties_path = Some(PathBuf::from("../Properties.sol"));
        let err = validate_options(&cli).expect_err("escaping properties path should fail");
        assert!(err.to_string().contains("must not escape"));
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_campaign_status_is_an_error() {
        let status =
            Command::new("sh").arg("-c").arg("exit 7").status().expect("failed to execute shell");
        let err = ensure_campaign_success(&status).expect_err("campaign failure should fail");
        assert!(err.to_string().contains("campaign failed"));
    }

    #[test]
    fn installed_sed_shim_accepts_gnu_no_backup_in_place_form() {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        install_sed_shim(temp.path()).expect("failed to install sed shim");
        let shim = temp.path().join("sed");
        if !shim.exists() {
            return;
        }

        let input = temp.path().join("input");
        fs::write(&input, "foo\n").expect("failed to write sed input");
        let status = Command::new(&shim)
            .args(["-i", "s/foo/bar/"])
            .arg(&input)
            .status()
            .expect("failed to run sed shim");
        assert!(status.success());
        assert_eq!(fs::read_to_string(&input).expect("failed to read sed output"), "bar\n");
    }

    #[test]
    fn validates_nested_showmap_approaches_with_header_only_single_relcov() {
        let (_temp, dirs) = temp_dirs();
        write_differential_coverage_inputs(
            &dirs,
            json!({
                "raw_trials": 1,
                "campaigns": {
                    "combined": {
                        "approaches": {
                            "foundry": {
                                "trials": 1,
                                "covered_edges": 12
                            }
                        }
                    }
                }
            }),
            "approach,relative_coverage\n",
        );

        validate_differential_coverage(&dirs)
            .expect("single-approach header-only relcov should be accepted");
    }

    #[test]
    fn validates_multi_approach_relcov_rows() {
        let (_temp, dirs) = temp_dirs();
        let manifest = json!({
            "raw_trials": 2,
            "campaigns": {
                "combined": {
                    "approaches": {
                        "foundry": {
                            "trials": 1,
                            "covered_edges": 12
                        },
                        "echidna": {
                            "trials": 1,
                            "covered_edges": 10
                        }
                    }
                }
            }
        });
        write_differential_coverage_inputs(&dirs, manifest.clone(), "approach,relative_coverage\n");

        let err = validate_differential_coverage(&dirs)
            .expect_err("multi-approach relcov should require data rows");
        assert!(err.to_string().contains("has no data rows"));

        write_differential_coverage_inputs(
            &dirs,
            manifest,
            "approach,relative_coverage\nfoundry,1.0\n",
        );
        validate_differential_coverage(&dirs)
            .expect("multi-approach relcov with data rows should be accepted");
    }

    #[test]
    fn does_not_collect_relcov_as_lcov_output() {
        let (_temp, dirs) = temp_dirs();
        fs::write(dirs.data.join("differential_coverage_relcov.csv"), "header\n")
            .expect("failed to write relcov");
        fs::write(dirs.data.join("coverage_diff.csv"), "diff\n")
            .expect("failed to write coverage diff");
        fs::create_dir_all(dirs.raw.join("nested")).expect("failed to create raw nested dir");
        fs::write(dirs.raw.join("nested/run-lcov.info"), "lcov\n")
            .expect("failed to write lcov file");

        let dest = dirs.artifacts.join("lcov-diff");
        collect_lcov_outputs(&dirs, &dest).expect("failed to collect lcov outputs");

        assert!(dest.join("coverage_diff.csv").exists());
        assert!(dest.join("run-lcov.info").exists());
        assert!(!dest.join("differential_coverage_relcov.csv").exists());
    }
}
