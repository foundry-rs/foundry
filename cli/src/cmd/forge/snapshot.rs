//! Snapshot command
use crate::cmd::{
    forge::{
        build::CoreBuildArgs,
        test,
        test::{custom_run, Test, TestOutcome},
    },
    Cmd,
};
use clap::{Parser, ValueHint};
use eyre::Context;
use forge::TestKindGas;
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
    Regex::new(r"(?P<file>(.*?)):(?P<sig>(\w+)\s*\((.*?)\))\s*\(((gas:)?\s*(?P<gas>\d+)|(runs:\s*(?P<runs>\d+),\s*μ:\s*(?P<avg>\d+),\s*~:\s*(?P<med>\d+)))\)").unwrap()
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
    /// By default the comparison is done with .gas-snapshot.
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
    /// By default the comparison is done with .gas-snapshot.
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

    /// Include the mean and median gas use of fuzz tests in the snapshot.
    #[clap(long, env = "FORGE_INCLUDE_FUZZ_TESTS")]
    pub include_fuzz_tests: bool,
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

    fn run(self) -> eyre::Result<()> {
        let outcome = custom_run(self.test, self.include_fuzz_tests)?;
        outcome.ensure_ok()?;
        let tests = self.config.apply(outcome);

        if let Some(path) = self.diff {
            let snap = path.as_ref().unwrap_or(&self.snap);
            let snaps = read_snapshot(snap)?;
            diff(tests, snaps)?;
        } else if let Some(path) = self.check {
            let snap = path.as_ref().unwrap_or(&self.snap);
            let snaps = read_snapshot(snap)?;
            if check(tests, snaps) {
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
    #[clap(help = "Only include tests that used more gas that the given amount.", long)]
    min: Option<u64>,
    #[clap(help = "Only include tests that used less gas that the given amount.", long)]
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
/// Has the form `<signature>(gas:? 40181)`
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SnapshotEntry {
    pub contract_name: String,
    pub signature: String,
    pub gas_used: TestKindGas,
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
                                gas_used: TestKindGas::Standard(gas.as_str().parse().unwrap()),
                            })
                        } else {
                            cap.name("runs")
                                .and_then(|runs| {
                                    cap.name("avg")
                                        .and_then(|avg| cap.name("med").map(|med| (runs, avg, med)))
                                })
                                .map(|(runs, avg, med)| SnapshotEntry {
                                    contract_name: file.as_str().to_string(),
                                    signature: sig.as_str().to_string(),
                                    gas_used: TestKindGas::Fuzz {
                                        runs: runs.as_str().parse().unwrap(),
                                        median: med.as_str().parse().unwrap(),
                                        mean: avg.as_str().parse().unwrap(),
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
        writeln!(
            out,
            "{}:{} {}",
            test.contract_name(),
            test.signature,
            test.result.kind.gas_used()
        )?;
    }
    Ok(fs::write(path, out)?)
}

/// A Snapshot entry diff
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SnapshotDiff {
    pub signature: String,
    pub source_gas_used: TestKindGas,
    pub target_gas_used: TestKindGas,
}

impl SnapshotDiff {
    /// Returns the gas diff
    ///
    /// `> 0` if the source used more gas
    /// `< 0` if the source used more gas
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
fn check(tests: Vec<Test>, snaps: Vec<SnapshotEntry>) -> bool {
    let snaps = snaps
        .into_iter()
        .map(|s| ((s.contract_name, s.signature), s.gas_used))
        .collect::<HashMap<_, _>>();
    let mut has_diff = false;
    for test in tests {
        if let Some(target_gas) =
            snaps.get(&(test.contract_name().to_string(), test.signature.clone())).cloned()
        {
            let source_gas = test.result.kind.gas_used();
            if source_gas.gas() != target_gas.gas() {
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
            source_gas_used: test.result.kind.gas_used(),
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
    match change.partial_cmp(&0.0).unwrap_or(Ordering::Equal) {
        Ordering::Less => Paint::green(format!("{:.3}%", change)).to_string(),
        Ordering::Equal => {
            format!("{:.3}%", change)
        }
        Ordering::Greater => Paint::red(format!("{:.3}%", change)).to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_basic_snapshot_entry() {
        let s = "Test:deposit() (gas: 7222)";
        let entry = SnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            SnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindGas::Standard(7222)
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
                gas_used: TestKindGas::Fuzz { runs: 256, median: 200, mean: 100 }
            }
        );
    }
}
