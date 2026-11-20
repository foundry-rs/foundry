//! Coverage summaries, ported from `llvm-cov`'s `CoverageSummaryInfo`.

use super::{
    mapping::{CountedRegion, CoverageMapping, Expansion, McdcRecord, RegionKind},
    segments::CoverageData,
};

/// A count of covered items out of all items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// The number of covered items.
    pub covered: usize,
    /// The total number of items.
    pub total: usize,
}

impl Coverage {
    /// The percentage of covered items, or 0 without items.
    pub const fn percent(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.covered as f64 / self.total as f64 * 100.0 }
    }

    const fn add(&mut self, other: Self) {
        self.covered += other.covered;
        self.total += other.total;
    }

    /// Keeps the larger counts, like `llvm-cov` merges the instantiations of a function.
    fn merge(&mut self, other: Self) {
        self.covered = self.covered.max(other.covered);
        self.total = self.total.max(other.total);
    }
}

/// The region, line, branch, and MC/DC coverage of a function or file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DataSummary {
    /// The code regions.
    pub regions: Coverage,
    /// The mapped lines.
    pub lines: Coverage,
    /// The true and false sides of the branches.
    pub branches: Coverage,
    /// The conditions of the MC/DC decisions.
    pub mcdc: Coverage,
}

impl DataSummary {
    const fn add(&mut self, other: &Self) {
        self.regions.add(other.regions);
        self.lines.add(other.lines);
        self.branches.add(other.branches);
        self.mcdc.add(other.mcdc);
    }
}

/// The summary of one function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FunctionSummary {
    /// The coverage.
    pub data: DataSummary,
    /// How many times the function was entered.
    pub execution_count: u64,
}

/// The summary of one file or of all files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileSummary {
    /// The coverage summed over the file's functions.
    pub data: DataSummary,
    /// The executed functions out of all functions.
    pub functions: Coverage,
    /// The executed instantiations out of all instantiations.
    pub instantiations: Coverage,
}

impl FileSummary {
    /// Adds the summary of another file.
    pub const fn add(&mut self, other: &Self) {
        self.data.add(&other.data);
        self.functions.add(other.functions);
        self.instantiations.add(other.instantiations);
    }
}

/// Sums the sides of branch regions.
pub(super) fn sum_branches(branches: &[CountedRegion]) -> Coverage {
    let mut sum = Coverage::default();
    for branch in branches {
        if !branch.true_folded {
            sum.total += 1;
            sum.covered += usize::from(branch.count > 0);
        }
        if !branch.false_folded {
            sum.total += 1;
            sum.covered += usize::from(branch.false_count > 0);
        }
    }
    sum
}

fn sum_branch_expansions(mapping: &CoverageMapping, expansions: &[Expansion]) -> Coverage {
    let mut sum = Coverage::default();
    for &expansion in expansions {
        let data = mapping.coverage_for_expansion(expansion);
        sum.add(sum_branches(&data.branches));
        sum.add(sum_branch_expansions(mapping, &data.expansions));
    }
    sum
}

fn sum_mcdc_pairs(records: &[McdcRecord]) -> Coverage {
    let mut sum = Coverage::default();
    for record in records {
        for condition in 0..record.num_conditions() {
            if !record.is_cond_folded(condition) {
                sum.total += 1;
                sum.covered += usize::from(record.is_independence_pair_covered(condition));
            }
        }
    }
    sum
}

fn data_summary(data: &CoverageData, code_regions: &[CountedRegion]) -> DataSummary {
    let regions = code_regions.iter().filter(|r| r.region.kind == RegionKind::Code);
    let regions = Coverage {
        covered: regions.clone().filter(|r| r.count != 0).count(),
        total: regions.count(),
    };
    let mut lines = Coverage::default();
    for stats in data.line_stats().iter().filter(|stats| stats.mapped) {
        lines.total += 1;
        lines.covered += usize::from(stats.execution_count > 0);
    }
    DataSummary {
        regions,
        lines,
        branches: sum_branches(&data.branches),
        mcdc: sum_mcdc_pairs(&data.mcdc_records),
    }
}

impl FunctionSummary {
    /// The summary of the function at `index` of the mapping.
    pub fn new(mapping: &CoverageMapping, index: usize) -> Self {
        let function = &mapping.functions()[index];
        let data = mapping.coverage_for_function(index);
        let mut summary = data_summary(&data, &function.regions);
        // The branches of expansions count too.
        summary.branches.add(sum_branch_expansions(mapping, &data.expansions));
        Self { data: summary, execution_count: function.execution_count }
    }
}

impl FileSummary {
    /// The summary of `filename`: its functions grouped by definition location.
    pub fn new(mapping: &CoverageMapping, filename: &str) -> Self {
        let mut file = Self::default();
        for group in mapping.instantiation_groups(filename) {
            let summaries =
                group.functions.iter().map(|&index| FunctionSummary::new(mapping, index));
            let mut group_data = None::<DataSummary>;
            let mut group_count = 0;
            for summary in summaries {
                file.instantiations.covered += usize::from(summary.execution_count > 0);
                file.instantiations.total += 1;
                group_count += summary.execution_count;
                match &mut group_data {
                    None => group_data = Some(summary.data),
                    Some(data) => {
                        data.regions.merge(summary.data.regions);
                        data.lines.merge(summary.data.lines);
                        data.branches.merge(summary.data.branches);
                        data.mcdc.merge(summary.data.mcdc);
                    }
                }
            }
            let Some(data) = group_data else { continue };
            file.data.add(&data);
            file.functions.covered += usize::from(group_count > 0);
            file.functions.total += 1;
        }
        file
    }
}
