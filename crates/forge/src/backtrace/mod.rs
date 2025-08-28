//! Solidity stack trace support for test failures.

use std::fmt;

mod display;
mod frame;
mod solidity;
mod source_map;
mod trace;

pub use display::BacktraceDisplay;
pub use frame::{BacktraceFrame, BacktraceFrameKind};
pub use solidity::{PcToSourceMapper, SourceLocation};
pub use source_map::PcSourceMapper;
pub use trace::extract_backtrace;

/// A Solidity stack trace for a test failure.
#[derive(Debug, Clone, Default)]
pub struct Backtrace {
    /// The frames of the backtrace, from innermost (where the revert happened) to outermost.
    pub frames: Vec<BacktraceFrame>,
}

impl Backtrace {
    /// Creates a new backtrace with the given frames.
    pub fn new(frames: Vec<BacktraceFrame>) -> Self {
        Self { frames }
    }

    /// Returns true if the backtrace is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Filters out frames that should not be displayed (e.g., internal/compiler-generated).
    pub fn filter_frames(&mut self) {
        self.frames.retain(|frame| {
            // Keep user-defined functions and test functions
            matches!(frame.kind, BacktraceFrameKind::UserFunction | BacktraceFrameKind::TestFunction)
        });
    }
}

impl fmt::Display for Backtrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        BacktraceDisplay::new(self).fmt(f)
    }
}