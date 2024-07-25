//! The TUI implementation.

use alloy_primitives::Address;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use eyre::Result;
use foundry_common::evm::Breakpoints;
use foundry_evm_traces::debug::ContractSources;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use std::{
    collections::HashMap,
    io,
    ops::ControlFlow,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

mod builder;
pub use builder::DebuggerBuilder;

mod context;
use context::DebuggerContext;

use crate::DebugNode;

mod draw;

type DebuggerTerminal = Terminal<CrosstermBackend<io::Stdout>>;

/// Debugger exit reason.
#[derive(Debug)]
pub enum ExitReason {
    /// Exit using 'q'.
    CharExit,
}

/// The TUI debugger.
pub struct Debugger {
    debug_arena: Vec<DebugNode>,
    identified_contracts: HashMap<Address, String>,
    /// Source map of contract sources
    contracts_sources: ContractSources,
    breakpoints: Breakpoints,
}

impl Debugger {
    /// Creates a new debugger builder.
    #[inline]
    pub fn builder() -> DebuggerBuilder {
        DebuggerBuilder::new()
    }

    /// Creates a new debugger.
    pub fn new(
        debug_arena: Vec<DebugNode>,
        identified_contracts: HashMap<Address, String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
    ) -> Self {
        Self { debug_arena, identified_contracts, contracts_sources, breakpoints }
    }

    /// Starts the debugger TUI. Terminates the current process on failure or user exit.
    pub fn run_exit(mut self) -> ! {
        let code = match self.try_run() {
            Ok(ExitReason::CharExit) => 0,
            Err(e) => {
                println!("{e}");
                1
            }
        };
        std::process::exit(code)
    }

    /// Starts the debugger TUI.
    pub fn try_run(&mut self) -> Result<ExitReason> {
        eyre::ensure!(!self.debug_arena.is_empty(), "debug arena is empty");

        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        TerminalGuard::with(terminal, |terminal| self.try_run_real(terminal))
    }

    #[instrument(target = "debugger", name = "run", skip_all, ret)]
    fn try_run_real(&mut self, terminal: &mut DebuggerTerminal) -> Result<ExitReason> {
        // Create the context.
        let mut cx = DebuggerContext::new(self);

        cx.init();

        // Create an event listener in a different thread.
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("event-listener".into())
            .spawn(move || Self::event_listener(tx))
            .expect("failed to spawn thread");

        // Start the event loop.
        loop {
            cx.draw(terminal)?;
            match cx.handle_event(rx.recv()?) {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(reason) => return Ok(reason),
            }
        }
    }

    fn event_listener(tx: mpsc::Sender<Event>) {
        // This is the recommend tick rate from `ratatui`, based on their examples
        let tick_rate = Duration::from_millis(200);

        let mut last_tick = Instant::now();
        loop {
            // Poll events since last tick - if last tick is greater than tick_rate, we
            // demand immediate availability of the event. This may affect interactivity,
            // but I'm not sure as it is hard to test.
            if event::poll(tick_rate.saturating_sub(last_tick.elapsed())).unwrap() {
                let event = event::read().unwrap();
                if tx.send(event).is_err() {
                    return;
                }
            }

            // Force update if time has passed
            if last_tick.elapsed() > tick_rate {
                last_tick = Instant::now();
            }
        }
    }
}

// TODO: Update once on 1.82
#[allow(deprecated)]
type PanicHandler = Box<dyn Fn(&std::panic::PanicInfo<'_>) + 'static + Sync + Send>;

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
