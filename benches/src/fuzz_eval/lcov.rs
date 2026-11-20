//! LCOV tracefile parsing.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Aggregated line, branch, and function counters for a set of source files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageStats {
    /// Number of source files included.
    pub files: usize,
    /// Instrumented lines (`LF`).
    pub lines_found: u64,
    /// Lines hit at least once (`LH`).
    pub lines_hit: u64,
    /// Instrumented branches (`BRF`).
    pub branches_found: u64,
    /// Branches taken at least once (`BRH`).
    pub branches_hit: u64,
    /// Instrumented functions (`FNF`).
    pub functions_found: u64,
    /// Functions hit at least once (`FNH`).
    pub functions_hit: u64,
}

impl CoverageStats {
    /// Line coverage in percent, or `None` if no lines are instrumented.
    pub fn line_pct(&self) -> Option<f64> {
        pct(self.lines_hit, self.lines_found)
    }

    /// Branch coverage in percent, or `None` if no branches are instrumented.
    pub fn branch_pct(&self) -> Option<f64> {
        pct(self.branches_hit, self.branches_found)
    }

    /// Function coverage in percent, or `None` if no functions are instrumented.
    pub fn function_pct(&self) -> Option<f64> {
        pct(self.functions_hit, self.functions_found)
    }

    fn add(&mut self, file: &FileRecord) {
        self.files += 1;
        self.lines_found += file.lines_found();
        self.lines_hit += file.lines_hit();
        self.branches_found += file.branches_found();
        self.branches_hit += file.branches_hit();
        self.functions_found += file.functions_found();
        self.functions_hit += file.functions_hit();
    }
}

/// Counters for one `SF:` record. Summary lines (`LF`, `BRF`, ...) take precedence; otherwise the
/// totals are derived from the detail lines (`DA`, `BRDA`, `FNDA`).
#[derive(Debug, Default)]
struct FileRecord {
    path: String,
    lf: Option<u64>,
    lh: Option<u64>,
    brf: Option<u64>,
    brh: Option<u64>,
    fnf: Option<u64>,
    fnh: Option<u64>,
    da_found: u64,
    da_hit: u64,
    brda_found: u64,
    brda_hit: u64,
    fnda_found: u64,
    fnda_hit: u64,
}

impl FileRecord {
    fn lines_found(&self) -> u64 {
        self.lf.unwrap_or(self.da_found)
    }

    fn lines_hit(&self) -> u64 {
        self.lh.unwrap_or(self.da_hit)
    }

    fn branches_found(&self) -> u64 {
        self.brf.unwrap_or(self.brda_found)
    }

    fn branches_hit(&self) -> u64 {
        self.brh.unwrap_or(self.brda_hit)
    }

    fn functions_found(&self) -> u64 {
        self.fnf.unwrap_or(self.fnda_found)
    }

    fn functions_hit(&self) -> u64 {
        self.fnh.unwrap_or(self.fnda_hit)
    }
}

/// Parses an LCOV tracefile and aggregates the records whose source path starts with one of
/// `include` (after stripping `root` from absolute paths). An empty `include` keeps every file.
///
/// Malformed lines are ignored rather than rejected so that a partially written report still
/// yields the records that are complete.
pub fn parse_lcov(content: &str, root: &Path, include: &[String]) -> CoverageStats {
    let mut stats = CoverageStats::default();
    let mut current = None::<FileRecord>;
    for line in content.lines().map(str::trim) {
        if line == "end_of_record" {
            if let Some(file) = current.take()
                && is_included(&file.path, root, include)
            {
                stats.add(&file);
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        if key == "SF" {
            current = Some(FileRecord { path: value.to_string(), ..Default::default() });
            continue;
        }
        let Some(file) = current.as_mut() else { continue };
        match key {
            "LF" => file.lf = value.parse().ok(),
            "LH" => file.lh = value.parse().ok(),
            "BRF" => file.brf = value.parse().ok(),
            "BRH" => file.brh = value.parse().ok(),
            "FNF" => file.fnf = value.parse().ok(),
            "FNH" => file.fnh = value.parse().ok(),
            "DA" => {
                if let Some(count) = value.split(',').nth(1) {
                    file.da_found += 1;
                    file.da_hit += u64::from(is_hit(count));
                }
            }
            "BRDA" => {
                if let Some(count) = value.split(',').nth(3) {
                    file.brda_found += 1;
                    file.brda_hit += u64::from(is_hit(count));
                }
            }
            "FNDA" => {
                if let Some((count, _)) = value.split_once(',') {
                    file.fnda_found += 1;
                    file.fnda_hit += u64::from(is_hit(count));
                }
            }
            _ => {}
        }
    }
    stats
}

fn is_hit(count: &str) -> bool {
    count.trim().parse::<u64>().is_ok_and(|count| count > 0)
}

fn is_included(path: &str, root: &Path, include: &[String]) -> bool {
    if include.is_empty() {
        return true;
    }
    let path = Path::new(path);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let relative = relative.to_string_lossy().replace('\\', "/");
    let relative = relative.trim_start_matches("./");
    include.iter().any(|prefix| relative.starts_with(prefix.trim_start_matches("./")))
}

fn pct(hit: u64, found: u64) -> Option<f64> {
    (found > 0).then(|| hit as f64 * 100.0 / found as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "\
TN:
SF:src/Vault.sol
DA:10,4
DA:11,0
FN:10,Vault.deposit
FNDA:4,Vault.deposit
FNF:2
FNH:1
BRDA:11,0,0,4
BRDA:11,0,1,-
BRF:2
BRH:1
LF:2
LH:1
end_of_record
TN:
SF:test/Vault.t.sol
DA:5,9
LF:1
LH:1
end_of_record
TN:
SF:/work/project/src/Escrow.sol
DA:3,1
DA:4,2
DA:5,0
FNDA:0,Escrow.claim
FNDA:3,Escrow.fund
BRDA:4,0,0,1
BRDA:4,0,1,0
BRDA:5,1,0,-
end_of_record
";

    #[test]
    fn aggregates_only_included_sources() {
        let stats = parse_lcov(REPORT, Path::new("/work/project"), &["src/".to_string()]);
        assert_eq!(
            stats,
            CoverageStats {
                files: 2,
                lines_found: 5,
                lines_hit: 3,
                branches_found: 5,
                branches_hit: 2,
                functions_found: 4,
                functions_hit: 2,
            }
        );
        assert_eq!(stats.line_pct(), Some(60.0));
        assert_eq!(stats.branch_pct(), Some(40.0));
        assert_eq!(stats.function_pct(), Some(50.0));
    }

    #[test]
    fn filters_by_file_prefix() {
        let stats = parse_lcov(REPORT, Path::new("/work/project"), &["src/Escrow.sol".to_string()]);
        assert_eq!(stats.files, 1);
        assert_eq!((stats.lines_found, stats.lines_hit), (3, 2));
        assert_eq!((stats.branches_found, stats.branches_hit), (3, 1));
        assert_eq!((stats.functions_found, stats.functions_hit), (2, 1));
    }

    #[test]
    fn empty_include_keeps_every_file() {
        let stats = parse_lcov(REPORT, Path::new("/work/project"), &[]);
        assert_eq!(stats.files, 3);
        assert_eq!(stats.lines_found, 6);
    }

    #[test]
    fn ignores_malformed_and_unterminated_records() {
        let report =
            "SF:src/A.sol\nDA:garbage\nLF:x\nDA:1,1\nend_of_record\nSF:src/B.sol\nDA:1,1\n";
        let stats = parse_lcov(report, Path::new("/"), &["src/".to_string()]);
        assert_eq!(stats.files, 1);
        assert_eq!((stats.lines_found, stats.lines_hit), (1, 1));
    }

    #[test]
    fn no_instrumented_items_has_no_percentage() {
        assert_eq!(CoverageStats::default().branch_pct(), None);
    }
}
