//! The debugger TUI.

use crossterm::event;
use eyre::Result;
use foundry_tui::{CrosstermTerminal, with_terminal};
use std::ops::ControlFlow;

mod context;
use crate::debugger::DebuggerContext;
use context::TUIContext;

mod draw;

type DebuggerTerminal = CrosstermTerminal;

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
        with_terminal(|terminal| self.run_inner(terminal))?
    }

    #[instrument(target = "debugger", name = "run", skip_all, ret)]
    fn run_inner(&mut self, terminal: &mut DebuggerTerminal) -> Result<ExitReason> {
        let mut cx = TUIContext::new(self.debugger_context);
        cx.init();
        loop {
            cx.draw(terminal)?;
            match cx.handle_event(event::read()?) {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(reason) => return Ok(reason),
            }
        }
    }
}
