//! Fuzz-campaign benchmarks.
//!
//! Unlike the timing benchmarks driven by hyperfine, fuzz campaigns measure
//! coverage-style signals over a fixed time budget: how many fuzzer runs the
//! invariant completed, how many handler calls were executed, how many of those
//! reverted, and how many handler-level assertion "canaries" were hit.

use crate::RepoConfig;
use eyre::{Result, WrapErr};
use foundry_common::sh_println;
use std::{path::Path, process::Command, str::FromStr};

/// Per-campaign invariant timeout (seconds). Matches the reference run.
pub const FUZZ_TIMEOUT_SECS: u64 = 30; // tmp change

/// Fuzz seed pinned across runs so results are reproducible.
pub const FUZZ_SEED: &str = "42";

/// A single fuzz campaign: which harness contract and which `invariant_*`
/// function to drive in which repo.
#[derive(Debug, Clone)]
pub struct FuzzCampaignSpec {
    pub repo: RepoConfig,
    /// Harness contract (passed to `forge test --mc`).
    pub contract: String,
    /// Invariant function name (passed to `forge test --mt`).
    pub test: String,
}

impl FromStr for FuzzCampaignSpec {
    type Err = eyre::Error;

    /// Parse a campaign spec of the form `org/repo[:rev];Contract;test[ <extra>]`.
    fn from_str(spec: &str) -> Result<Self> {
        let parts: Vec<&str> = spec.splitn(3, ';').collect();
        if parts.len() != 3 {
            eyre::bail!(
                "Invalid fuzz campaign spec '{spec}'. Expected 'org/repo[:rev];Contract;test'"
            );
        }
        let repo = parts[0].trim().parse::<RepoConfig>()?;
        let contract = parts[1].trim().to_string();
        let test = parts[2].trim().to_string();
        if contract.is_empty() || test.is_empty() {
            eyre::bail!("Empty contract or test in fuzz campaign spec '{spec}'");
        }
        Ok(Self { repo, contract, test })
    }
}

/// Parsed result of one fuzz campaign for one foundry version.
#[derive(Debug, Clone, Default)]
pub struct FuzzCampaignResult {
    pub runs: u64,
    pub calls: u64,
    pub reverts: u64,
    /// Total handler-level assertion bugs reported by forge for this campaign.
    pub assertion_bugs: u64,
}

/// Set up a fuzz project: shallow git clone, branch checkout, submodule init
/// with SSH→HTTPS rewrite (required for repos like Recon-Fuzz/* whose
/// `.gitmodules` use `git@github.com:` URLs).
#[allow(unused_must_use)]
pub fn setup_fuzz_project(config: &RepoConfig, root: &Path) -> Result<()> {
    if root.exists() {
        std::fs::remove_dir_all(root).ok();
    }
    std::fs::create_dir_all(root)?;

    let repo_url = format!("https://github.com/{}/{}.git", config.org, config.repo);
    sh_println!("  📥 Cloning {repo_url} into {}", root.display());
    let status = Command::new("git")
        .args(["clone", "--no-recurse-submodules", &repo_url, root.to_str().unwrap()])
        .status()
        .wrap_err("git clone failed")?;
    if !status.success() {
        eyre::bail!("git clone failed for {}", config.name);
    }

    if !config.rev.is_empty() && config.rev != "main" && config.rev != "master" {
        sh_println!("  🔀 Checking out {}", config.rev);
        let status = Command::new("git")
            .current_dir(root)
            .args(["checkout", &config.rev])
            .status()
            .wrap_err("git checkout failed")?;
        if !status.success() {
            eyre::bail!("git checkout {} failed for {}", config.rev, config.name);
        }
    }

    sh_println!("  🔗 Initialising submodules");
    let status = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "url.https://github.com/.insteadOf=git@github.com:",
            "submodule",
            "update",
            "--init",
            "--recursive",
        ])
        .status()
        .wrap_err("git submodule update failed")?;
    if !status.success() {
        eyre::bail!("git submodule update failed for {}", config.name);
    }

    Ok(())
}

/// Run a single fuzz campaign and parse its result.
///
/// The campaign always uses `FOUNDRY_INVARIANT_TIMEOUT=3600` and `--fuzz-seed 42`
/// so cross-version numbers are directly comparable to the PR reference table.
///
/// A non-zero forge exit code is expected and intentionally ignored: handler
/// "canary" assertions are the coverage signal and they make forge exit with
/// failure. We parse stdout regardless.
#[allow(unused_must_use)]
pub fn run_campaign(
    project_root: &Path,
    spec: &FuzzCampaignSpec,
    version: &str,
    timeout_secs: u64,
    verbose: bool,
) -> Result<FuzzCampaignResult> {
    // Always start from a clean cache so we never replay a cached failure
    // from a previous run.
    sh_println!("  🧹 forge clean");
    Command::new("forge").current_dir(project_root).arg("clean").status().ok();
    std::fs::remove_dir_all(project_root.join("cache")).ok();
    std::fs::remove_dir_all(project_root.join("corpus")).ok();

    let extra = spec.repo.extra_args.as_deref().map(str::trim).filter(|s| !s.is_empty());
    sh_println!(
        "  🚀 [{version}] forge test --mc {} --mt {} --fuzz-seed {FUZZ_SEED}  (timeout={timeout_secs}s){}",
        spec.contract,
        spec.test,
        extra.map(|e| format!("  + {e}")).unwrap_or_default(),
    );

    let mut cmd = Command::new("forge");
    cmd.current_dir(project_root)
        .env("FOUNDRY_INVARIANT_TIMEOUT", timeout_secs.to_string())
        .args(["test", "--mc", &spec.contract, "--mt", &spec.test, "--fuzz-seed", FUZZ_SEED]);
    if let Some(extra) = extra {
        for arg in extra.split_whitespace() {
            cmd.arg(arg);
        }
    }

    let output = if verbose {
        // Stream stdout/stderr live to the workflow log AND capture for parsing.
        let proc = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .wrap_err("Failed to spawn forge")?;
        let out = proc.wait_with_output().wrap_err("forge wait failed")?;
        // Echo so workflow logs show the full forge output.
        if !out.stdout.is_empty() {
            foundry_common::sh_print!("{}", String::from_utf8_lossy(&out.stdout));
        }
        if !out.stderr.is_empty() {
            foundry_common::sh_eprint!("{}", String::from_utf8_lossy(&out.stderr));
        }
        out
    } else {
        cmd.output().wrap_err("Failed to run forge")?
    };

    // Exit code intentionally ignored: assertion canaries flip it to non-zero
    // on every successful campaign.
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_campaign_output(&stdout, &spec.test)
}

/// Extract `runs`, `calls`, `reverts` and handler assertion-bug count for a
/// given invariant from forge stdout.
///
/// Expected lines:
/// * ` invariant_noop() (runs: 253, calls: 25300, reverts: 11554)`
/// * `Suite handlers: 1 assertion bug(s) found`
fn parse_campaign_output(stdout: &str, test_name: &str) -> Result<FuzzCampaignResult> {
    let mut result = FuzzCampaignResult::default();

    let test_marker = format!("{test_name}()");
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&test_marker)
            && let Some(stats_start) = trimmed.find("(runs:")
        {
            let stats = &trimmed[stats_start + 1..];
            let stats = stats.trim_end_matches(')');
            for kv in stats.split(',') {
                let mut it = kv.split(':');
                let k = it.next().map(str::trim).unwrap_or("");
                let v = it.next().map(str::trim).unwrap_or("");
                let n: u64 = v.parse().unwrap_or(0);
                match k {
                    "runs" => result.runs = n,
                    "calls" => result.calls = n,
                    "reverts" => result.reverts = n,
                    _ => {}
                }
            }
        }

        if trimmed.starts_with("Suite handlers:")
            && let Some(rest) = trimmed.strip_prefix("Suite handlers:")
        {
            // "Suite handlers: 1 assertion bug(s) found"
            for tok in rest.split_whitespace() {
                if let Ok(n) = tok.parse::<u64>() {
                    result.assertion_bugs = n;
                    break;
                }
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_invariant_stats() {
        let out = "\
some preamble
[FAIL: ] invariant_canary
Suite assert_all: 1/10 invariants broken
Suite handlers: 1 assertion bug(s) found
[FAIL: assertion failed] tests/recon/CryticToFoundry.sol:CryticToFoundry::assert_canary_ASSERTION_CANARY
\t[Sequence] (original: 8, shrunk: 1)
\t\tvm.warp(block.timestamp + 1886183);
 invariant_noop() (runs: 253, calls: 25300, reverts: 11554)
";
        let r = parse_campaign_output(out, "invariant_noop").unwrap();
        assert_eq!(r.runs, 253);
        assert_eq!(r.calls, 25300);
        assert_eq!(r.reverts, 11554);
        assert_eq!(r.assertion_bugs, 1);
    }

    #[test]
    fn missing_lines_default_to_zero() {
        let r = parse_campaign_output("nothing here\n", "invariant_noop").unwrap();
        assert_eq!(r.runs, 0);
        assert_eq!(r.calls, 0);
        assert_eq!(r.reverts, 0);
        assert_eq!(r.assertion_bugs, 0);
    }

    #[test]
    fn parse_spec_round_trip() {
        let s: FuzzCampaignSpec =
            "Recon-Fuzz/aave-v4-scfuzzbench:v0.5.6-recon;CryticToFoundry;invariant_noop"
                .parse()
                .unwrap();
        assert_eq!(s.repo.org, "Recon-Fuzz");
        assert_eq!(s.repo.repo, "aave-v4-scfuzzbench");
        assert_eq!(s.repo.rev, "v0.5.6-recon");
        assert_eq!(s.contract, "CryticToFoundry");
        assert_eq!(s.test, "invariant_noop");
    }
}
