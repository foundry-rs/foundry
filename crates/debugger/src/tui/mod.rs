//! The debugger TUI.

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use eyre::Result;
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::{io, ops::ControlFlow, sync::Arc};

mod context;
use crate::debugger::DebuggerContext;
use context::TUIContext;

mod draw;

type DebuggerTerminal = Terminal<CrosstermBackend<io::Stdout>>;

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
    pub fn new(debugger_context: &'a mut DebuggerContext) -> Self {
        Self { debugger_context }
    }

    /// Starts the debugger TUI.
    pub fn try_run(&mut self) -> Result<ExitReason> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        TerminalGuard::with(terminal, |terminal| self.run_inner(terminal))
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

type PanicHandler = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + 'static + Sync + Send>;

/// Handles terminal state.
#[must_use]
struct TerminalGuard<B: Backend + io::Write> {
    terminal: Terminal<B>,
    hook: Option<Arc<PanicHandler>>,
}

impl<B: Backend + io::Write> TerminalGuard<B> {
    fn with<T>(terminal: Terminal<B>, mut f: impl FnMut(&mut Terminal<B>) -> T) -> T {
        let mut guard = Self { terminal, hook: None };
        guard.setup();
        f(&mut guard.terminal)
    }

    fn setup(&mut self) {
        let previous = Arc::new(std::panic::take_hook());
        self.hook = Some(previous.clone());
        // We need to restore the terminal state before displaying the panic message.
        // TODO: Use `std::panic::update_hook` when it's stable
        std::panic::set_hook(Box::new(move |info| {
            Self::half_restore(&mut std::io::stdout());
            (previous)(info)
        }));

        let _ = enable_raw_mode();
        let _ = execute!(*self.terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture);
        let _ = self.terminal.hide_cursor();
        let _ = self.terminal.clear();
    }

    fn restore(&mut self) {
        if !std::thread::panicking() {
            // Drop the current hook to guarantee that `self.hook` is the only reference to it.
            let _ = std::panic::take_hook();
            // Restore the previous panic hook.
            let prev = self.hook.take().unwrap();
            let prev = match Arc::try_unwrap(prev) {
                Ok(prev) => prev,
                Err(_) => unreachable!("`self.hook` is not the only reference to the panic hook"),
            };
            std::panic::set_hook(prev);

            // NOTE: Our panic handler calls this function, so we only have to call it here if we're
            // not panicking.
            Self::half_restore(self.terminal.backend_mut());
        }

        let _ = self.terminal.show_cursor();
    }

    fn half_restore(w: &mut impl io::Write) {
        let _ = disable_raw_mode();
        let _ = execute!(*w, LeaveAlternateScreen, DisableMouseCapture);
    }
}

impl<B: Backend + io::Write> Drop for TerminalGuard<B> {
    #[inline]
    fn drop(&mut self) {
        self.restore();
    }
}
