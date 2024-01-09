//! The TUI implementation.

use alloy_primitives::Address;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use eyre::Result;
use foundry_common::{compile::ContractSources, evm::Breakpoints};
use foundry_evm_core::{debug::DebugNodeFlat, utils::PcIcMap};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use revm::primitives::SpecId;
use std::{
    collections::{BTreeMap, HashMap},
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
    debug_arena: Vec<DebugNodeFlat>,
    identified_contracts: HashMap<Address, String>,
    /// Source map of contract sources
    contracts_sources: ContractSources,
    /// A mapping of source -> (PC -> IC map for deploy code, PC -> IC map for runtime code)
    pc_ic_maps: BTreeMap<String, (PcIcMap, PcIcMap)>,
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
        debug_arena: Vec<DebugNodeFlat>,
        identified_contracts: HashMap<Address, String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
    ) -> Self {
        let pc_ic_maps = contracts_sources
            .0
            .iter()
            .flat_map(|(contract_name, files_sources)| {
                files_sources.iter().filter_map(|(_, (_, contract))| {
                    Some((
                        contract_name.clone(),
                        (
                            PcIcMap::new(SpecId::LATEST, contract.bytecode.bytes()?),
                            PcIcMap::new(SpecId::LATEST, contract.deployed_bytecode.bytes()?),
                        ),
                    ))
                })
            })
            .collect();
        Self { debug_arena, identified_contracts, contracts_sources, pc_ic_maps, breakpoints }
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
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        TerminalGuard::with(&mut terminal, |terminal| self.try_run_real(terminal))
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

        eyre::ensure!(!cx.debug_arena().is_empty(), "debug arena is empty");

        // Draw the initial state.
        cx.draw(terminal)?;

        // Start the event loop.
        loop {
            match cx.handle_event(rx.recv()?) {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(reason) => return Ok(reason),
            }
            cx.draw(terminal)?;
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

type PanicHandler = Box<dyn Fn(&std::panic::PanicInfo<'_>) + 'static + Sync + Send>;

/// Handles terminal state.
#[must_use]
struct TerminalGuard<'a, B: Backend + io::Write> {
    terminal: &'a mut Terminal<B>,
    hook: Option<Arc<PanicHandler>>,
}

impl<'a, B: Backend + io::Write> TerminalGuard<'a, B> {
    fn with<T>(terminal: &'a mut Terminal<B>, mut f: impl FnMut(&mut Terminal<B>) -> T) -> T {
        let mut guard = Self { terminal, hook: None };
        guard.setup();
        f(guard.terminal)
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
            let _ = std::panic::take_hook();
            let prev = self.hook.take().unwrap();
            let prev = match Arc::try_unwrap(prev) {
                Ok(prev) => prev,
                Err(_) => unreachable!(),
            };
            std::panic::set_hook(prev);
        }

        Self::half_restore(self.terminal.backend_mut());
        let _ = self.terminal.show_cursor();
    }

    fn half_restore(w: &mut impl io::Write) {
        let _ = disable_raw_mode();
        let _ = execute!(*w, LeaveAlternateScreen, DisableMouseCapture);
    }
}

impl<B: Backend + io::Write> Drop for TerminalGuard<'_, B> {
    #[inline]
    fn drop(&mut self) {
        self.restore();
    }
}
