//! The debugger TUI.

use eyre::Result;
use foundry_tui::{TuiFallbackReason, TuiMode, run_app_if_interactive, tui_mode};

mod context;
use crate::debugger::DebuggerContext;
use context::TUIContext;

mod draw;
mod storage;

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
                         interactive terminal.",
                    ),
                };
                eyre::bail!("{message} {}", debugger_dump_hint());
            }
        }
    }
}

fn non_interactive_debugger_message(reason: TuiFallbackReason) -> String {
    format!(
        "Cannot open the debugger TUI because {}. Re-run in an interactive terminal.",
        reason.as_str()
    )
}

const fn debugger_dump_hint() -> &'static str {
    "Pass `--dump <PATH>` to export debugger steps."
}

#[cfg(test)]
mod tests {
    use super::{TuiFallbackReason, debugger_dump_hint, non_interactive_debugger_message};
    use crate::{DebugNode, Debugger};
    use std::{env, ffi::OsString};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = env::var_os(key);
            unsafe { env::set_var(key, value) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn fallback_message_includes_reason() {
        let msg = non_interactive_debugger_message(TuiFallbackReason::Ci);
        assert!(msg.contains("running in CI"));
        assert!(!msg.contains("--dump <PATH>"));
    }

    #[test]
    fn dump_hint_includes_dump_flag() {
        assert!(debugger_dump_hint().contains("--dump <PATH>"));
    }

    #[test]
    fn debugger_tui_falls_back_in_ci_with_dump_hint() {
        let _ci = EnvVarGuard::set("CI", "1");
        let mut debugger = Debugger::new(
            vec![DebugNode::default()],
            Default::default(),
            Default::default(),
            Default::default(),
        );

        let message = debugger.try_run_tui().unwrap_err().to_string();

        assert!(message.contains("running in CI"));
        assert!(message.contains("--dump <PATH>"));
    }
}
