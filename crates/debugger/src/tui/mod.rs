//! The debugger TUI.

use eyre::Result;
use foundry_tui::{TuiFallbackReason, TuiMode, run_app_if_interactive, tui_mode};

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
        match run_app_if_interactive(&mut cx)? {
            Some(exit_reason) => Ok(exit_reason),
            None => {
                let message = match tui_mode() {
                    TuiMode::Fallback(reason) => non_interactive_debugger_message(reason),
                    TuiMode::Interactive => String::from(
                        "Cannot open the debugger TUI in this environment. Re-run in an \
                         interactive terminal, or pass `--dump <PATH>` to export debugger steps.",
                    ),
                };
                eyre::bail!("{message}");
            }
        }
    }
}

fn non_interactive_debugger_message(reason: TuiFallbackReason) -> String {
    format!(
        "Cannot open the debugger TUI because {}. Re-run in an interactive terminal, or pass \
         `--dump <PATH>` to export debugger steps.",
        reason.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::{TuiFallbackReason, non_interactive_debugger_message};

    #[test]
    fn fallback_message_includes_reason_and_dump_hint() {
        let msg = non_interactive_debugger_message(TuiFallbackReason::Ci);
        assert!(msg.contains("running in CI"));
        assert!(msg.contains("--dump <PATH>"));
    }
}
