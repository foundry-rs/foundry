use super::test;
use crate::result::{SuiteTestResult, TestKindReport, TestOutcome};
use alloy_primitives::{U256, map::HashMap};
use clap::{Parser, ValueHint, builder::RangedU64ValueParser};
use comfy_table::{
    Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN,
};
use eyre::{Context, Result};
use foundry_cli::utils::STATIC_FUZZ_SEED;
use foundry_common::shell;
use regex::Regex;
use std::{
    cmp::Ordering,
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
    str::FromStr,
    sync::LazyLock,
};
use yansi::Paint;

/// A regex that matches a basic snapshot entry like
/// `Test:testDeposit() (gas: 58804)`
pub static RE_BASIC_SNAPSHOT_ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<file>(.*?)):(?P<sig>(\w+)\s*\((.*?)\))\s*\(((gas:)?\s*(?P<gas>\d+)|(runs:\s*(?P<runs>\d+),\s*μ:\s*(?P<avg>\d+),\s*~:\s*(?P<med>\d+))|(runs:\s*(?P<invruns>\d+),\s*calls:\s*(?P<calls>\d+),\s*reverts:\s*(?P<reverts>\d+)))\)").unwrap()
});

/// CLI arguments for `forge snapshot`.
#[derive(Clone, Debug, Parser)]
pub struct GasSnapshotArgs {
    /// Output a diff against a pre-existing gas snapshot.
    ///
    /// By default, the comparison is done with .gas-snapshot.
    #[arg(
        conflicts_with = "snap",
        long,
        value_hint = ValueHint::FilePath,
        value_name = "SNAPSHOT_FILE",
    )]
    diff: Option<Option<PathBuf>>,

    /// Compare against a pre-existing gas snapshot, exiting with code 1 if they do not match.
    ///
    /// Outputs a diff if the gas snapshots do not match.
    ///
    /// By default, the comparison is done with .gas-snapshot.
    #[arg(
        conflicts_with = "diff",
        long,
        value_hint = ValueHint::FilePath,
        value_name = "SNAPSHOT_FILE",
    )]
    check: Option<Option<PathBuf>>,

    // Hidden because there is only one option
    /// How to format the output.
    #[arg(long, hide(true))]
    format: Option<Format>,

    /// Output file for the gas snapshot.
    #[arg(
        long,
        default_value = ".gas-snapshot",
        value_hint = ValueHint::FilePath,
        value_name = "FILE",
    )]
    snap: PathBuf,

    /// Tolerates gas deviations up to the specified percentage.
    #[arg(
        long,
        value_parser = RangedU64ValueParser::<u32>::new().range(0..100),
        value_name = "SNAPSHOT_THRESHOLD"
    )]
    tolerance: Option<u32>,

    /// How to sort diff results.
    #[arg(long, value_name = "ORDER")]
    diff_sort: Option<DiffSortOrder>,

    /// All test arguments are supported
    #[command(flatten)]
    pub(crate) test: test::TestArgs,

    /// Additional configs for test results
    #[command(flatten)]
    config: GasSnapshotConfig,
}

impl GasSnapshotArgs {
    /// Returns whether `GasSnapshotArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.test.is_watch()
    }

    /// Returns the [`watchexec::Config`] necessary to bootstrap a new watch loop.
    pub(crate) fn watchexec_config(&self) -> Result<watchexec::Config> {
        self.test.watchexec_config()
    }

    pub async fn run(mut self) -> Result<()> {
        // Set fuzz seed so gas snapshots are deterministic
        self.test.fuzz_seed = Some(U256::from_be_bytes(STATIC_FUZZ_SEED));

        let outcome = self.test.compile_and_run().await?;
        outcome.ensure_ok(false)?;
        let tests = self.config.apply(outcome);

        if let Some(path) = self.diff {
            let snap = path.as_ref().unwrap_or(&self.snap);
            let snaps = read_gas_snapshot(snap)?;
            diff(tests, snaps, self.diff_sort.unwrap_or_default())?;
        } else if let Some(path) = self.check {
            let snap = path.as_ref().unwrap_or(&self.snap);
            let snaps = read_gas_snapshot(snap)?;
            if check(tests, snaps, self.tolerance) {
                std::process::exit(0)
            } else {
                std::process::exit(1)
            }
        } else {
            if matches!(self.format, Some(Format::Table)) {
                let table = build_gas_snapshot_table(&tests);
                sh_println!("\n{}", table)?;
            }
            write_to_gas_snapshot_file(&tests, self.snap, self.format)?;
        }
        Ok(())
    }
}

// Gas report format on stdout.
#[derive(Clone, Debug)]
pub enum Format {
    Table,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "t" | "table" => Ok(Self::Table),
            _ => Err(format!("Unrecognized format `{s}`")),
        }
    }
}

/// Additional filters that can be applied on the test results
#[derive(Clone, Debug, Default, Parser)]
struct GasSnapshotConfig {
    /// Sort results by gas used (ascending).
    #[arg(long)]
    asc: bool,

    /// Sort results by gas used (descending).
    #[arg(conflicts_with = "asc", long)]
    desc: bool,

    /// Only include tests that used more gas that the given amount.
    #[arg(long, value_name = "MIN_GAS")]
    min: Option<u64>,

    /// Only include tests that used less gas that the given amount.
    #[arg(long, value_name = "MAX_GAS")]
    max: Option<u64>,
}

/// Sort order for diff output
#[derive(Clone, Debug, Default, clap::ValueEnum)]
enum DiffSortOrder {
    /// Sort by percentage change (smallest to largest) - default behavior
    #[default]
    Percentage,
    /// Sort by percentage change (largest to smallest)
    PercentageDesc,
    /// Sort by absolute gas change (smallest to largest)
    Absolute,
    /// Sort by absolute gas change (largest to smallest)
    AbsoluteDesc,
}

impl GasSnapshotConfig {
    fn is_in_gas_range(&self, gas_used: u64) -> bool {
        if let Some(min) = self.min
            && gas_used < min
        {
            return false;
        }
        if let Some(max) = self.max
            && gas_used > max
        {
            return false;
        }
        true
    }

    fn apply(&self, outcome: TestOutcome) -> Vec<SuiteTestResult> {
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

/// A general entry in a gas snapshot file
///
/// Has the form:
///   `<signature>(gas:? 40181)` for normal tests
///   `<signature>(runs: 256, μ: 40181, ~: 40181)` for fuzz tests
///   `<signature>(runs: 256, calls: 40181, reverts: 40181)` for invariant tests
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GasSnapshotEntry {
    pub contract_name: String,
    pub signature: String,
    pub gas_used: TestKindReport,
}

impl FromStr for GasSnapshotEntry {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RE_BASIC_SNAPSHOT_ENTRY
            .captures(s)
            .and_then(|cap| {
                cap.name("file").and_then(|file| {
                    cap.name("sig").and_then(|sig| {
                        if let Some(gas) = cap.name("gas") {
                            Some(Self {
                                contract_name: file.as_str().to_string(),
                                signature: sig.as_str().to_string(),
                                gas_used: TestKindReport::Unit {
                                    gas: gas.as_str().parse().unwrap(),
                                },
                            })
                        } else if let Some(runs) = cap.name("runs") {
                            cap.name("avg")
                                .and_then(|avg| cap.name("med").map(|med| (runs, avg, med)))
                                .map(|(runs, avg, med)| Self {
                                    contract_name: file.as_str().to_string(),
                                    signature: sig.as_str().to_string(),
                                    gas_used: TestKindReport::Fuzz {
                                        runs: runs.as_str().parse().unwrap(),
                                        median_gas: med.as_str().parse().unwrap(),
                                        mean_gas: avg.as_str().parse().unwrap(),
                                        failed_corpus_replays: 0,
                                    },
                                })
                        } else {
                            cap.name("invruns")
                                .and_then(|runs| {
                                    cap.name("calls").and_then(|avg| {
                                        cap.name("reverts").map(|med| (runs, avg, med))
                                    })
                                })
                                .map(|(runs, calls, reverts)| Self {
                                    contract_name: file.as_str().to_string(),
                                    signature: sig.as_str().to_string(),
                                    gas_used: TestKindReport::Invariant {
                                        runs: runs.as_str().parse().unwrap(),
                                        calls: calls.as_str().parse().unwrap(),
                                        reverts: reverts.as_str().parse().unwrap(),
                                        metrics: HashMap::default(),
                                        failed_corpus_replays: 0,
                                    },
                                })
                        }
                    })
                })
            })
            .ok_or_else(|| format!("Could not extract Snapshot Entry for {s}"))
    }
}

/// Reads a list of gas snapshot entries from a gas snapshot file.
fn read_gas_snapshot(path: impl AsRef<Path>) -> Result<Vec<GasSnapshotEntry>> {
    let path = path.as_ref();
    let mut entries = Vec::new();
    for line in io::BufReader::new(
        fs::File::open(path)
            .wrap_err(format!("failed to read snapshot file \"{}\"", path.display()))?,
    )
    .lines()
    {
        entries
            .push(GasSnapshotEntry::from_str(line?.as_str()).map_err(|err| eyre::eyre!("{err}"))?);
    }
    Ok(entries)
}

/// Writes a series of tests to a gas snapshot file after sorting them.
fn write_to_gas_snapshot_file(
    tests: &[SuiteTestResult],
    path: impl AsRef<Path>,
    _format: Option<Format>,
) -> Result<()> {
    let mut reports = tests
        .iter()
        .map(|test| {
            format!("{}:{} {}", test.contract_name(), test.signature, test.result.kind.report())
        })
        .collect::<Vec<_>>();

    // sort all reports
    reports.sort();

    let content = reports.join("\n");
    Ok(fs::write(path, content)?)
}

fn build_gas_snapshot_table(tests: &[SuiteTestResult]) -> Table {
    let mut table = Table::new();
    if shell::is_markdown() {
        table.load_preset(ASCII_MARKDOWN);
    } else {
        table.apply_modifier(UTF8_ROUND_CORNERS);
    }

    table.set_header(vec![
        Cell::new("Contract").fg(Color::Cyan),
        Cell::new("Signature").fg(Color::Cyan),
        Cell::new("Report").fg(Color::Cyan),
    ]);

    for test in tests {
        let mut row = Row::new();
        row.add_cell(Cell::new(test.contract_name()));
        row.add_cell(Cell::new(&test.signature));
        row.add_cell(Cell::new(test.result.kind.report()));
        table.add_row(row);
    }

    table
}

/// A Gas snapshot entry diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GasSnapshotDiff {
    pub signature: String,
    pub source_gas_used: TestKindReport,
    pub target_gas_used: TestKindReport,
}

impl GasSnapshotDiff {
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

/// Compares the set of tests with an existing gas snapshot.
///
/// Returns true all tests match
fn check(
    tests: Vec<SuiteTestResult>,
    snaps: Vec<GasSnapshotEntry>,
    tolerance: Option<u32>,
) -> bool {
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
                let _ = sh_println!(
                    "Diff in \"{}::{}\": consumed \"{}\" gas, expected \"{}\" gas ",
                    test.contract_name(),
                    test.signature,
                    source_gas,
                    target_gas
                );
                has_diff = true;
            }
        } else {
            let _ = sh_println!(
                "No matching snapshot entry found for \"{}::{}\" in snapshot file",
                test.contract_name(),
                test.signature
            );
            has_diff = true;
        }
    }
    !has_diff
}

/// Compare the set of tests with an existing gas snapshot.
fn diff(
    tests: Vec<SuiteTestResult>,
    snaps: Vec<GasSnapshotEntry>,
    sort_order: DiffSortOrder,
) -> Result<()> {
    let snaps = snaps
        .into_iter()
        .map(|s| ((s.contract_name, s.signature), s.gas_used))
        .collect::<HashMap<_, _>>();
    let mut diffs = Vec::with_capacity(tests.len());
    let mut new_tests = Vec::new();

    for test in tests.into_iter() {
        if let Some(target_gas_used) =
            snaps.get(&(test.contract_name().to_string(), test.signature.clone())).cloned()
        {
            diffs.push(GasSnapshotDiff {
                source_gas_used: test.result.kind.report(),
                signature: format!("{}::{}", test.contract_name(), test.signature),
                target_gas_used,
            });
        } else {
            // Track new tests
            new_tests.push(format!("{}::{}", test.contract_name(), test.signature));
        }
    }

    let mut increased = 0;
    let mut decreased = 0;
    let mut unchanged = 0;
    let mut overall_gas_change = 0i128;
    let mut overall_gas_used = 0i128;

    // Sort based on user preference
    match sort_order {
        DiffSortOrder::Percentage => {
            // Default: sort by percentage change (smallest to largest)
            diffs.sort_by(|a, b| a.gas_diff().abs().total_cmp(&b.gas_diff().abs()));
        }
        DiffSortOrder::PercentageDesc => {
            // Sort by percentage change (largest to smallest)
            diffs.sort_by(|a, b| b.gas_diff().abs().total_cmp(&a.gas_diff().abs()));
        }
        DiffSortOrder::Absolute => {
            // Sort by absolute gas change (smallest to largest)
            diffs.sort_by_key(|d| d.gas_change().abs());
        }
        DiffSortOrder::AbsoluteDesc => {
            // Sort by absolute gas change (largest to smallest)
            diffs.sort_by_key(|d| std::cmp::Reverse(d.gas_change().abs()));
        }
    }

    for diff in &diffs {
        let gas_change = diff.gas_change();
        overall_gas_change += gas_change;
        overall_gas_used += diff.target_gas_used.gas() as i128;
        let gas_diff = diff.gas_diff();

        // Classify changes
        if gas_change > 0 {
            increased += 1;
        } else if gas_change < 0 {
            decreased += 1;
        } else {
            unchanged += 1;
        }

        // Display with icon and before/after values
        let icon = if gas_change > 0 {
            "↑".red().to_string()
        } else if gas_change < 0 {
            "↓".green().to_string()
        } else {
            "━".to_string()
        };

        sh_println!(
            "{} {} (gas: {} → {} | {} {})",
            icon,
            diff.signature,
            diff.target_gas_used.gas(),
            diff.source_gas_used.gas(),
            fmt_change(gas_change),
            fmt_pct_change(gas_diff)
        )?;
    }

    // Display new tests if any
    if !new_tests.is_empty() {
        sh_println!("\n{}", "New tests:".yellow())?;
        for test in new_tests {
            sh_println!("  {} {}", "+".green(), test)?;
        }
    }

    // Summary separator
    sh_println!("\n{}", "-".repeat(80))?;

    let overall_gas_diff = if overall_gas_used > 0 {
        overall_gas_change as f64 / overall_gas_used as f64
    } else {
        0.0
    };

    sh_println!(
        "Total tests: {}, {} {}, {} {}, {} {}",
        diffs.len(),
        "↑".red().to_string(),
        increased,
        "↓".green().to_string(),
        decreased,
        "━",
        unchanged
    )?;
    sh_println!(
        "Overall gas change: {} ({})",
        fmt_change(overall_gas_change),
        fmt_pct_change(overall_gas_diff)
    )?;
    Ok(())
}

fn fmt_pct_change(change: f64) -> String {
    let change_pct = change * 100.0;
    match change.total_cmp(&0.0) {
        Ordering::Less => format!("{change_pct:.3}%").green().to_string(),
        Ordering::Equal => {
            format!("{change_pct:.3}%")
        }
        Ordering::Greater => format!("{change_pct:.3}%").red().to_string(),
    }
}

fn fmt_change(change: i128) -> String {
    match change.cmp(&0) {
        Ordering::Less => format!("{change}").green().to_string(),
        Ordering::Equal => change.to_string(),
        Ordering::Greater => format!("{change}").red().to_string(),
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
    fn can_parse_basic_gas_snapshot_entry() {
        let s = "Test:deposit() (gas: 7222)";
        let entry = GasSnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            GasSnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindReport::Unit { gas: 7222 }
            }
        );
    }

    #[test]
    fn can_parse_fuzz_gas_snapshot_entry() {
        let s = "Test:deposit() (runs: 256, μ: 100, ~:200)";
        let entry = GasSnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            GasSnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindReport::Fuzz {
                    runs: 256,
                    median_gas: 200,
                    mean_gas: 100,
                    failed_corpus_replays: 0
                }
            }
        );
    }

    #[test]
    fn can_parse_invariant_gas_snapshot_entry() {
        let s = "Test:deposit() (runs: 256, calls: 100, reverts: 200)";
        let entry = GasSnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            GasSnapshotEntry {
                contract_name: "Test".to_string(),
                signature: "deposit()".to_string(),
                gas_used: TestKindReport::Invariant {
                    runs: 256,
                    calls: 100,
                    reverts: 200,
                    metrics: HashMap::default(),
                    failed_corpus_replays: 0,
                }
            }
        );
    }

    #[test]
    fn can_parse_invariant_gas_snapshot_entry2() {
        let s = "ERC20Invariants:invariantBalanceSum() (runs: 256, calls: 3840, reverts: 2388)";
        let entry = GasSnapshotEntry::from_str(s).unwrap();
        assert_eq!(
            entry,
            GasSnapshotEntry {
                contract_name: "ERC20Invariants".to_string(),
                signature: "invariantBalanceSum()".to_string(),
                gas_used: TestKindReport::Invariant {
                    runs: 256,
                    calls: 3840,
                    reverts: 2388,
                    metrics: HashMap::default(),
                    failed_corpus_replays: 0,
                }
            }
        );
    }
}
