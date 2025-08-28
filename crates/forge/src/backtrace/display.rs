//! Display formatting for backtraces.

use super::Backtrace;
use std::fmt;
use yansi::Paint;

/// A display wrapper for backtraces.
pub struct BacktraceDisplay<'a> {
    backtrace: &'a Backtrace,
}

impl<'a> BacktraceDisplay<'a> {
    /// Creates a new backtrace display wrapper.
    pub fn new(backtrace: &'a Backtrace) -> Self {
        Self { backtrace }
    }
}

impl<'a> fmt::Display for BacktraceDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.backtrace.frames.is_empty() {
            return Ok(());
        }

        writeln!(f, "{}", "Stack trace:".yellow())?;

        // Print frames (innermost first)
        for (i, frame) in self.backtrace.frames.iter().enumerate() {
            // Indent based on depth
            write!(f, "  ")?;
            
            // Add arrow for first frame (where the revert happened)
            if i == 0 {
                write!(f, "→ ")?;
            } else {
                write!(f, "  ")?;
            }

            // Start with "at"
            write!(f, "at ")?;

            // Format the frame
            let formatted = frame.format();
            
            // Color based on frame type
            let colored = match frame.kind {
                super::BacktraceFrameKind::TestFunction => formatted.green(),
                super::BacktraceFrameKind::ExternalCall => formatted.cyan(),
                super::BacktraceFrameKind::LibraryFunction => formatted.blue(),
                super::BacktraceFrameKind::Fallback | 
                super::BacktraceFrameKind::Receive => formatted.yellow(),
                _ => formatted.white(),
            };

            writeln!(f, "{}", colored)?;
        }

        Ok(())
    }
}