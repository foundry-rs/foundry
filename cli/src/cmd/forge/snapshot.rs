//! Snapshot command
use crate::{
    cmd::{
        forge::{
            build::CoreBuildArgs,
            test,
            test::{custom_run, Test, TestOutcome},
        },
        u32_validator, Cmd,
    },
    utils::STATIC_FUZZ_SEED,
};
use clap::{Parser, ValueHint};
use ethers::types::U256;
use eyre::Context;
use forge::result::TestKindReport;
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    cmp::Ordering,
    collections::HashMap,
    fmt::Write,
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
    str::FromStr,
};
use watchexec::config::{InitConfig, RuntimeConfig};
use yansi::Paint;

/// A regex that matches a basic snapshot entry like
/// `Test:testDeposit() (gas: 58804)`
pub static RE_BASIC_SNAPSHOT_ENTRY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?P<file>(.*?)):(?P<sig>(\w+)\s*\((.*?)\))\s*\(((gas:)?\s*(?P<gas>\d+)|(runs:\s*(?P<runs>\d+),\s*μ:\s*(?P<avg>\d+),\s*~:\s*(?P<med>\d+))|(runs:\s*(?P<invruns>\d+),\s*calls:\s*(?P<calls>\d+),\s*reverts:\s*(?P<reverts>\d+)))\)").unwrap()
});

#[derive(Debug, Clone, Parser)]
pub struct SnapshotArgs {
    /// All test arguments are supported
    #[clap(flatten, next_help_heading = "TEST OPTIONS")]
    pub(crate) test: test::TestArgs,

    /// Additional configs for test results
    #[clap(flatten)]
    config: SnapshotConfig,

    /// Output a diff against a pre-existing snapshot.
    ///
    /// By default, the comparison is done with .gas-snapshot.
    #[clap(
        conflicts_with = "snap",
        long,
        value_hint = ValueHint::FilePath,
        value_name = "SNAPSHOT_FILE",
    )]
    diff: Option<Option<PathBuf>>,

    /// Compare against a pre-existing snapshot, exiting with code 1 if they do not match.
    ///
    /// Outputs a diff if the snapshots do not match.
    ///
    /// By default, the comparison is done with .gas-snapshot.
    #[clap(
        conflicts_with = "diff",
        long,
        value_hint = ValueHint::FilePath,
        value_name = "SNAPSHOT_FILE",
    )]
    check: Option<Option<PathBuf>>,

    // Hidden because there is only one option
    #[clap(help = "How to format the output.", long, hide(true))]
    format: Option<Format>,

    #[clap(
        help = "Output file for the snapshot.",
        default_value = ".gas-snapshot",
        long,
        value_name = "SNAPSHOT_FILE"
    )]
    snap: PathBuf,

    #[clap(
        help = "Tolerates gas deviations up to the specified percentage.",
        long,
        validator = u32_validator(0, 100),
        value_name = "SNAPSHOT_THRESHOLD"
    )]
    tolerance: Option<u32>,
}

impl SnapshotArgs {
    /// Returns whether `SnapshotArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.test.is_watch()
    }

    /// Returns the [`watchexec::InitConfig`] and [`watchexec::RuntimeConfig`] necessary to
    /// bootstrap a new [`watchexe::Watchexec`] loop.
    pub(crate) fn watchexec_config(&self) -> eyre::Result<(InitConfig, RuntimeConfig)> {
        self.test.watchexec_config()
    }

    /// Returns the nested [`CoreBuildArgs`]
    pub fn build_args(&self) -> &CoreBuildArgs {
        self.test.build_args()
    }
}

impl Cmd for SnapshotArgs {
    type Output = ();

    fn run(mut self) -> eyre::Result<()> {
        // Set fuzz seed so gas snapshots are deterministic
        self.test.fuzz_seed = Some(U256::from_big_endian(&STATIC_FUZZ_SEED));

        let outcome = custom_run(self.test)?;
        outcome.ensure_ok()?;
        let tests = self.config.apply(outcome);

        if let Some(path) = self.diff {
            let snap = path.as_ref().unwrap_or(&self.snap);
            let snaps = read_snapshot(snap)?;
            diff(tests, snaps)?;
        } else if let Some(path) = self.check {
            let snap = path.as_ref().unwrap_or(&self.snap);
            let snaps = read_snapshot(snap)?;
            if check(tests, snaps, self.tolerance) {
                std::process::exit(0)
            } else {
                std::process::exit(1)
            }
        } else {
            write_to_snapshot_file(&tests, self.snap, self.format)?;
        }
        Ok(())
    }
}

// TODO implement pretty tables
#[derive(Debug, Clone)]
pub enum Format {
    Table,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "t" | "table" => Ok(Format::Table),
            _ => Err(format!("Unrecognized format `{s}`")),
        }
    }
}

/// Additional filters that can be applied on the test results
#[derive(Debug, Clone, Parser, Default)]
struct SnapshotConfig {
    #[clap(help = "Sort results by gas used (ascending).", long)]
    asc: bool,
    #[clap(help = "Sort results by gas used (descending).", conflicts_with = "asc", long)]
    desc: bool,
    #[clap(
        help = "Only include tests that used more gas that the given amount.",
        long,
        value_name = "MIN_GAS"
    )]
    min: Option<u64>,
    #[clap(
        help = "Only include tests that used less gas that the given amount.",
        long,
        value_name = "MAX_GAS"
    )]
    max: Option<u64>,
}

impl SnapshotConfig {
    fn is_in_gas_range(&self, gas_used: u64) -> bool {
        if let Some(min) = self.min {
            if gas_used < min {
                return false
            }
        }
        if let Some(max) = self.max {
            if gas_used > max {
                return false
            }
        }
        true
    }

    fn apply(&self, outcome: TestOutcome) -> Vec<Test> {
        let mut tests = outcome
            .into_tests()
            .filter(|test| self.is_in_gas_range(test.gas_used()))
            .collect::<Vec<_>>();

        if self.asc {
            tests.sort_by_key(|a| a.gas_used());
        } else if self.desc {
            tests.sort_by_key(|b| std::cmp::Reverse(b.gas_used()))
        }

        tests
    }
}

/// A general entry in a snapshot file
///
/// Has the form:
///   `<signature>(gas:? 40181)` for normal tests
///   `<signature>(runs: 256, μ: 40181, ~: 40181)` for fuzz tests
///   `<signature>(runs: 256, calls: 40181, reverts: 40181)` for invariant tests
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SnapshotEntry {
    pub contract_name: String,
    pub signature: String,
    pub gas_used: TestKindReport,
}

impl FromStr for SnapshotEntry {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RE_BASIC_SNAPSHOT_ENTRY
            .captures(s)
            .and_then(|cap| {
                cap.name("file").and_then(|file| {
                    cap.name("sig").and_then(|sig| {
                        if let Some(gas) = cap.name("gas") {
                            Some(SnapshotEntry {
                                contract_name: file.as_str().to_string(),
                                signature: sig.as_str().to_string(),
                                gas_used: TestKindReport::Standard {
                                    gas: gas.as_str().parse().unwrap(),
                                },
                            })
                        } else if let Some(runs) = cap.name("runs") {
                            cap.name("avg")
                                .and_then(|avg| cap.name("med").map(|med| (runs, avg, med)))
                                .map(|(runs, avg, med)| SnapshotEntry {
                                    contract_name: file.as_str().to_string(),
                                    signature: sig.as_str().to_string(),
                                    gas_used: TestKindReport::Fuzz {
                                        runs: runs.as_str().parse().unwrap(),
                                        median_gas: med.as_str().parse().unwrap(),
                                        mean_gas: avg.as_str().parse().unwrap(),
                                    },
                                })
                        } else {
                            cap.name("invruns")
                                .and_then(|runs| {
                                    cap.name("calls").and_then(|avg| {
                                        cap.name("reverts").map(|med| (runs, avg, med))
                                    })
                                })
                                .map(|(runs, calls, reverts)| SnapshotEntry {
                                    contract_name: file.as_str().to_string(),
                                    signature: sig.as_str().to_string(),
                                    gas_used: TestKindReport::Invariant {
                                        runs: runs.as_str().parse().unwrap(),
                                        calls: calls.as_str().parse().unwrap(),
                                        reverts: reverts.as_str().parse().unwrap(),
                                    },
                                })
                        }
                    })
                })
            })
            .ok_or_else(|| format!("Could not extract Snapshot Entry for {s}"))
    }
}

/// Reads a list of snapshot entries from a snapshot file
fn read_snapshot(path: impl AsRef<Path>) -> eyre::Result<Vec<SnapshotEntry>> {
    let path = path.as_ref();
    let mut entries = Vec::new();
    for line in io::BufReader::new(
        fs::File::open(path)
            .wrap_err(format!("failed to read snapshot file \"{}\"", path.display()))?,
    )
    .lines()
    {
        entries.push(SnapshotEntry::from_str(line?.as_str()).map_err(|err| eyre::eyre!("{err}"))?);
    }
    Ok(entries)
}

/// Writes a series of tests to a snapshot file
fn write_to_snapshot_file(
    tests: &[Test],
    path: impl AsRef<Path>,
    _format: Option<Format>,
) -> eyre::Result<()> {
    let mut out = String::new();
    for test in tests {
        writeln!(out, "{}:{} {}", test.contract_name(), test.signature, test.result.kind.report())?;
    }
    Ok(fs::write(path, out)?)
}

/// A Snapshot entry diff
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SnapshotDiff {
    pub signature: String,
    pub source_gas_used: TestKindReport,
    pub target_gas_used: TestKindReport,
}

impl SnapshotDiff {
    /// Returns the gas diff
    ///
    /// `> 0` if the source used more gas
    /// `< 0` if the target used more gas
    fn gas_change(&self) -> i128 {
        self.source_gas_used.gas() as i128 - self.target_gas_used.gas() as i128
    }

    /// Determines the percentage change
    fn gas_diff(&self) -> f64 {
        self.gas_change() as f64 / self.target_gas_used.gas() as f64
    }
}

/// Compares the set of tests with an existing snapshot
///
/// Returns true all tests match
fn check(tests: Vec<Test>, snaps: Vec<SnapshotEntry>, tolerance: Option<u32>) -> bool {
    let snaps = snaps
        .into_iter()
        .map(|s| ((s.contract_name, s.signature), s.gas_used))
        .collect::<HashMap<_, _>>();
    let mut has_diff = false;
    for test in tests {
        if let Some(target_gas) =
            snaps.get(&(test.contract_name().to_string(), test.signature.clone())).cloned()
        {
            let source_gas = test.result.kind.report();
            if !within_tolerance(source_gas.gas(), target_gas.gas(), tolerance) {
                eprintln!(
                    "Diff in \"{}::{}\": consumed \"{}\" gas, expected \"{}\" gas ",
                    test.contract_name(),
                    test.signature,
                    source_gas,
                    target_gas
                );
                has_diff = true;
            }
        } else {
            eprintln!(
                "No matching snapshot entry found for \"{}::{}\" in snapshot file",
                test.contract_name(),
                test.signature
            );
            has_diff = true;
        }
    }
    !has_diff
}

/// Compare the set of tests with an existing snapshot
fn diff(tests: Vec<Test>, snaps: Vec<SnapshotEntry>) -> eyre::Result<()> {
    let snaps = snaps
        .into_iter()
        .map(|s| ((s.contract_name, s.signature), s.gas_used))
        .collect::<HashMap<_, _>>();
    let mut diffs = Vec::with_capacity(tests.len());
    for test in tests.into_iter() {
        let target_gas_used = snaps
            .get(&(test.contract_name().to_string(), test.signature.clone()))
            .cloned()
            .ok_or_else(|| {
                eyre::eyre!(
                    "No matching snapshot entry found for \"{}\" in snapshot file",
                    test.signature
                )
            })?;

        diffs.push(SnapshotDiff {
            source_gas_used: test.result.kind.report(),
            signature: test.signature,
            target_gas_used,
        });
    }
    let mut overall_gas_change = 0i128;
    let mut overall_gas_diff = 0f64;

    diffs.sort_by(|a, b| {
        a.gas_diff().abs().partial_cmp(&b.gas_diff().abs()).unwrap_or(Ordering::Equal)
    });

    for diff in diffs {
        let gas_change = diff.gas_change();
        overall_gas_change += gas_change;
        let gas_diff = diff.gas_diff();
        overall_gas_diff += gas_diff;
        println!(
            "{} (gas: {} ({})) ",
            diff.signature,
            fmt_change(gas_change),
            fmt_pct_change(gas_diff)
        );
    }

    println!(
        "Overall gas change: {} ({})",
        fmt_change(overall_gas_change),
        fmt_pct_change(overall_gas_diff)
    );
    Ok(())
}

fn fmt_pct_change(change: f64) -> String {
    let change_pct = change * 100.0;
    match change.partial_cmp(&0.0).unwrap_or(Ordering::Equal) {
        Ordering::Less => Paint::green(format!("{:.3}%", change_pct)).to_string(),
        Ordering::Equal => {
            format!("{:.3}%", change_pct)
        }
        Ordering::Greater => Paint::red(format!("{:.3}%", change_pct)).to_string(),
    }
}

fn fmt_change(change: i128) -> String {
    match change.cmp(&0) {
        Ordering::Less => Paint::green(format!("{change}")).to_string(),
        Ordering::Equal => {
            format!("{change}")
        }
        Ordering::Greater => Paint::red(format!("{change}")).to_string(),
    }
}

/// Returns true of the difference between the gas values exceeds the tolerance
///
/// If `tolerance` is `None`, then this returns `true` if both gas values are equal
fn within_tolerance(source_gas: u64, target_gas: u64, tolerance_pct: Option<u32>) -> bool {
    if let Some(tolerance) = tolerance_pct {
        let (hi, lo) = if source_gas > target_gas {
            (source_gas, target_gas)
        } else {
            (target_gas, source_gas)
        };
        let diff = (1. - (lo as f64 / hi as f64)) * 100.;
        diff < tolerance as f64
    } else {
        source_gas == target_gas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tolerance() {
        assert!(within_tolerance(100, 105, Some(5)));
        assert!(within_tolerance(105, 100, Some(5)));
        assert!(!within_tolerance(100, 106, Some(5)));
        assert!(!within_tolerance(106, 100, Some(5)));
        assert!(within_tolerance(100, 100, None));
    }

    #[test]
    fn can_parse_basic_snapshot_entry() {
        let s = "Test:deposit() (gas: 7222)";
        let entry = SnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            SnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindReport::Standard { gas: 7222 }
            }
        );
    }

    #[test]
    fn can_parse_fuzz_snapshot_entry() {
        let s = "Test:deposit() (runs: 256, μ: 100, ~:200)";
        let entry = SnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            SnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindReport::Fuzz { runs: 256, median_gas: 200, mean_gas: 100 }
            }
        );
    }

    #[test]
    fn can_parse_invariant_snapshot_entry() {
        let s = "Test:deposit() (runs: 256, calls: 100, reverts: 200)";
        let entry = SnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            SnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindReport::Invariant { runs: 256, calls: 100, reverts: 200 }
            }
        );
    }
}
