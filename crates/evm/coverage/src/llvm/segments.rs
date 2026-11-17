//! Coverage segments and per-line statistics, ported from LLVM's `SegmentBuilder` and
//! `LineCoverageStats`.

use super::mapping::{CountedRegion, Expansion, LineCol, McdcRecord, RegionKind};

/// The execution count at a point in a file, like LLVM's `CoverageSegment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverageSegment {
    /// The line where the segment begins.
    pub line: u32,
    /// The column where the segment begins.
    pub col: u32,
    /// The execution count, or zero if no count was recorded.
    pub count: u64,
    /// Whether the segment has a count. It has none when it was uninstrumented or skipped.
    pub has_count: bool,
    /// Whether the segment enters a new region or returns to a previous count.
    pub is_region_entry: bool,
    /// Whether the segment enters a gap region.
    pub is_gap_region: bool,
}

impl CoverageSegment {
    const fn skipped(line: u32, col: u32, is_region_entry: bool) -> Self {
        Self { line, col, count: 0, has_count: false, is_region_entry, is_gap_region: false }
    }

    const fn counted(line: u32, col: u32, count: u64, is_region_entry: bool, is_gap: bool) -> Self {
        Self { line, col, count, has_count: true, is_region_entry, is_gap_region: is_gap }
    }
}

/// The coverage of a file, a function, or an expansion: sorted segments, with the expansions,
/// branches, and MC/DC records inside it.
#[derive(Clone, Debug, Default)]
pub struct CoverageData {
    /// The name of the file.
    pub filename: String,
    /// The segments, sorted by location.
    pub segments: Vec<CoverageSegment>,
    /// The expansions that can be processed further.
    pub expansions: Vec<Expansion>,
    /// The branch regions.
    pub branches: Vec<CountedRegion>,
    /// The MC/DC records.
    pub mcdc_records: Vec<McdcRecord>,
}

impl CoverageData {
    pub(super) fn new(filename: &str) -> Self {
        Self { filename: filename.to_string(), ..Default::default() }
    }

    /// The statistics of every line from the first line with a segment on.
    pub fn line_stats(&self) -> Vec<LineCoverageStats> {
        match self.segments.first() {
            Some(first) => line_stats_from(self, first.line),
            None => Vec::new(),
        }
    }

    /// The statistics of every line from `line`.
    pub fn line_stats_from(&self, line: u32) -> Vec<LineCoverageStats> {
        if self.segments.is_empty() { Vec::new() } else { line_stats_from(self, line) }
    }
}

/// The coverage statistics of one line, like LLVM's `LineCoverageStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineCoverageStats {
    /// The line.
    pub line: u32,
    /// The execution count of the line.
    pub execution_count: u64,
    /// Whether more than one region starts on the line.
    pub has_multiple_regions: bool,
    /// Whether the line has any code mapped to it.
    pub mapped: bool,
}

fn line_stats_from(data: &CoverageData, start_line: u32) -> Vec<LineCoverageStats> {
    let segments = &data.segments;
    let mut stats = Vec::new();
    let mut next = 0;
    let mut line = start_line;
    let mut wrapped: Option<&CoverageSegment> = None;
    let mut line_segments = Vec::<&CoverageSegment>::new();
    // Every step yields the statistics of `line`, until the segments run out.
    while next < segments.len() {
        if let Some(last) = line_segments.last() {
            wrapped = Some(last);
        }
        line_segments.clear();
        while next < segments.len() && segments[next].line == line {
            line_segments.push(&segments[next]);
            next += 1;
        }
        stats.push(line_stats(&line_segments, wrapped, line));
        line += 1;
    }
    stats
}

fn line_stats(
    segments: &[&CoverageSegment],
    wrapped: Option<&CoverageSegment>,
    line: u32,
) -> LineCoverageStats {
    // Count the regions that start in this line, up to 2.
    let is_start_of_region =
        |s: &&CoverageSegment| !s.is_gap_region && s.has_count && s.is_region_entry;
    let min_region_count = segments.iter().filter(|s| is_start_of_region(s)).take(2).count();
    let start_of_skipped_region =
        segments.first().is_some_and(|s| !s.has_count && s.is_region_entry);

    let has_multiple_regions = min_region_count > 1;
    let mut mapped =
        !start_of_skipped_region && (wrapped.is_some_and(|w| w.has_count) || min_region_count > 0);
    // A segment that starts a region with a counter maps the line.
    mapped |= segments.iter().any(|s| s.is_region_entry && s.has_count);

    let mut stats = LineCoverageStats { line, execution_count: 0, has_multiple_regions, mapped };
    if !mapped {
        return stats;
    }
    // The count is the largest of the wrapped count and the non-gap region entries.
    if let Some(wrapped) = wrapped {
        stats.execution_count = wrapped.count;
    }
    if min_region_count == 0 {
        return stats;
    }
    for segment in segments {
        if is_start_of_region(segment) {
            stats.execution_count = stats.execution_count.max(segment.count);
        }
    }
    stats
}

/// Builds the sorted segments of a file's regions, like LLVM's `SegmentBuilder`.
pub(super) fn build_segments(regions: &mut [CountedRegion]) -> Vec<CoverageSegment> {
    let mut builder = SegmentBuilder::default();
    sort_nested_regions(regions);
    let combined = combine_regions(regions);
    builder.build(&combined);
    builder.segments
}

/// Sorts regions of one file so that a region comes before the regions it contains.
fn sort_nested_regions(regions: &mut [CountedRegion]) {
    regions.sort_by(|lhs, rhs| {
        lhs.start()
            .cmp(&rhs.start())
            // A region that contains another sorts first.
            .then_with(|| rhs.end().cmp(&lhs.end()))
            // Regions over the same area sort by kind, so that the most suitable region becomes
            // active when they are combined.
            .then_with(|| lhs.region.kind.cmp(&rhs.region.kind))
    });
}

/// Combines the counts of regions that cover the same area.
fn combine_regions(regions: &[CountedRegion]) -> Vec<CountedRegion> {
    let mut combined: Vec<CountedRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        match combined.last_mut() {
            Some(active) if active.start() == region.start() && active.end() == region.end() => {
                // Only regions of the active region's kind add up, or an area that code and an
                // expansion both cover would count twice.
                if region.region.kind == active.region.kind {
                    active.count = active.count.wrapping_add(region.count);
                }
            }
            _ => combined.push(*region),
        }
    }
    combined
}

#[derive(Default)]
struct SegmentBuilder {
    segments: Vec<CoverageSegment>,
    active: Vec<CountedRegion>,
}

impl SegmentBuilder {
    /// Emits a segment with the count of `region` starting at `start`.
    fn start_segment(
        &mut self,
        region: &CountedRegion,
        start: LineCol,
        is_region_entry: bool,
        emit_skipped: bool,
    ) {
        let has_count = !emit_skipped && region.region.kind != RegionKind::Skipped;

        // A segment that would not change how coverage renders is not needed.
        if let Some(last) = self.segments.last()
            && !is_region_entry
            && !emit_skipped
            && last.has_count == has_count
            && last.count == region.count
            && !last.is_region_entry
        {
            return;
        }

        self.segments.push(if has_count {
            CoverageSegment::counted(
                start.0,
                start.1,
                region.count,
                is_region_entry,
                region.region.kind == RegionKind::Gap,
            )
        } else {
            CoverageSegment::skipped(start.0, start.1, is_region_entry)
        });
    }

    /// Emits segments for the active regions from `first_completed` that end before `loc`.
    fn complete_regions_until(&mut self, loc: Option<LineCol>, first_completed: usize) {
        // Sorting the completed regions by end location makes the closing segments sorted.
        self.active[first_completed..].sort_by_key(CountedRegion::end);

        let len = self.active.len();
        for i in first_completed + 1..len {
            let mut completed = self.active[i];
            let previous = self.active[i - 1];
            let completed_segment_loc = previous.end();

            // Stop when the segments would start where the new region begins.
            if Some(completed_segment_loc) == loc {
                break;
            }
            // Skip a segment when the next completed region ends at the same location.
            if completed_segment_loc == completed.end() {
                continue;
            }
            // Use the count of the last completed region that ends at this location.
            for later in &self.active[i + 1..len] {
                if completed.end() == later.end() {
                    completed = *later;
                }
            }
            self.start_segment(&completed, completed_segment_loc, false, false);
        }

        let last = self.active[len - 1];
        if first_completed > 0 && Some(last.end()) != loc {
            // A gap between the last completed region and the new region takes the count of the
            // last active region.
            let filler = self.active[first_completed - 1];
            self.start_segment(&filler, last.end(), false, false);
        } else if first_completed == 0 && loc != Some(last.end()) {
            // Without active regions, a skipped segment marks the gap between functions.
            self.start_segment(&last, last.end(), false, true);
        }

        self.active.truncate(first_completed);
    }

    fn build(&mut self, regions: &[CountedRegion]) {
        for (index, region) in regions.iter().enumerate() {
            let cur_start = region.start();

            // Active regions that end before the current region start completed.
            let mut still_active = Vec::with_capacity(self.active.len());
            let mut completed = Vec::new();
            for active in &self.active {
                if active.end() <= cur_start {
                    completed.push(*active);
                } else {
                    still_active.push(*active);
                }
            }
            if !completed.is_empty() {
                let first_completed = still_active.len();
                still_active.extend(completed);
                self.active = still_active;
                self.complete_regions_until(Some(cur_start), first_completed);
            }

            let gap = region.region.kind == RegionKind::Gap;

            // Try to emit a segment for the current region.
            if cur_start == region.end() {
                // Zero-length regions do not become active. The last region emits a skipped
                // segment, and the others use the count of the region around them.
                let skipped =
                    index + 1 == regions.len() || region.region.kind == RegionKind::Skipped;
                let source = *self.active.last().unwrap_or(region);
                self.start_segment(&source, cur_start, !gap, skipped);
                if skipped && let Some(&last) = self.active.last() {
                    self.start_segment(&last, cur_start, false, false);
                }
                continue;
            }
            if index + 1 == regions.len() || cur_start != regions[index + 1].start() {
                // Emit a segment if the next region does not start where this one does.
                self.start_segment(region, cur_start, !gap, false);
            }

            // The region is active.
            self.active.push(*region);
        }

        // Complete the remaining active regions.
        if !self.active.is_empty() {
            self.complete_regions_until(None, 0);
        }
    }
}
