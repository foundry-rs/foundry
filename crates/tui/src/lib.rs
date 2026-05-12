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
use std::{io, sync::Arc};

/// The default terminal backend used by Foundry TUIs.
pub type CrosstermTerminal = Terminal<CrosstermBackend<io::Stdout>>;

type PanicHandler = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + 'static + Sync + Send>;

/// Runs a closure with the default Foundry terminal setup.
pub fn with_terminal<T>(f: impl FnMut(&mut CrosstermTerminal) -> T) -> io::Result<T> {
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(TerminalGuard::with(terminal, f))
}

/// Handles terminal setup and teardown for interactive TUIs.
#[must_use]
pub struct TerminalGuard<B: Backend + io::Write> {
    terminal: Terminal<B>,
    hook: Option<Arc<PanicHandler>>,
}

impl<B: Backend + io::Write> TerminalGuard<B> {
    /// Runs a closure while the terminal is in alternate-screen raw mode.
    pub fn with<T>(terminal: Terminal<B>, mut f: impl FnMut(&mut Terminal<B>) -> T) -> T {
        let mut guard = Self { terminal, hook: None };
        guard.setup();
        f(&mut guard.terminal)
    }

    fn setup(&mut self) {
        let previous = Arc::new(std::panic::take_hook());
        self.hook = Some(previous.clone());
        // Restore terminal state before displaying the panic message.
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
            let _ = std::panic::take_hook();
            let prev = self.hook.take().unwrap();
            let prev = match Arc::try_unwrap(prev) {
                Ok(prev) => prev,
                Err(_) => unreachable!("`self.hook` is not the only reference to the panic hook"),
            };
            std::panic::set_hook(prev);

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
