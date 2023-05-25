mod debug;
mod html;
mod lcov;
mod summary;

pub use debug::*;
pub use foundry_evm::coverage::*;
pub use html::*;
pub use lcov::*;
pub use summary::*;

/// A coverage reporter.
pub trait CoverageReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()>;
}
