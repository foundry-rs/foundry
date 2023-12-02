//! The TUI implementation.

use crate::{op::OpcodeParam, DebuggerBuilder};
use alloy_primitives::{Address, U256};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use eyre::Result;
use foundry_common::{compile::ContractSources, evm::Breakpoints};
use foundry_evm_core::{
    debug::{DebugStep, Instruction},
    utils::{build_pc_ic_map, CallKind, PCICMap},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use revm::{interpreter::opcode, primitives::SpecId};
use std::{
    cmp::{max, min},
    collections::{BTreeMap, HashMap, VecDeque},
    io,
    ops::ControlFlow,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// Used to indicate why the debugger quit.
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

/// This is currently used to remember last scroll position so screen doesn't wiggle as much.
#[derive(Default)]
struct DrawMemory {
    current_startline: usize,
    inner_call_index: usize,
    current_mem_startline: usize,
    current_stack_startline: usize,
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

struct DebuggerContext<'a> {
    debugger: &'a mut Debugger,

    /// Buffer for keys prior to execution, i.e. '10' + 'k' => move up 10 operations.
    key_buffer: String,
    /// Current step in the debug steps.
    current_step: usize,
    draw_memory: DrawMemory,
    opcode_list: Vec<String>,
    last_index: usize,

    stack_labels: bool,
    mem_utf: bool,
    show_shortcuts: bool,
}

impl<'a> DebuggerContext<'a> {
    fn new(debugger: &'a mut Debugger) -> Self {
        DebuggerContext {
            debugger,

            key_buffer: String::with_capacity(64),
            current_step: 0,
            draw_memory: DrawMemory::default(),
            opcode_list: Vec::new(),
            last_index: 0,

            stack_labels: false,
            mem_utf: false,
            show_shortcuts: true,
        }
    }

    fn init(&mut self) -> Result<()> {
        self.debugger.terminal.clear()?;
        self.gen_opcode_list();
        Ok(())
    }

    fn debug_arena(&self) -> &[(Address, Vec<DebugStep>, CallKind)] {
        &self.debugger.debug_arena
    }

    fn gen_opcode_list(&mut self) {
        self.opcode_list = self.opcode_list();
    }

    fn opcode_list(&self) -> Vec<String> {
        self.debugger.debug_arena[self.draw_memory.inner_call_index]
            .1
            .iter()
            .map(DebugStep::pretty_opcode)
            .collect()
    }
}

impl DebuggerContext<'_> {
    fn handle_event(&mut self, event: Event) -> ControlFlow<ExitReason> {
        if self.last_index != self.draw_memory.inner_call_index {
            self.opcode_list = self.debug_arena()[self.draw_memory.inner_call_index]
                .1
                .iter()
                .map(|step| step.pretty_opcode())
                .collect();
            self.last_index = self.draw_memory.inner_call_index;
        }

        match event {
            Event::Key(event) => self.handle_key_event(event),
            Event::Mouse(event) => self.handle_mouse_event(event),
            _ => ControlFlow::Continue(()),
        }
    }

    fn handle_key_event(&mut self, event: KeyEvent) -> ControlFlow<ExitReason> {
        if let KeyCode::Char(c) = event.code {
            if c.is_alphanumeric() || c == '\'' {
                self.handle_breakpoint(c);
                return ControlFlow::Continue(());
            }
        }

        match event.code {
            // Exit
            KeyCode::Char('q') => return ControlFlow::Break(ExitReason::CharExit),
            // Move down
            KeyCode::Char('j') | KeyCode::Down => {
                // Grab number of times to do it
                for _ in 0..Debugger::buffer_as_number(&self.key_buffer, 1) {
                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                        let max_mem = (self.debug_arena()[self.draw_memory.inner_call_index].1
                            [self.current_step]
                            .memory
                            .len() /
                            32)
                        .saturating_sub(1);
                        if self.draw_memory.current_mem_startline < max_mem {
                            self.draw_memory.current_mem_startline += 1;
                        }
                    } else if self.current_step < self.opcode_list.len() - 1 {
                        self.current_step += 1;
                    } else if self.draw_memory.inner_call_index < self.debug_arena().len() - 1 {
                        self.draw_memory.inner_call_index += 1;
                        self.current_step = 0;
                    }
                }
                self.key_buffer.clear();
            }
            KeyCode::Char('J') => {
                for _ in 0..Debugger::buffer_as_number(&self.key_buffer, 1) {
                    let max_stack = self.debug_arena()[self.draw_memory.inner_call_index].1
                        [self.current_step]
                        .stack
                        .len()
                        .saturating_sub(1);
                    if self.draw_memory.current_stack_startline < max_stack {
                        self.draw_memory.current_stack_startline += 1;
                    }
                }
                self.key_buffer.clear();
            }
            // Move up
            KeyCode::Char('k') | KeyCode::Up => {
                for _ in 0..Debugger::buffer_as_number(&self.key_buffer, 1) {
                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                        self.draw_memory.current_mem_startline =
                            self.draw_memory.current_mem_startline.saturating_sub(1);
                    } else if self.current_step > 0 {
                        self.current_step -= 1;
                    } else if self.draw_memory.inner_call_index > 0 {
                        self.draw_memory.inner_call_index -= 1;
                        self.current_step =
                            self.debug_arena()[self.draw_memory.inner_call_index].1.len() - 1;
                    }
                }
                self.key_buffer.clear();
            }
            KeyCode::Char('K') => {
                for _ in 0..Debugger::buffer_as_number(&self.key_buffer, 1) {
                    self.draw_memory.current_stack_startline =
                        self.draw_memory.current_stack_startline.saturating_sub(1);
                }
                self.key_buffer.clear();
            }
            // Go to top of file
            KeyCode::Char('g') => {
                self.draw_memory.inner_call_index = 0;
                self.current_step = 0;
                self.key_buffer.clear();
            }
            // Go to bottom of file
            KeyCode::Char('G') => {
                self.draw_memory.inner_call_index = self.debug_arena().len() - 1;
                self.current_step =
                    self.debug_arena()[self.draw_memory.inner_call_index].1.len() - 1;
                self.key_buffer.clear();
            }
            // Go to previous call
            KeyCode::Char('c') => {
                self.draw_memory.inner_call_index =
                    self.draw_memory.inner_call_index.saturating_sub(1);
                self.current_step =
                    self.debug_arena()[self.draw_memory.inner_call_index].1.len() - 1;
                self.key_buffer.clear();
            }
            // Go to next call
            KeyCode::Char('C') => {
                if self.debug_arena().len() > self.draw_memory.inner_call_index + 1 {
                    self.draw_memory.inner_call_index += 1;
                    self.current_step = 0;
                }
                self.key_buffer.clear();
            }
            // Step forward
            KeyCode::Char('s') => {
                for _ in 0..Debugger::buffer_as_number(&self.key_buffer, 1) {
                    let remaining_ops = self.opcode_list[self.current_step..].to_vec();
                    self.current_step += remaining_ops
                        .iter()
                        .enumerate()
                        .find_map(|(i, op)| {
                            if i < remaining_ops.len() - 1 {
                                match (
                                    op.contains("JUMP") && op != "JUMPDEST",
                                    &*remaining_ops[i + 1],
                                ) {
                                    (true, "JUMPDEST") => Some(i + 1),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        })
                        .unwrap_or(self.opcode_list.len() - 1);
                    if self.current_step > self.opcode_list.len() {
                        self.current_step = self.opcode_list.len() - 1
                    };
                }
                self.key_buffer.clear();
            }
            // Step backwards
            KeyCode::Char('a') => {
                for _ in 0..Debugger::buffer_as_number(&self.key_buffer, 1) {
                    let prev_ops = self.opcode_list[..self.current_step].to_vec();
                    self.current_step = prev_ops
                        .iter()
                        .enumerate()
                        .rev()
                        .find_map(|(i, op)| {
                            if i > 0 {
                                match (
                                    prev_ops[i - 1].contains("JUMP") &&
                                        prev_ops[i - 1] != "JUMPDEST",
                                    &**op,
                                ) {
                                    (true, "JUMPDEST") => Some(i - 1),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                }
                self.key_buffer.clear();
            }
            // toggle stack labels
            KeyCode::Char('t') => self.stack_labels = !self.stack_labels,
            // toggle memory utf8 decoding
            KeyCode::Char('m') => self.mem_utf = !self.mem_utf,
            // toggle help notice
            KeyCode::Char('h') => self.show_shortcuts = !self.show_shortcuts,
            KeyCode::Char(other) => match other {
                '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '\'' => {
                    self.key_buffer.push(other);
                }
                // Invalid key, clear buffer
                _ => self.key_buffer.clear(),
            },
            _ => self.key_buffer.clear(),
        };

        ControlFlow::Continue(())
    }

    fn handle_breakpoint(&mut self, c: char) {
        // Find the location of the called breakpoint in the whole debug arena (at this address with
        // this pc)
        if let Some((caller, pc)) = self.debugger.breakpoints.get(&c) {
            for (i, (_caller, debug_steps, _)) in self.debug_arena().iter().enumerate() {
                if _caller == caller {
                    if let Some(step) = debug_steps.iter().position(|step| step.pc == *pc) {
                        self.draw_memory.inner_call_index = i;
                        self.current_step = step;
                        break
                    }
                }
            }
        }
        self.key_buffer.clear();
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> ControlFlow<ExitReason> {
        match event.kind {
            MouseEventKind::ScrollUp => {
                if self.current_step > 0 {
                    self.current_step -= 1;
                } else if self.draw_memory.inner_call_index > 0 {
                    self.draw_memory.inner_call_index -= 1;
                    self.draw_memory.current_mem_startline = 0;
                    self.draw_memory.current_stack_startline = 0;
                    self.current_step =
                        self.debug_arena()[self.draw_memory.inner_call_index].1.len() - 1;
                }
            }
            MouseEventKind::ScrollDown => {
                if self.current_step < self.opcode_list.len() - 1 {
                    self.current_step += 1;
                } else if self.draw_memory.inner_call_index < self.debug_arena().len() - 1 {
                    self.draw_memory.inner_call_index += 1;
                    self.draw_memory.current_mem_startline = 0;
                    self.draw_memory.current_stack_startline = 0;
                    self.current_step = 0;
                }
            }
            _ => {}
        }

        ControlFlow::Continue(())
    }

    fn draw(&mut self) -> Result<()> {
        self.debugger.terminal.draw(|f| {
            let debug_arena = &self.debugger.debug_arena;
            Debugger::draw_layout(
                f,
                debug_arena[self.draw_memory.inner_call_index].0,
                &self.debugger.identified_contracts,
                &self.debugger.pc_ic_maps,
                &self.debugger.contracts_sources,
                &debug_arena[self.draw_memory.inner_call_index].1[..],
                &self.opcode_list,
                self.current_step,
                debug_arena[self.draw_memory.inner_call_index].2,
                &mut self.draw_memory,
                self.stack_labels,
                self.mem_utf,
                self.show_shortcuts,
            )
        })?;
        Ok(())
    }
}

impl Debugger {
    /// Grab number from buffer. Used for something like '10k' to move up 10 operations
    fn buffer_as_number(s: &str, default_value: usize) -> usize {
        match s.parse() {
            Ok(num) if num >= 1 => num,
            _ => default_value,
        }
    }

    /// Create layout and subcomponents
    #[allow(clippy::too_many_arguments)]
    fn draw_layout(
        f: &mut Frame<'_>,
        address: Address,
        identified_contracts: &HashMap<Address, String>,
        pc_ic_maps: &BTreeMap<String, (PCICMap, PCICMap)>,
        contracts_sources: &ContractSources,
        debug_steps: &[DebugStep],
        opcode_list: &[String],
        current_step: usize,
        call_kind: CallKind,
        draw_memory: &mut DrawMemory,
        stack_labels: bool,
        mem_utf: bool,
        show_shortcuts: bool,
    ) {
        let total_size = f.size();
        if total_size.width < 225 {
            Debugger::vertical_layout(
                f,
                address,
                identified_contracts,
                pc_ic_maps,
                contracts_sources,
                debug_steps,
                opcode_list,
                current_step,
                call_kind,
                draw_memory,
                stack_labels,
                mem_utf,
                show_shortcuts,
            );
        } else {
            Debugger::square_layout(
                f,
                address,
                identified_contracts,
                pc_ic_maps,
                contracts_sources,
                debug_steps,
                opcode_list,
                current_step,
                call_kind,
                draw_memory,
                stack_labels,
                mem_utf,
                show_shortcuts,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn vertical_layout(
        f: &mut Frame<'_>,
        address: Address,
        identified_contracts: &HashMap<Address, String>,
        pc_ic_maps: &BTreeMap<String, (PCICMap, PCICMap)>,
        contracts_sources: &ContractSources,
        debug_steps: &[DebugStep],
        opcode_list: &[String],
        current_step: usize,
        call_kind: CallKind,
        draw_memory: &mut DrawMemory,
        stack_labels: bool,
        mem_utf: bool,
        show_shortcuts: bool,
    ) {
        let total_size = f.size();
        let h_height = if show_shortcuts { 4 } else { 0 };

        if let [app, footer] = Layout::default()
            .constraints(
                [Constraint::Ratio(100 - h_height, 100), Constraint::Ratio(h_height, 100)].as_ref(),
            )
            .direction(Direction::Vertical)
            .split(total_size)[..]
        {
            if let [op_pane, stack_pane, memory_pane, src_pane] = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Ratio(1, 6),
                        Constraint::Ratio(1, 6),
                        Constraint::Ratio(1, 6),
                        Constraint::Ratio(3, 6),
                    ]
                    .as_ref(),
                )
                .split(app)[..]
            {
                if show_shortcuts {
                    Debugger::draw_footer(f, footer);
                }
                Debugger::draw_src(
                    f,
                    address,
                    identified_contracts,
                    pc_ic_maps,
                    contracts_sources,
                    debug_steps[current_step].pc,
                    call_kind,
                    src_pane,
                );
                Debugger::draw_op_list(
                    f,
                    address,
                    debug_steps,
                    opcode_list,
                    current_step,
                    draw_memory,
                    op_pane,
                );
                Debugger::draw_stack(
                    f,
                    debug_steps,
                    current_step,
                    stack_pane,
                    stack_labels,
                    draw_memory,
                );
                Debugger::draw_memory(
                    f,
                    debug_steps,
                    current_step,
                    memory_pane,
                    mem_utf,
                    draw_memory,
                );
            } else {
                panic!("unable to create vertical panes")
            }
        } else {
            panic!("unable to create footer / app")
        };
    }

    #[allow(clippy::too_many_arguments)]
    fn square_layout(
        f: &mut Frame<'_>,
        address: Address,
        identified_contracts: &HashMap<Address, String>,
        pc_ic_maps: &BTreeMap<String, (PCICMap, PCICMap)>,
        contracts_sources: &ContractSources,
        debug_steps: &[DebugStep],
        opcode_list: &[String],
        current_step: usize,
        call_kind: CallKind,
        draw_memory: &mut DrawMemory,
        stack_labels: bool,
        mem_utf: bool,
        show_shortcuts: bool,
    ) {
        let total_size = f.size();
        let h_height = if show_shortcuts { 4 } else { 0 };

        // split in 2 vertically

        if let [app, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [Constraint::Ratio(100 - h_height, 100), Constraint::Ratio(h_height, 100)].as_ref(),
            )
            .split(total_size)[..]
        {
            if let [left_pane, right_pane] = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
                .split(app)[..]
            {
                // split right pane horizontally to construct stack and memory
                if let [op_pane, src_pane] = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)].as_ref())
                    .split(left_pane)[..]
                {
                    if let [stack_pane, memory_pane] = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)].as_ref())
                        .split(right_pane)[..]
                    {
                        if show_shortcuts {
                            Debugger::draw_footer(f, footer)
                        };
                        Debugger::draw_src(
                            f,
                            address,
                            identified_contracts,
                            pc_ic_maps,
                            contracts_sources,
                            debug_steps[current_step].pc,
                            call_kind,
                            src_pane,
                        );
                        Debugger::draw_op_list(
                            f,
                            address,
                            debug_steps,
                            opcode_list,
                            current_step,
                            draw_memory,
                            op_pane,
                        );
                        Debugger::draw_stack(
                            f,
                            debug_steps,
                            current_step,
                            stack_pane,
                            stack_labels,
                            draw_memory,
                        );
                        Debugger::draw_memory(
                            f,
                            debug_steps,
                            current_step,
                            memory_pane,
                            mem_utf,
                            draw_memory,
                        );
                    }
                } else {
                    panic!("Couldn't generate horizontal split layout 1:2.");
                }
            } else {
                panic!("Couldn't generate vertical split layout 1:2.");
            }
        } else {
            panic!("Couldn't generate application & footer")
        }
    }

    fn draw_footer(f: &mut Frame<'_>, area: Rect) {
        let block_controls = Block::default();

        let text_output = vec![Line::from(Span::styled(
            "[q]: quit | [k/j]: prev/next op | [a/s]: prev/next jump | [c/C]: prev/next call | [g/G]: start/end", Style::default().add_modifier(Modifier::DIM))),
Line::from(Span::styled("[t]: stack labels | [m]: memory decoding | [shift + j/k]: scroll stack | [ctrl + j/k]: scroll memory | ['<char>]: goto breakpoint | [h] toggle help", Style::default().add_modifier(Modifier::DIM)))];

        let paragraph = Paragraph::new(text_output)
            .block(block_controls)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, area);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_src(
        f: &mut Frame<'_>,
        address: Address,
        identified_contracts: &HashMap<Address, String>,
        pc_ic_maps: &BTreeMap<String, (PCICMap, PCICMap)>,
        contracts_sources: &ContractSources,
        pc: usize,
        call_kind: CallKind,
        area: Rect,
    ) {
        let block_source_code = Block::default()
            .title(match call_kind {
                CallKind::Create | CallKind::Create2 => "Contract creation",
                CallKind::Call => "Contract call",
                CallKind::StaticCall => "Contract staticcall",
                CallKind::CallCode => "Contract callcode",
                CallKind::DelegateCall => "Contract delegatecall",
            })
            .borders(Borders::ALL);

        let mut text_output: Text = Text::from("");

        if let Some(contract_name) = identified_contracts.get(&address) {
            if let Some(files_source_code) = contracts_sources.0.get(contract_name) {
                let pc_ic_map = pc_ic_maps.get(contract_name);
                // find the contract source with the correct source_element's file_id
                if let Some((source_element, source_code)) = files_source_code.iter().find_map(
                    |(file_id, (source_code, contract_source))| {
                        // grab either the creation source map or runtime sourcemap
                        if let Some((Ok(source_map), ic)) =
                            if matches!(call_kind, CallKind::Create | CallKind::Create2) {
                                contract_source
                                    .bytecode
                                    .source_map()
                                    .zip(pc_ic_map.and_then(|(c, _)| c.get(&pc)))
                            } else {
                                contract_source
                                    .deployed_bytecode
                                    .bytecode
                                    .as_ref()
                                    .expect("no bytecode")
                                    .source_map()
                                    .zip(pc_ic_map.and_then(|(_, r)| r.get(&pc)))
                            }
                        {
                            let source_element = source_map[*ic].clone();
                            if let Some(index) = source_element.index {
                                if *file_id == index {
                                    Some((source_element, source_code))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    },
                ) {
                    // we are handed a vector of SourceElements that give
                    // us a span of sourcecode that is currently being executed
                    // This includes an offset and length. This vector is in
                    // instruction pointer order, meaning the location of
                    // the instruction - sum(push_bytes[..pc])
                    let offset = source_element.offset;
                    let len = source_element.length;
                    let max = source_code.len();

                    // split source into before, relevant, and after chunks
                    // split by line as well to do some formatting stuff
                    let mut before = source_code[..std::cmp::min(offset, max)]
                        .split_inclusive('\n')
                        .collect::<Vec<&str>>();
                    let actual = source_code
                        [std::cmp::min(offset, max)..std::cmp::min(offset + len, max)]
                        .split_inclusive('\n')
                        .map(|s| s.to_string())
                        .collect::<Vec<String>>();
                    let mut after = source_code[std::cmp::min(offset + len, max)..]
                        .split_inclusive('\n')
                        .collect::<VecDeque<&str>>();

                    let mut line_number = 0;

                    let num_lines = before.len() + actual.len() + after.len();
                    let height = area.height as usize;
                    let needed_highlight = actual.len();
                    let mid_len = before.len() + actual.len();

                    // adjust what text we show of the source code
                    let (start_line, end_line) = if needed_highlight > height {
                        // highlighted section is more lines than we have avail
                        (before.len(), before.len() + needed_highlight)
                    } else if height > num_lines {
                        // we can fit entire source
                        (0, num_lines)
                    } else {
                        let remaining = height - needed_highlight;
                        let mut above = remaining / 2;
                        let mut below = remaining / 2;
                        if below > after.len() {
                            // unused space below the highlight
                            above += below - after.len();
                        } else if above > before.len() {
                            // we have unused space above the highlight
                            below += above - before.len();
                        } else {
                            // no unused space
                        }

                        (before.len().saturating_sub(above), mid_len + below)
                    };

                    let max_line_num = num_lines.to_string().len();
                    // We check if there is other text on the same line before the
                    // highlight starts
                    if let Some(last) = before.pop() {
                        if !last.ends_with('\n') {
                            before.iter().skip(start_line).for_each(|line| {
                                text_output.lines.push(Line::from(vec![
                                    Span::styled(
                                        format!(
                                            "{: >max_line_num$}",
                                            line_number.to_string(),
                                            max_line_num = max_line_num
                                        ),
                                        Style::default().fg(Color::Gray).bg(Color::DarkGray),
                                    ),
                                    Span::styled(
                                        "\u{2800} ".to_string() + line,
                                        Style::default().add_modifier(Modifier::DIM),
                                    ),
                                ]));
                                line_number += 1;
                            });

                            text_output.lines.push(Line::from(vec![
                                Span::styled(
                                    format!(
                                        "{: >max_line_num$}",
                                        line_number.to_string(),
                                        max_line_num = max_line_num
                                    ),
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .bg(Color::DarkGray)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::raw("\u{2800} "),
                                Span::raw(last),
                                Span::styled(
                                    actual[0].to_string(),
                                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                                ),
                            ]));
                            line_number += 1;

                            actual.iter().skip(1).for_each(|s| {
                                text_output.lines.push(Line::from(vec![
                                    Span::styled(
                                        format!(
                                            "{: >max_line_num$}",
                                            line_number.to_string(),
                                            max_line_num = max_line_num
                                        ),
                                        Style::default()
                                            .fg(Color::Cyan)
                                            .bg(Color::DarkGray)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::raw("\u{2800} "),
                                    Span::styled(
                                        // this is a hack to add coloring
                                        // because tui does weird trimming
                                        if s.is_empty() || s == "\n" {
                                            "\u{2800} \n".to_string()
                                        } else {
                                            s.to_string()
                                        },
                                        Style::default()
                                            .fg(Color::Cyan)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                ]));
                                line_number += 1;
                            });
                        } else {
                            before.push(last);
                            before.iter().skip(start_line).for_each(|line| {
                                text_output.lines.push(Line::from(vec![
                                    Span::styled(
                                        format!(
                                            "{: >max_line_num$}",
                                            line_number.to_string(),
                                            max_line_num = max_line_num
                                        ),
                                        Style::default().fg(Color::Gray).bg(Color::DarkGray),
                                    ),
                                    Span::styled(
                                        "\u{2800} ".to_string() + line,
                                        Style::default().add_modifier(Modifier::DIM),
                                    ),
                                ]));

                                line_number += 1;
                            });
                            actual.iter().for_each(|s| {
                                text_output.lines.push(Line::from(vec![
                                    Span::styled(
                                        format!(
                                            "{: >max_line_num$}",
                                            line_number.to_string(),
                                            max_line_num = max_line_num
                                        ),
                                        Style::default()
                                            .fg(Color::Cyan)
                                            .bg(Color::DarkGray)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::raw("\u{2800} "),
                                    Span::styled(
                                        if s.is_empty() || s == "\n" {
                                            "\u{2800} \n".to_string()
                                        } else {
                                            s.to_string()
                                        },
                                        Style::default()
                                            .fg(Color::Cyan)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                ]));
                                line_number += 1;
                            });
                        }
                    } else {
                        actual.iter().for_each(|s| {
                            text_output.lines.push(Line::from(vec![
                                Span::styled(
                                    format!(
                                        "{: >max_line_num$}",
                                        line_number.to_string(),
                                        max_line_num = max_line_num
                                    ),
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .bg(Color::DarkGray)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::raw("\u{2800} "),
                                Span::styled(
                                    if s.is_empty() || s == "\n" {
                                        "\u{2800} \n".to_string()
                                    } else {
                                        s.to_string()
                                    },
                                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                                ),
                            ]));
                            line_number += 1;
                        });
                    }

                    // fill in the rest of the line as unhighlighted
                    if let Some(last) = actual.last() {
                        if !last.ends_with('\n') {
                            if let Some(post) = after.pop_front() {
                                if let Some(last) = text_output.lines.last_mut() {
                                    last.spans.push(Span::raw(post));
                                }
                            }
                        }
                    }

                    // add after highlighted text
                    while mid_len + after.len() > end_line {
                        after.pop_back();
                    }
                    after.iter().for_each(|line| {
                        text_output.lines.push(Line::from(vec![
                            Span::styled(
                                format!(
                                    "{: >max_line_num$}",
                                    line_number.to_string(),
                                    max_line_num = max_line_num
                                ),
                                Style::default().fg(Color::Gray).bg(Color::DarkGray),
                            ),
                            Span::styled(
                                "\u{2800} ".to_string() + line,
                                Style::default().add_modifier(Modifier::DIM),
                            ),
                        ]));
                        line_number += 1;
                    });
                } else {
                    text_output.extend(Text::from("No sourcemap for contract"));
                }
            } else {
                text_output
                    .extend(Text::from(format!("No srcmap index for contract {contract_name}")));
            }
        } else {
            text_output.extend(Text::from(format!("Unknown contract at address {address}")));
        }

        let paragraph =
            Paragraph::new(text_output).block(block_source_code).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    /// Draw opcode list into main component
    fn draw_op_list(
        f: &mut Frame<'_>,
        address: Address,
        debug_steps: &[DebugStep],
        opcode_list: &[String],
        current_step: usize,
        draw_memory: &mut DrawMemory,
        area: Rect,
    ) {
        let block_source_code = Block::default()
            .title(format!(
                "Address: {} | PC: {} | Gas used in call: {}",
                address,
                if let Some(step) = debug_steps.get(current_step) {
                    step.pc.to_string()
                } else {
                    "END".to_string()
                },
                debug_steps[current_step].total_gas_used,
            ))
            .borders(Borders::ALL);
        let mut text_output: Vec<Line> = Vec::new();

        // Scroll:
        // Focused line is line that should always be at the center of the screen.
        let display_start;

        let height = area.height as i32;
        let extra_top_lines = height / 2;
        let prev_start = draw_memory.current_startline;
        // Absolute minimum start line
        let abs_min_start = 0;
        // Adjust for weird scrolling for max top line
        let abs_max_start = (opcode_list.len() as i32 - 1) - (height / 2);
        // actual minimum start line
        let mut min_start =
            max(current_step as i32 - height + extra_top_lines, abs_min_start) as usize;

        // actual max start line
        let mut max_start =
            max(min(current_step as i32 - extra_top_lines, abs_max_start), abs_min_start) as usize;

        // Sometimes, towards end of file, maximum and minim lines have swapped values. Swap if the
        // case
        if min_start > max_start {
            std::mem::swap(&mut min_start, &mut max_start);
        }

        if prev_start < min_start {
            display_start = min_start;
        } else if prev_start > max_start {
            display_start = max_start;
        } else {
            display_start = prev_start;
        }
        draw_memory.current_startline = display_start;

        let max_pc_len =
            debug_steps.iter().fold(0, |max_val, val| val.pc.max(max_val)).to_string().len();

        // Define closure that prints one more line of source code
        let mut add_new_line = |line_number| {
            let bg_color = if line_number == current_step { Color::DarkGray } else { Color::Reset };

            // Format line number
            let line_number_format = if line_number == current_step {
                let step: &DebugStep = &debug_steps[line_number];
                format!("{:0>max_pc_len$x}|▶", step.pc)
            } else if line_number < debug_steps.len() {
                let step: &DebugStep = &debug_steps[line_number];
                format!("{:0>max_pc_len$x}| ", step.pc)
            } else {
                "END CALL".to_string()
            };

            if let Some(op) = opcode_list.get(line_number) {
                text_output.push(Line::from(Span::styled(
                    format!("{line_number_format}{op}"),
                    Style::default().fg(Color::White).bg(bg_color),
                )));
            } else {
                text_output.push(Line::from(Span::styled(
                    line_number_format,
                    Style::default().fg(Color::White).bg(bg_color),
                )));
            }
        };
        for number in display_start..opcode_list.len() {
            add_new_line(number);
        }
        // Add one more "phantom" line so we see line where current segment execution ends
        add_new_line(opcode_list.len());
        let paragraph =
            Paragraph::new(text_output).block(block_source_code).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// Draw the stack into the stack pane
    fn draw_stack(
        f: &mut Frame<'_>,
        debug_steps: &[DebugStep],
        current_step: usize,
        area: Rect,
        stack_labels: bool,
        draw_memory: &DrawMemory,
    ) {
        let stack = &debug_steps[current_step].stack;
        let stack_space =
            Block::default().title(format!("Stack: {}", stack.len())).borders(Borders::ALL);
        let min_len = usize::max(format!("{}", stack.len()).len(), 2);

        let params = if let Instruction::OpCode(op) = debug_steps[current_step].instruction {
            OpcodeParam::of(op)
        } else {
            &[]
        };

        let text: Vec<Line> = stack
            .iter()
            .rev()
            .enumerate()
            .skip(draw_memory.current_stack_startline)
            .map(|(i, stack_item)| {
                let param = params.iter().find(|param| param.index == i);
                let mut words: Vec<Span> = (0..32)
                    .rev()
                    .map(|i| stack_item.byte(i))
                    .map(|byte| {
                        Span::styled(
                            format!("{byte:02x} "),
                            if param.is_some() {
                                Style::default().fg(Color::Cyan)
                            } else if byte == 0 {
                                // this improves compatibility across terminals by not combining
                                // color with DIM modifier
                                Style::default().add_modifier(Modifier::DIM)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        )
                    })
                    .collect();

                if stack_labels {
                    if let Some(param) = param {
                        words.push(Span::raw(format!("| {}", param.name)));
                    } else {
                        words.push(Span::raw("| ".to_string()));
                    }
                }

                let mut spans = vec![Span::styled(
                    format!("{i:0min_len$}| "),
                    Style::default().fg(Color::White),
                )];
                spans.extend(words);
                spans.push(Span::raw("\n"));

                Line::from(spans)
            })
            .collect();

        let paragraph = Paragraph::new(text).block(stack_space).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// The memory_access variable stores the index on the stack that indicates the memory
    /// offset/size accessed by the given opcode:
    ///   (read memory offset, read memory size, write memory offset, write memory size)
    ///   >= 1: the stack index
    ///   0: no memory access
    ///   -1: a fixed size of 32 bytes
    ///   -2: a fixed size of 1 byte
    /// The return value is a tuple about accessed memory region by the given opcode:
    ///   (read memory offset, read memory size, write memory offset, write memory size)
    fn get_memory_access(
        op: u8,
        stack: &Vec<U256>,
    ) -> (Option<usize>, Option<usize>, Option<usize>, Option<usize>) {
        let memory_access = match op {
            opcode::KECCAK256 | opcode::RETURN | opcode::REVERT => (1, 2, 0, 0),
            opcode::CALLDATACOPY | opcode::CODECOPY | opcode::RETURNDATACOPY => (0, 0, 1, 3),
            opcode::EXTCODECOPY => (0, 0, 2, 4),
            opcode::MLOAD => (1, -1, 0, 0),
            opcode::MSTORE => (0, 0, 1, -1),
            opcode::MSTORE8 => (0, 0, 1, -2),
            opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
                (1, 2, 0, 0)
            }
            opcode::CREATE | opcode::CREATE2 => (2, 3, 0, 0),
            opcode::CALL | opcode::CALLCODE => (4, 5, 0, 0),
            opcode::DELEGATECALL | opcode::STATICCALL => (3, 4, 0, 0),
            _ => Default::default(),
        };

        let stack_len = stack.len();
        let get_size = |stack_index| match stack_index {
            -2 => Some(1),
            -1 => Some(32),
            0 => None,
            1.. => Some(stack[stack_len - stack_index as usize].saturating_to()),
            _ => panic!("invalid stack index"),
        };

        let (read_offset, read_size, write_offset, write_size) = (
            get_size(memory_access.0),
            get_size(memory_access.1),
            get_size(memory_access.2),
            get_size(memory_access.3),
        );
        (read_offset, read_size, write_offset, write_size)
    }

    /// Draw memory in memory pane
    fn draw_memory(
        f: &mut Frame<'_>,
        debug_steps: &[DebugStep],
        current_step: usize,
        area: Rect,
        mem_utf8: bool,
        draw_mem: &DrawMemory,
    ) {
        let memory = &debug_steps[current_step].memory;
        let memory_space = Block::default()
            .title(format!("Memory (max expansion: {} bytes)", memory.len()))
            .borders(Borders::ALL);
        let max_i = memory.len() / 32;
        let min_len = format!("{:x}", max_i * 32).len();

        // color memory region based on write/read
        let mut offset: Option<usize> = None;
        let mut size: Option<usize> = None;
        let mut color = None;
        if let Instruction::OpCode(op) = debug_steps[current_step].instruction {
            let stack_len = debug_steps[current_step].stack.len();
            if stack_len > 0 {
                let (read_offset, read_size, write_offset, write_size) =
                    Debugger::get_memory_access(op, &debug_steps[current_step].stack);
                if read_offset.is_some() {
                    offset = read_offset;
                    size = read_size;
                    color = Some(Color::Cyan);
                } else if write_offset.is_some() {
                    offset = write_offset;
                    size = write_size;
                    color = Some(Color::Red);
                }
            }
        }

        // color word on previous write op
        if current_step > 0 {
            let prev_step = current_step - 1;
            if let Instruction::OpCode(op) = debug_steps[prev_step].instruction {
                let (_, _, write_offset, write_size) =
                    Debugger::get_memory_access(op, &debug_steps[prev_step].stack);
                if write_offset.is_some() {
                    offset = write_offset;
                    size = write_size;
                    color = Some(Color::Green);
                }
            }
        }

        let height = area.height as usize;
        let end_line = draw_mem.current_mem_startline + height;

        let text: Vec<Line> = memory
            .chunks(32)
            .enumerate()
            .skip(draw_mem.current_mem_startline)
            .take_while(|(i, _)| i < &end_line)
            .map(|(i, mem_word)| {
                let words: Vec<Span> = mem_word
                    .iter()
                    .enumerate()
                    .map(|(j, byte)| {
                        Span::styled(
                            format!("{byte:02x} "),
                            if let (Some(offset), Some(size), Some(color)) = (offset, size, color) {
                                if (i == offset / 32 && j >= offset % 32) ||
                                    (i > offset / 32 && i < (offset + size - 1) / 32) ||
                                    (i == (offset + size - 1) / 32 &&
                                        j <= (offset + size - 1) % 32)
                                {
                                    // [offset, offset + size] is the memory region to be colored.
                                    // If a byte at row i and column j in the memory panel
                                    // falls in this region, set the color.
                                    Style::default().fg(color)
                                } else if *byte == 0 {
                                    Style::default().add_modifier(Modifier::DIM)
                                } else {
                                    Style::default().fg(Color::White)
                                }
                            } else if *byte == 0 {
                                Style::default().add_modifier(Modifier::DIM)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        )
                    })
                    .collect();

                let mut spans = vec![Span::styled(
                    format!("{:0min_len$x}| ", i * 32),
                    Style::default().fg(Color::White),
                )];
                spans.extend(words);

                if mem_utf8 {
                    let chars: Vec<Span> = mem_word
                        .chunks(4)
                        .map(|utf| {
                            if let Ok(utf_str) = std::str::from_utf8(utf) {
                                Span::raw(utf_str.replace(char::from(0), "."))
                            } else {
                                Span::raw(".")
                            }
                        })
                        .collect();
                    spans.push(Span::raw("|"));
                    spans.extend(chars);
                }

                spans.push(Span::raw("\n"));

                Line::from(spans)
            })
            .collect();
        let paragraph = Paragraph::new(text).block(memory_space).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }
}
