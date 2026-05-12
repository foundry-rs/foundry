//! Shared terminal UI utilities for Foundry.

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::{
    io::{Result as IoResult, Stdout, Write, stdout},
    panic::{PanicHookInfo, set_hook, take_hook},
    sync::Arc,
    thread::panicking,
};

/// The default terminal backend used by Foundry TUIs.
pub type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

type PanicHandler = Box<dyn Fn(&PanicHookInfo<'_>) + 'static + Sync + Send>;

/// Runs a closure with the default Foundry terminal setup.
pub fn with_terminal<T>(f: impl FnMut(&mut CrosstermTerminal) -> T) -> IoResult<T> {
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(TerminalGuard::with(terminal, f))
}

/// Handles terminal setup and teardown for interactive TUIs.
#[must_use]
pub struct TerminalGuard<B: Backend + Write> {
    terminal: Terminal<B>,
    hook: Option<Arc<PanicHandler>>,
}

impl<B: Backend + Write> TerminalGuard<B> {
    /// Runs a closure while the terminal is in alternate-screen raw mode.
    pub fn with<T>(terminal: Terminal<B>, mut f: impl FnMut(&mut Terminal<B>) -> T) -> T {
        let mut guard = Self { terminal, hook: None };
        guard.setup();
        f(&mut guard.terminal)
    }

    fn setup(&mut self) {
        let previous = Arc::new(take_hook());
        self.hook = Some(previous.clone());
        // Restore terminal state before displaying the panic message.
        set_hook(Box::new(move |info| {
            Self::half_restore(&mut stdout());
            (previous)(info)
        }));

        let _ = enable_raw_mode();
        let _ = execute!(*self.terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture);
        let _ = self.terminal.hide_cursor();
        let _ = self.terminal.clear();
    }

    fn restore(&mut self) {
        if !panicking() {
            let _ = take_hook();
            let prev = self.hook.take().unwrap();
            let prev = match Arc::try_unwrap(prev) {
                Ok(prev) => prev,
                Err(_) => unreachable!("`self.hook` is not the only reference to the panic hook"),
            };
            set_hook(prev);

            Self::half_restore(self.terminal.backend_mut());
        }

        let _ = self.terminal.show_cursor();
    }

    fn half_restore(w: &mut impl Write) {
        let _ = disable_raw_mode();
        let _ = execute!(*w, LeaveAlternateScreen, DisableMouseCapture);
    }
}

impl<B: Backend + Write> Drop for TerminalGuard<B> {
    #[inline]
    fn drop(&mut self) {
        self.restore();
    }
}
