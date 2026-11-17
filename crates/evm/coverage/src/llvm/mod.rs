//! A model of LLVM's coverage mapping and the writers for the files its tools read.
//!
//! [`FunctionInput`]s describe functions as LLVM's coverage-mapping writer takes them: regions
//! over counters and expressions, with the profile counters and MC/DC bitmap. [`CoverageMapping`]
//! loads them the way `llvm-cov` does and builds the segments, line statistics, and summaries of
//! its reports. [`export_json`] renders them as `llvm-cov export` JSON, and [`CoverageObject`]
//! writes the coverage mapping object and the text profile that `llvm-cov` and `llvm-profdata`
//! read.

mod encode;
mod export;
mod mapping;
mod segments;
mod summary;

pub use encode::{CoverageObject, md5_low};
pub use export::{EXPORT_TYPE, EXPORT_VERSION, export_json};
pub use mapping::{
    CondState, CountedRegion, Counter, CoverageMapping, Expansion, Expression, FunctionInput,
    FunctionRecord, InstantiationGroup, LineCol, MappingRegion, McdcParams, McdcRecord, RegionKind,
    TvIdxBuilder, decision_vectors,
};
pub use segments::{CoverageData, CoverageSegment, LineCoverageStats};
pub use summary::{Coverage, DataSummary, FileSummary, FunctionSummary};

#[cfg(test)]
pub(crate) mod tests;
