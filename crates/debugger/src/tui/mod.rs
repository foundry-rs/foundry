//! The TUI implementation.

use alloy_primitives::Address;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use eyre::Result;
use foundry_common::{compile::ContractSources, evm::Breakpoints};
use foundry_evm_core::{
    debug::DebugStep,
    utils::{build_pc_ic_map, CallKind, PCICMap},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use revm::primitives::SpecId;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    ops::ControlFlow,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

mod builder;
pub use builder::DebuggerBuilder;

mod context;
use context::DebuggerContext;

mod draw;

/// Debugger exit reason.
#[derive(Debug)]
pub enum ExitReason {
    /// Exit using 'q'.
    CharExit,
}

/// The TUI debugger.
pub struct Debugger {
    debug_arena: Vec<(Address, Vec<DebugStep>, CallKind)>,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    identified_contracts: HashMap<Address, String>,
    /// Source map of contract sources
    contracts_sources: ContractSources,
    /// A mapping of source -> (PC -> IC map for deploy code, PC -> IC map for runtime code)
    pc_ic_maps: BTreeMap<String, (PCICMap, PCICMap)>,
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
        debug_arena: Vec<(Address, Vec<DebugStep>, CallKind)>,
        identified_contracts: HashMap<Address, String>,
        contracts_sources: ContractSources,
        breakpoints: Breakpoints,
    ) -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        let pc_ic_maps = contracts_sources
            .0
            .iter()
            .flat_map(|(contract_name, files_sources)| {
                files_sources.iter().filter_map(|(_, (_, contract))| {
                    Some((
                        contract_name.clone(),
                        (
                            build_pc_ic_map(
                                SpecId::LATEST,
                                contract.bytecode.object.as_bytes()?.as_ref(),
                            ),
                            build_pc_ic_map(
                                SpecId::LATEST,
                                contract
                                    .deployed_bytecode
                                    .bytecode
                                    .as_ref()?
                                    .object
                                    .as_bytes()?
                                    .as_ref(),
                            ),
                        ),
                    ))
                })
            })
            .collect();
        Ok(Self {
            debug_arena,
            terminal,
            identified_contracts,
            contracts_sources,
            pc_ic_maps,
            breakpoints,
        })
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
        let mut guard = DebuggerGuard::setup(self)?;
        let r = guard.0.try_run_real();
        // Cleanup only once.
        guard.restore()?;
        std::mem::forget(guard);
        r
    }

    #[instrument(target = "debugger", name = "run", skip_all, ret)]
    fn try_run_real(&mut self) -> Result<ExitReason> {
        // Create the context.
        let mut cx = DebuggerContext::new(self);
        cx.init()?;

        // Create an event listener in a different thread.
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("event-listener".into())
            .spawn(move || Self::event_listener(tx))
            .expect("failed to spawn thread");

        loop {
            match cx.handle_event(rx.recv()?) {
                ControlFlow::Continue(()) => {}
                ControlFlow::Break(reason) => return Ok(reason),
            }
            cx.draw()?;
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

/// Handles terminal state. `restore` should be called before drop to handle errors.
#[must_use]
struct DebuggerGuard<'a>(&'a mut Debugger);

impl<'a> DebuggerGuard<'a> {
    fn setup(dbg: &'a mut Debugger) -> Result<Self> {
        let this = Self(dbg);
        enable_raw_mode()?;
        execute!(*this.0.terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
        this.0.terminal.hide_cursor()?;
        Ok(this)
    }

    fn restore(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(*self.0.terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        self.0.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for DebuggerGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
