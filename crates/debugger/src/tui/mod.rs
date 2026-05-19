//! The debugger TUI.

use eyre::Result;
use foundry_tui::run_app;

mod context;
use crate::debugger::DebuggerContext;
use context::TUIContext;

mod draw;

/// Debugger exit reason.
#[derive(Debug)]
pub enum ExitReason {
    /// Exit using 'q'.
    CharExit,
}

/// The debugger TUI.
pub struct TUI<'a> {
    debugger_context: &'a mut DebuggerContext,
}

impl<'a> TUI<'a> {
    /// Creates a new debugger.
    pub const fn new(debugger_context: &'a mut DebuggerContext) -> Self {
        Self { debugger_context }
    }

    /// Starts the debugger TUI.
    pub fn try_run(&mut self) -> Result<ExitReason> {
        self.run_inner()
    }

    #[instrument(target = "debugger", name = "run", skip_all, ret)]
    fn run_inner(&mut self) -> Result<ExitReason> {
        let mut cx = TUIContext::new(self.debugger_context);
        cx.init();
        Ok(run_app(&mut cx)?)
    }
}
