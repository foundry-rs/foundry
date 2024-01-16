//! The coverage reports module

mod bytecode;
mod debug;
mod lcov;
mod summary;

pub use foundry_evm::coverage::*;

pub use bytecode::*;
pub use debug::*;
pub use lcov::*;
pub use summary::*;

/// A coverage reporter.
pub trait CoverageReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()>;
}
