//! Seeded fuzzing evaluation over a fixed benchmark with a train/test split.
//!
//! Runs every manifest target once per seed, records which planted bugs each run found and
//! (optionally) the source coverage it reached, and summarizes the results with confidence
//! intervals so fuzzing changes can be accepted or rejected on the held-out test split.

use clap::Parser;
use eyre::{Result, WrapErr, ensure};
use foundry_common::{sh_eprintln, sh_println};
use manifest::{Budget, Manifest, Split};
use report::{Verdict, render_comparison, render_summary};
use results::{EvalResults, SCHEMA_VERSION};
use runner::{Runner, RunnerConfig, forge_version};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

mod forge_json;
mod lcov;
mod manifest;
mod report;
mod results;
mod runner;
mod stats;

/// Run seeded fuzzing evaluations and compare them against a baseline.
#[derive(Parser, Debug)]
#[clap(name = "foundry-fuzz-eval", about = "Seeded fuzzing evaluation with a train/test split")]
struct Cli {
    /// Benchmark manifest (TOML).
    #[clap(long, required_unless_present = "current")]
    manifest: Option<PathBuf>,

    /// Directory for `results.json`, `SUMMARY.md`, logs, and scratch state.
    #[clap(long, required_unless_present = "current")]
    output_dir: Option<PathBuf>,

    /// forge binary to evaluate.
    #[clap(long, default_value = "forge")]
    forge_bin: PathBuf,

    /// Run seeds 1..=N.
    #[clap(long, default_value_t = 5, conflicts_with = "seed_list")]
    seeds: u64,

    /// Explicit comma-separated seeds.
    #[clap(long, value_delimiter = ',')]
    seed_list: Vec<u64>,

    /// Also run `forge coverage --report lcov` once per seed.
    #[clap(long)]
    coverage: bool,

    /// Arguments appended to every forge test and coverage invocation, split on whitespace.
    /// May be repeated. Use this to evaluate a candidate flag.
    #[clap(long, allow_hyphen_values = true)]
    extra_forge_args: Vec<String>,

    /// Override the stateless fuzz runs of every target.
    #[clap(long)]
    fuzz_runs: Option<u64>,

    /// Override the stateless fuzz timeout (seconds) of every target.
    #[clap(long)]
    fuzz_timeout: Option<u64>,

    /// Override the invariant runs of every target.
    #[clap(long)]
    invariant_runs: Option<u64>,

    /// Override the invariant depth of every target.
    #[clap(long)]
    invariant_depth: Option<u64>,

    /// Override the invariant timeout (seconds) of every target.
    #[clap(long)]
    invariant_timeout: Option<u64>,

    /// Override the per-invocation timeout (seconds) of every target.
    #[clap(long)]
    timeout_secs: Option<u64>,

    /// Only run targets in this split.
    #[clap(long, value_parser = parse_split)]
    split: Option<Split>,

    /// Only run these comma-separated targets.
    #[clap(long, value_delimiter = ',')]
    targets: Vec<String>,

    /// Directory for git checkouts of `repo` targets. Defaults to `<output-dir>/cache`.
    #[clap(long)]
    cache_dir: Option<PathBuf>,

    /// Label recorded in the results, such as `baseline` or a treatment name.
    #[clap(long)]
    label: Option<String>,

    /// Baseline `results.json` to compare against. Writes `COMPARE.md` and prints the verdict.
    #[clap(long)]
    compare: Option<PathBuf>,

    /// Compare an existing `results.json` against `--compare` instead of running.
    #[clap(long, requires = "compare")]
    current: Option<PathBuf>,

    /// Exit with status 2 unless the comparison verdict is KEEP.
    #[clap(long, requires = "compare")]
    fail_unless_keep: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let current = match &cli.current {
        Some(path) => load_results(path)?,
        None => run(&cli)?,
    };

    if let Some(baseline_path) = &cli.compare {
        let baseline = load_results(baseline_path)?;
        let (markdown, verdict) = render_comparison(&baseline, &current);
        if let Some(output_dir) = &cli.output_dir {
            fs::create_dir_all(output_dir)?;
            fs::write(output_dir.join("COMPARE.md"), &markdown)?;
        }
        sh_println!("{markdown}")?;
        if cli.fail_unless_keep && verdict != Verdict::Keep {
            std::process::exit(2);
        }
    }
    Ok(())
}

fn run(cli: &Cli) -> Result<EvalResults> {
    let (Some(manifest_path), Some(output_dir)) = (&cli.manifest, &cli.output_dir) else {
        eyre::bail!("--manifest and --output-dir are required unless --current is set");
    };
    let manifest_path = fs::canonicalize(manifest_path)
        .wrap_err_with(|| format!("failed to resolve {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or(Path::new("."));
    let mut targets = Manifest::load(&manifest_path)?.resolve(base_dir)?;
    for name in &cli.targets {
        ensure!(targets.iter().any(|target| &target.name == name), "unknown target `{name}`");
    }
    targets.retain(|target| {
        cli.split.is_none_or(|split| target.split == split)
            && (cli.targets.is_empty() || cli.targets.contains(&target.name))
    });
    ensure!(!targets.is_empty(), "no targets selected");

    let seeds =
        if cli.seed_list.is_empty() { (1..=cli.seeds).collect() } else { cli.seed_list.clone() };
    ensure!(!seeds.is_empty(), "at least one seed is required");

    fs::create_dir_all(output_dir)?;
    let output_dir = fs::canonicalize(output_dir)?;
    let forge_bin = resolve_forge(&cli.forge_bin)?;
    let forge_version = forge_version(&forge_bin)?;
    let extra_forge_args = cli
        .extra_forge_args
        .iter()
        .flat_map(|args| args.split_whitespace().map(str::to_string))
        .collect::<Vec<_>>();

    let mut runner = Runner::new(RunnerConfig {
        forge_bin: forge_bin.clone(),
        extra_args: extra_forge_args.clone(),
        budget_override: Budget {
            fuzz_runs: cli.fuzz_runs,
            fuzz_timeout: cli.fuzz_timeout,
            invariant_runs: cli.invariant_runs,
            invariant_depth: cli.invariant_depth,
            invariant_timeout: cli.invariant_timeout,
        },
        timeout_override: cli.timeout_secs,
        coverage: cli.coverage,
        work_dir: output_dir.join("work"),
        cache_dir: cli.cache_dir.clone().unwrap_or_else(|| output_dir.join("cache")),
        logs_dir: output_dir.join("logs"),
    });

    sh_eprintln!(
        "evaluating {} targets x {} seeds with {forge_version}",
        targets.len(),
        seeds.len()
    )?;
    let mut results = EvalResults {
        schema_version: SCHEMA_VERSION,
        label: cli.label.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        manifest: manifest_path.display().to_string(),
        forge_bin: forge_bin.display().to_string(),
        forge_version,
        extra_forge_args,
        seeds: seeds.clone(),
        coverage: cli.coverage,
        targets: targets.iter().map(|target| runner.run_target(target, &seeds)).collect(),
        splits: BTreeMap::new(),
    };
    results.summarize();

    let results_path = output_dir.join("results.json");
    fs::write(&results_path, serde_json::to_string_pretty(&results)?)?;
    let summary = render_summary(&results);
    fs::write(output_dir.join("SUMMARY.md"), &summary)?;
    sh_println!("{summary}")?;
    sh_eprintln!("wrote {}", results_path.display())?;
    Ok(results)
}

fn load_results(path: &Path) -> Result<EvalResults> {
    let content =
        fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    let mut results: EvalResults = serde_json::from_str(&content)
        .wrap_err_with(|| format!("failed to parse {}", path.display()))?;
    ensure!(
        results.schema_version == SCHEMA_VERSION,
        "{} has schema version {}, expected {SCHEMA_VERSION}",
        path.display(),
        results.schema_version
    );
    results.summarize();
    Ok(results)
}

/// Resolves a bare binary name through `PATH` so the recorded path is absolute.
fn resolve_forge(forge_bin: &Path) -> Result<PathBuf> {
    if forge_bin.components().count() > 1 {
        return fs::canonicalize(forge_bin)
            .wrap_err_with(|| format!("forge binary {} not found", forge_bin.display()));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(forge_bin))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| eyre::eyre!("`{}` not found on PATH", forge_bin.display()))
}

fn parse_split(value: &str) -> Result<Split, String> {
    match value {
        "train" => Ok(Split::Train),
        "test" => Ok(Split::Test),
        _ => Err(format!("invalid split `{value}`, expected `train` or `test`")),
    }
}
