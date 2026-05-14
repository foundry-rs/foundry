//! Shared terminal UI utilities for Foundry.

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, read},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::{
    env,
    io::{IsTerminal, Result as IoResult, Stdout, Write, stdin, stdout},
    ops::ControlFlow,
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

/// The resolved mode for a requested TUI run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiMode {
    /// The process can open an interactive TUI.
    Interactive,
    /// The process should use a line-oriented fallback.
    Fallback(TuiFallbackReason),
}

impl TuiMode {
    /// Returns whether the mode can run an interactive TUI.
    pub const fn is_interactive(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

/// Why an interactive TUI should not be opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiFallbackReason {
    /// Foundry is running in a CI environment.
    Ci,
    /// Standard input is not connected to a terminal.
    StdinNotTerminal,
    /// Standard output is not connected to a terminal.
    StdoutNotTerminal,
}

impl TuiFallbackReason {
    /// Returns a short stable description of the fallback reason.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ci => "running in CI",
            Self::StdinNotTerminal => "stdin is not a terminal",
            Self::StdoutNotTerminal => "stdout is not a terminal",
        }
    }
}

/// Runtime environment details used to decide whether a TUI can run interactively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiEnvironment {
    /// Whether standard input is connected to a terminal.
    pub stdin_is_terminal: bool,
    /// Whether standard output is connected to a terminal.
    pub stdout_is_terminal: bool,
    /// Whether Foundry appears to be running in CI.
    pub is_ci: bool,
}

impl TuiEnvironment {
    /// Creates a new environment descriptor.
    pub const fn new(stdin_is_terminal: bool, stdout_is_terminal: bool, is_ci: bool) -> Self {
        Self { stdin_is_terminal, stdout_is_terminal, is_ci }
    }

    /// Detects the current process environment.
    pub fn detect() -> Self {
        Self::new(stdin().is_terminal(), stdout().is_terminal(), env::var_os("CI").is_some())
    }

    /// Resolves the TUI mode for this environment.
    pub const fn mode(self) -> TuiMode {
        if self.is_ci {
            TuiMode::Fallback(TuiFallbackReason::Ci)
        } else if !self.stdin_is_terminal {
            TuiMode::Fallback(TuiFallbackReason::StdinNotTerminal)
        } else if !self.stdout_is_terminal {
            TuiMode::Fallback(TuiFallbackReason::StdoutNotTerminal)
        } else {
            TuiMode::Interactive
        }
    }
}

/// Detects whether a requested TUI should run interactively or fall back to line output.
pub fn tui_mode() -> TuiMode {
    TuiEnvironment::detect().mode()
}

/// An interactive terminal application.
pub trait TuiApp {
    /// The reason the application exited.
    type Exit;

    /// Draws one frame.
    fn draw(&mut self, frame: &mut Frame<'_>);

    /// Handles one terminal event.
    fn handle_event(&mut self, event: Event) -> ControlFlow<Self::Exit>;
}

/// Runs an interactive terminal application with the default Foundry terminal setup.
pub fn run_app<App: TuiApp>(app: &mut App) -> IoResult<App::Exit> {
    with_terminal(|terminal| run_app_inner(terminal, app))?
}

/// Runs an app only when the current environment supports an interactive TUI.
pub fn run_app_if_interactive<App: TuiApp>(app: &mut App) -> IoResult<Option<App::Exit>> {
    match tui_mode() {
        TuiMode::Interactive => run_app(app).map(Some),
        TuiMode::Fallback(_) => Ok(None),
    }
}

fn run_app_inner<App: TuiApp>(
    terminal: &mut CrosstermTerminal,
    app: &mut App,
) -> IoResult<App::Exit> {
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        match app.handle_event(read()?) {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(reason) => return Ok(reason),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{TuiEnvironment, TuiFallbackReason, TuiMode};

    #[test]
    fn detects_interactive_mode() {
        let env = TuiEnvironment::new(true, true, false);

        assert_eq!(env.mode(), TuiMode::Interactive);
        assert!(env.mode().is_interactive());
    }

    #[test]
    fn ci_forces_fallback() {
        let env = TuiEnvironment::new(true, true, true);

        assert_eq!(env.mode(), TuiMode::Fallback(TuiFallbackReason::Ci));
        assert!(!env.mode().is_interactive());
    }

    #[test]
    fn stdin_must_be_terminal() {
        let env = TuiEnvironment::new(false, true, false);

        assert_eq!(env.mode(), TuiMode::Fallback(TuiFallbackReason::StdinNotTerminal));
    }

    #[test]
    fn stdout_must_be_terminal() {
        let env = TuiEnvironment::new(true, false, false);

        assert_eq!(env.mode(), TuiMode::Fallback(TuiFallbackReason::StdoutNotTerminal));
    }

    #[test]
    fn ci_reason_takes_precedence() {
        let env = TuiEnvironment::new(false, false, true);

        assert_eq!(env.mode(), TuiMode::Fallback(TuiFallbackReason::Ci));
    }

    #[test]
    fn fallback_reasons_have_stable_descriptions() {
        assert_eq!(TuiFallbackReason::Ci.as_str(), "running in CI");
        assert_eq!(TuiFallbackReason::StdinNotTerminal.as_str(), "stdin is not a terminal");
        assert_eq!(TuiFallbackReason::StdoutNotTerminal.as_str(), "stdout is not a terminal");
    }
}
