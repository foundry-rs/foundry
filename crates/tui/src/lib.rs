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
    io::{Result as IoResult, Stdout, Write, stdout},
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
