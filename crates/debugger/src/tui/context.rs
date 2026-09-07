//! Debugger context and event handler implementation.

use super::storage::{
    StorageAccess, StorageSpace, hex_u256, next_storage_write_values, storage_access_at,
    storage_accesses_until,
};
use crate::{DebugNode, DebuggerLayout, ExitReason, debugger::DebuggerContext};
use alloy_primitives::{
    Address, B256, U256, hex,
    map::{B256Map, IndexMap},
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use foundry_compilers::artifacts::sourcemap::SourceElement;
use foundry_evm_core::buffer::{BufferKind, get_buffer_accesses};
use foundry_evm_traces::debug::SourceData;
use foundry_tui::TuiApp;
use ratatui::Frame;
use revm::bytecode::opcode::OpCode;
use revm_inspectors::tracing::types::{CallKind, CallTraceStep};
use std::{fmt::Write, ops::ControlFlow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    Info,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusMessage {
    pub(crate) kind: StatusKind,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveInternalCallLocation {
    pub(crate) trace_node_idx: usize,
    pub(crate) marker_node_idx: usize,
    pub(crate) marker_step_idx: usize,
    pub(crate) entry_step: usize,
    pub(crate) end_step: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveInternalCallCache {
    pub(crate) current_node_idx: usize,
    pub(crate) trace_node_idx: usize,
    pub(crate) absolute_step: usize,
    pub(crate) location: Option<ActiveInternalCallLocation>,
}

impl ActiveInternalCallCache {
    pub(crate) const fn matches(
        self,
        current_node_idx: usize,
        trace_node_idx: usize,
        absolute_step: usize,
    ) -> bool {
        self.current_node_idx == current_node_idx
            && self.trace_node_idx == trace_node_idx
            && self.absolute_step == absolute_step
    }
}

/// This is currently used to remember last scroll position so screen doesn't wiggle as much.
#[derive(Default)]
pub(crate) struct DrawMemory {
    pub(crate) inner_call_index: usize,
    pub(crate) current_buf_startline: usize,
    pub(crate) current_storage_startline: usize,
    pub(crate) current_stack_startline: usize,
    pub(crate) active_internal_call: Option<ActiveInternalCallCache>,
}

#[derive(Default)]
struct OpcodeListState {
    inner_call_index: Option<usize>,
    max_pc: usize,
}

/// The active prompt and its current contents.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    Pc(String),
    BufferOffset(String),
    Command(String),
    OpcodeSearch(String),
}

impl Prompt {
    const fn input_mut(&mut self) -> &mut String {
        match self {
            Self::Pc(input)
            | Self::BufferOffset(input)
            | Self::Command(input)
            | Self::OpcodeSearch(input) => input,
        }
    }

    fn accepts(&self, c: char) -> bool {
        match self {
            Self::Pc(_) => c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | ':'),
            Self::BufferOffset(input) => is_buffer_offset_input_char(input, c),
            Self::Command(_) => !c.is_control(),
            Self::OpcodeSearch(_) => true,
        }
    }
}

pub(crate) struct TUIContext<'a> {
    pub(crate) debugger_context: &'a mut DebuggerContext,

    /// Buffer for keys prior to execution, i.e. '10' + 'k' => move up 10 operations.
    pub(crate) key_buffer: String,
    /// Current prompt, if one is active.
    pub(crate) prompt: Option<Prompt>,
    /// Last opcode search term, used by repeat-search shortcuts.
    pub(crate) last_opcode_search: Option<String>,
    /// Last status or error message to show in the footer.
    pub(crate) status: Option<StatusMessage>,
    /// Current step in the debug steps.
    pub(crate) current_step: usize,
    pub(crate) draw_memory: DrawMemory,
    opcode_list: OpcodeListState,

    pub(crate) stack_labels: bool,
    /// Whether to decode active buffer as utf8 or not.
    pub(crate) buf_utf: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) show_opcodes: bool,
    pub(crate) show_source: bool,
    pub(crate) show_variables: bool,
    pub(crate) show_stack: bool,
    pub(crate) show_data: bool,
    /// The currently active buffer (memory, calldata, returndata) to be drawn.
    pub(crate) active_buffer: BufferKind,
    active_storage: Option<StorageSpace>,
}

impl<'a> TUIContext<'a> {
    pub(crate) fn new(debugger_context: &'a mut DebuggerContext) -> Self {
        TUIContext {
            debugger_context,

            key_buffer: String::with_capacity(64),
            prompt: None,
            last_opcode_search: None,
            status: None,
            current_step: 0,
            draw_memory: DrawMemory::default(),
            opcode_list: OpcodeListState::default(),

            stack_labels: false,
            buf_utf: false,
            show_shortcuts: true,
            show_opcodes: true,
            show_source: true,
            show_variables: true,
            show_stack: true,
            show_data: true,
            active_buffer: BufferKind::Memory,
            active_storage: None,
        }
    }

    pub(crate) fn init(&mut self) {
        self.refresh_opcode_list_state();
    }

    pub(crate) fn debug_arena(&self) -> &[DebugNode] {
        &self.debugger_context.debug_arena
    }

    pub(crate) const fn layout(&self) -> DebuggerLayout {
        self.debugger_context.layout
    }

    pub(crate) fn debug_call(&self) -> &DebugNode {
        &self.debug_arena()[self.draw_memory.inner_call_index]
    }

    /// Returns the current call address.
    pub(crate) fn address(&self) -> &Address {
        &self.debug_call().address
    }

    /// Returns the current call kind.
    pub(crate) fn call_kind(&self) -> CallKind {
        self.debug_call().kind
    }

    /// Returns the current debug steps.
    pub(crate) fn debug_steps(&self) -> &[CallTraceStep] {
        &self.debug_call().steps
    }

    /// Returns the current debug step.
    pub(crate) fn current_step(&self) -> &CallTraceStep {
        &self.debug_steps()[self.current_step]
    }

    pub(super) const fn opcode_max_pc(&self) -> usize {
        self.opcode_list.max_pc
    }

    fn refresh_opcode_list_state(&mut self) {
        let inner_call_index = self.draw_memory.inner_call_index;
        if self.opcode_list.inner_call_index == Some(inner_call_index) {
            return;
        }

        let debug_steps = &self.debugger_context.debug_arena[inner_call_index].steps;
        self.opcode_list.max_pc = debug_steps.iter().map(|step| step.pc).max().unwrap_or(0);
        self.opcode_list.inner_call_index = Some(inner_call_index);
    }

    fn active_buffer(&self) -> &[u8] {
        self.buffer(&self.active_buffer)
    }

    pub(super) const fn active_storage(&self) -> Option<StorageSpace> {
        self.active_storage
    }

    pub(super) fn storage_accesses(&self, space: StorageSpace) -> IndexMap<U256, StorageAccess> {
        storage_accesses_until(
            self.debug_arena(),
            self.draw_memory.inner_call_index,
            self.current_step,
            space,
        )
    }

    pub(super) fn storage_label(
        &self,
        space: StorageSpace,
        slot: U256,
        storage_values: Option<&B256Map<B256>>,
        next_values: Option<&B256Map<B256>>,
    ) -> Option<String> {
        if space != StorageSpace::Persistent {
            return None;
        }
        let identifier = self.debugger_context.slot_identifiers.as_ref()?.get(self.address())?;
        let slot = B256::from(slot);
        identifier
            .identify(&slot, None)
            .or_else(|| {
                storage_values.and_then(|values| identifier.identify_bytes_or_string(&slot, values))
            })
            .or_else(|| {
                // Solc writes newly allocated string/bytes payload slots before their base-slot
                // length, so retry with the next write values when the current state is stale.
                next_values.and_then(|values| identifier.identify_bytes_or_string(&slot, values))
            })
            .map(|info| info.label)
    }

    pub(super) fn next_storage_write_values(&self) -> B256Map<B256> {
        next_storage_write_values(
            self.debug_arena(),
            self.draw_memory.inner_call_index,
            self.current_step,
        )
    }

    fn active_data_len(&self) -> usize {
        self.active_storage.map_or_else(
            || self.active_buffer().len().div_ceil(32),
            |space| self.storage_accesses(space).len(),
        )
    }

    fn buffer(&self, buffer: &BufferKind) -> &[u8] {
        match buffer {
            BufferKind::Memory => self.current_step().memory.as_ref().map_or(&[], |m| m.as_bytes()),
            BufferKind::Calldata => &self.debug_call().calldata,
            BufferKind::Returndata => &self.current_step().returndata,
        }
    }

    pub(crate) const fn active_buffer_name(&self) -> &'static str {
        buffer_name(&self.active_buffer)
    }

    /// Returns source map, source code and source name of the current line.
    pub(crate) fn src_map(&self) -> Result<(SourceElement, &SourceData), String> {
        let address = self.address();
        let Some(contract_name) = self.debugger_context.identified_contracts.get(address) else {
            return Err(format!("Unknown contract at address {address}"));
        };

        self.debugger_context
            .contracts_sources
            .find_source_mapping(
                contract_name,
                self.current_step().pc as u32,
                self.debug_call().kind.is_any_create(),
            )
            .ok_or_else(|| format!("No source map for contract {contract_name}"))
    }
}

impl TUIContext<'_> {
    pub(crate) fn handle_event(&mut self, event: Event) -> ControlFlow<ExitReason> {
        let ret = match event {
            Event::Key(event) => self.handle_key_event(event),
            Event::Mouse(event) => self.handle_mouse_event(event),
            _ => ControlFlow::Continue(()),
        };
        self.refresh_opcode_list_state();
        ret
    }

    fn handle_key_event(&mut self, event: KeyEvent) -> ControlFlow<ExitReason> {
        if let Some(prompt) = self.prompt.take() {
            self.handle_prompt_key_event(prompt, event);
            return ControlFlow::Continue(());
        }

        // Breakpoints
        if let KeyCode::Char(c) = event.code
            && c.is_alphabetic()
            && self.key_buffer.starts_with('\'')
        {
            self.handle_breakpoint(c);
            return ControlFlow::Continue(());
        }

        let control = event.modifiers.contains(KeyModifiers::CONTROL);

        match event.code {
            // Exit
            KeyCode::Char('q') => return ControlFlow::Break(ExitReason::CharExit),

            // Scroll up the active data pane
            KeyCode::Char('k') | KeyCode::Up if control => self.repeat(|this| {
                if this.active_storage.is_some() {
                    this.draw_memory.current_storage_startline =
                        this.draw_memory.current_storage_startline.saturating_sub(1);
                } else {
                    this.draw_memory.current_buf_startline =
                        this.draw_memory.current_buf_startline.saturating_sub(1);
                }
            }),
            // Scroll down the active data pane
            KeyCode::Char('j') | KeyCode::Down if control => {
                let max_line = self.active_data_len().saturating_sub(1);
                self.repeat(|this| {
                    if this.active_storage.is_some() {
                        if this.draw_memory.current_storage_startline < max_line {
                            this.draw_memory.current_storage_startline += 1;
                        }
                    } else if this.draw_memory.current_buf_startline < max_line {
                        this.draw_memory.current_buf_startline += 1;
                    }
                });
            }

            // Move up
            KeyCode::Char('k') | KeyCode::Up => self.repeat(Self::step_back),
            // Move down
            KeyCode::Char('j') | KeyCode::Down => self.repeat(Self::step),

            // Scroll up the stack
            KeyCode::Char('K') => self.repeat(|this| {
                this.draw_memory.current_stack_startline =
                    this.draw_memory.current_stack_startline.saturating_sub(1);
            }),
            // Scroll down the stack
            KeyCode::Char('J') => self.repeat(|this| {
                let max_stack =
                    this.current_step().stack.as_ref().map_or(0, |s| s.len()).saturating_sub(1);
                if this.draw_memory.current_stack_startline < max_stack {
                    this.draw_memory.current_stack_startline += 1;
                }
            }),

            // Cycle buffers
            KeyCode::Char('b') => {
                if self.active_storage.take().is_none() {
                    self.active_buffer = self.active_buffer.next();
                }
                self.draw_memory.current_buf_startline = 0;
                self.set_info(format!("Active buffer: {}", self.active_buffer_name()));
            }

            // Cycle layout
            KeyCode::Char('l') => self.cycle_layout(),

            // Go to top of file
            KeyCode::Char('g') => {
                self.draw_memory.inner_call_index = 0;
                self.current_step = 0;
                self.update_scroll_positions();
            }

            // Go to bottom of file
            KeyCode::Char('G') => {
                self.draw_memory.inner_call_index = self.debug_arena().len() - 1;
                self.current_step = self.n_steps() - 1;
                self.update_scroll_positions();
            }

            // Go to previous call
            KeyCode::Char('c') if self.draw_memory.inner_call_index > 0 => {
                self.draw_memory.inner_call_index -= 1;
                self.current_step = self.n_steps() - 1;
                self.update_scroll_positions();
            }

            // Go to next call
            KeyCode::Char('C')
                if self.debug_arena().len() > self.draw_memory.inner_call_index + 1 =>
            {
                self.draw_memory.inner_call_index += 1;
                self.current_step = 0;
                self.update_scroll_positions();
            }

            // Step forward
            KeyCode::Char('s') => self.repeat(|this| {
                let remaining_steps = &this.debug_steps()[this.current_step..];
                if let Some((i, _)) =
                    remaining_steps.iter().enumerate().skip(1).find(|(i, step)| {
                        let prev = &remaining_steps[*i - 1];
                        is_jump(step, prev)
                    })
                {
                    this.current_step += i;
                    this.update_scroll_positions();
                }
            }),

            // Step backwards
            KeyCode::Char('a') => self.repeat(|this| {
                let ops = &this.debug_steps()[..this.current_step];
                this.current_step = ops
                    .iter()
                    .enumerate()
                    .skip(1)
                    .rev()
                    .find(|&(i, op)| {
                        let prev = &ops[i - 1];
                        is_jump(op, prev)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or_default();
                this.update_scroll_positions();
            }),

            // Toggle stack labels
            KeyCode::Char('t') => {
                self.stack_labels = !self.stack_labels;
                self.set_info(format!("Stack labels: {}", toggle_state(self.stack_labels)));
            }

            // Toggle memory UTF-8 decoding
            KeyCode::Char('m') => {
                self.buf_utf = !self.buf_utf;
                self.set_info(format!("UTF-8 decoding: {}", toggle_state(self.buf_utf)));
            }

            // Go to program counter
            KeyCode::Char('p') => {
                self.key_buffer.clear();
                self.status = None;
                self.prompt = Some(Prompt::Pc(String::new()));
            }

            // Go to byte offset in the active buffer
            KeyCode::Char('o') => {
                self.key_buffer.clear();
                self.status = None;
                if let Some(space) = self.active_storage {
                    self.prompt = Some(Prompt::Command(format!("{} ", space.command())));
                } else {
                    self.prompt = Some(Prompt::BufferOffset(String::new()));
                }
            }

            // Run debugger command
            KeyCode::Char(':') => {
                self.key_buffer.clear();
                self.status = None;
                self.prompt = Some(Prompt::Command(String::new()));
            }

            // Search opcodes in the current call
            KeyCode::Char('/') => {
                self.key_buffer.clear();
                self.status = None;
                self.prompt = Some(Prompt::OpcodeSearch(String::new()));
            }

            // Repeat opcode search forward
            KeyCode::Char('n') => self.repeat(|this| {
                this.repeat_opcode_search(SearchDirection::Forward);
            }),

            // Repeat opcode search backward
            KeyCode::Char('N') => self.repeat(|this| {
                this.repeat_opcode_search(SearchDirection::Backward);
            }),

            // Toggle help notice
            KeyCode::Char('h') => {
                self.show_shortcuts = !self.show_shortcuts;
                let state = if self.show_shortcuts { "shown" } else { "hidden" };
                self.set_info(format!("Shortcut help: {state}"));
            }

            // Numbers for repeating commands or breakpoints
            KeyCode::Char(
                other @ ('0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '\''),
            ) => {
                // Early return to not clear the buffer.
                self.key_buffer.push(other);
                return ControlFlow::Continue(());
            }

            // Unknown/unhandled key code
            _ => {}
        };

        self.key_buffer.clear();
        ControlFlow::Continue(())
    }

    fn handle_prompt_key_event(&mut self, mut prompt: Prompt, event: KeyEvent) {
        match event.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                match prompt {
                    Prompt::Pc(input) => self.goto_pc_from_input(&input),
                    Prompt::BufferOffset(input) => self.goto_buffer_offset_from_input(&input),
                    Prompt::Command(input) => self.run_command_from_input(&input),
                    Prompt::OpcodeSearch(input) => self.search_opcode_from_input(&input),
                }
                return;
            }
            KeyCode::Backspace => {
                prompt.input_mut().pop();
            }
            KeyCode::Char(c)
                if !event.modifiers.contains(KeyModifiers::CONTROL) && prompt.accepts(c) =>
            {
                prompt.input_mut().push(c);
            }
            _ => {}
        }
        self.prompt = Some(prompt);
    }

    fn search_opcode_from_input(&mut self, input: &str) {
        let query = input.trim();
        if query.is_empty() {
            self.set_error("Enter an opcode search term".to_string());
            return;
        }

        self.last_opcode_search = Some(query.to_string());
        self.search_opcode(query, SearchDirection::Forward);
    }

    fn repeat_opcode_search(&mut self, direction: SearchDirection) {
        let Some(query) = self.last_opcode_search.clone() else {
            self.set_error("No previous opcode search".to_string());
            return;
        };

        self.search_opcode(&query, direction);
    }

    fn search_opcode(&mut self, query: &str, direction: SearchDirection) {
        let Some((node_index, step_index)) = find_opcode_match(
            self.debug_arena(),
            self.draw_memory.inner_call_index,
            self.current_step,
            query,
            direction,
        ) else {
            self.set_error(format!("No opcode matching `{query}` in current call"));
            return;
        };

        self.draw_memory.inner_call_index = node_index;
        self.current_step = step_index;
        self.update_scroll_positions();

        let pc = self.current_step().pc;
        let opcode = pretty_opcode(self.current_step());
        self.set_info(format!("Found `{query}` at PC 0x{pc:x} ({pc}): {opcode}"));
    }

    fn goto_pc_from_input(&mut self, input: &str) {
        self.move_to_pc_from_input(input, find_pc_target);
    }

    fn move_to_pc_from_input(
        &mut self,
        input: &str,
        find_target: impl Fn(&[DebugNode], usize, usize, usize) -> Option<StepTarget>,
    ) {
        let candidates = match parse_pc_candidates(input) {
            Ok(candidates) => candidates,
            Err(err) => {
                self.set_error(err);
                return;
            }
        };

        let mut found = Vec::new();
        for &candidate in &candidates {
            if let Some(target) = find_target(
                self.debug_arena(),
                self.draw_memory.inner_call_index,
                self.current_step,
                candidate.pc,
            ) {
                found.push((candidate, target));
            }
        }

        match found.as_slice() {
            [] => {
                let current = self.debug_call();
                let outside = candidates.iter().any(|candidate| {
                    pc_exists_outside_code_context(self.debug_arena(), current, candidate.pc)
                });
                let pc = if let [candidate] = candidates.as_slice() {
                    let pc = candidate.pc;
                    format!("PC 0x{pc:x} ({pc})")
                } else {
                    format!("PC `{}`", input.trim())
                };
                let mut msg = format!("{pc} not found in current contract");
                if outside {
                    msg.push_str("; it exists in another contract, switch calls first");
                }
                self.set_error(msg);
            }
            [(candidate, target)] => self.apply_pc_target(*candidate, *target),
            _ => {
                let input = input.trim();
                let options = found
                    .iter()
                    .map(|(candidate, _)| candidate.describe())
                    .collect::<Vec<_>>()
                    .join(" and ");
                self.set_error(format!(
                    "Ambiguous PC `{input}`: {options} both exist; use d:<pc> or 0x<pc>"
                ));
            }
        }
    }

    fn apply_pc_target(&mut self, candidate: PcCandidate, target: StepTarget) {
        let already_at_target = self.draw_memory.inner_call_index == target.node_index
            && self.current_step == target.step_index;

        self.draw_memory.inner_call_index = target.node_index;
        self.current_step = target.step_index;
        self.draw_memory.current_buf_startline = 0;
        self.draw_memory.current_stack_startline = 0;
        self.update_scroll_positions();
        self.key_buffer.clear();

        let pc = candidate.pc;
        let scope = match target.scope {
            StepTargetScope::CurrentNode => "current trace",
            StepTargetScope::SameCodeContext => "same contract",
        };
        let action = if already_at_target { "Already at" } else { "Jumped to" };
        self.set_info(format!("{action} PC 0x{pc:x} ({pc}) in {scope}"));
    }

    fn goto_buffer_offset_from_input(&mut self, input: &str) {
        self.goto_buffer_offset(self.active_buffer, input);
    }

    fn goto_buffer_offset(&mut self, buffer: BufferKind, input: &str) {
        let offset = match parse_buffer_offset(input) {
            Ok(offset) => offset,
            Err(err) => {
                self.set_error(err);
                return;
            }
        };

        let buffer_name = buffer_name(&buffer);
        let buffer_len = self.buffer(&buffer).len();
        if buffer_len == 0 {
            self.set_error(format!("Current {buffer_name} buffer is empty"));
            return;
        }

        if offset >= buffer_len {
            self.set_error(format!(
                "{buffer_name} offset 0x{offset:x} ({offset}) is outside the {buffer_len}-byte buffer"
            ));
            return;
        }

        self.active_buffer = buffer;
        self.active_storage = None;
        self.apply_buffer_offset(offset);
    }

    fn run_command_from_input(&mut self, input: &str) {
        let input = input.trim();
        let input = input.strip_prefix(':').unwrap_or(input).trim_start();
        if input.is_empty() {
            self.set_error("Enter a debugger command".to_string());
            return;
        }

        let mut parts = input.split_whitespace();
        let command = parts.next().unwrap();
        let is_continue_command = CONTINUE_COMMANDS.contains(&command);
        if is_continue_command || PC_COMMANDS.contains(&command) {
            let Some(pc) = parts.next() else {
                return self.set_error(command_usage(command, "<pc>"));
            };
            if parts.next().is_some() {
                return self.set_error(command_usage(command, "<pc>"));
            }
            if is_continue_command {
                self.move_to_pc_from_input(pc, find_next_pc_target);
            } else {
                self.goto_pc_from_input(pc);
            }
        } else if MEMORY_COMMANDS.contains(&command) {
            self.run_buffer_command(command, BufferKind::Memory, parts);
        } else if CALLDATA_COMMANDS.contains(&command) {
            self.run_buffer_command(command, BufferKind::Calldata, parts);
        } else if RETURNDATA_COMMANDS.contains(&command) {
            self.run_buffer_command(command, BufferKind::Returndata, parts);
        } else if STORAGE_COMMANDS.contains(&command) {
            self.run_storage_command(command, StorageSpace::Persistent, parts);
        } else if TRANSIENT_STORAGE_COMMANDS.contains(&command) {
            self.run_storage_command(command, StorageSpace::Transient, parts);
        } else if LINE_COMMANDS.contains(&command) {
            let Some(line) = parts.next() else {
                return self.set_error(command_usage(command, "<line>"));
            };
            if parts.next().is_some() {
                return self.set_error(command_usage(command, "<line>"));
            }
            self.goto_source_line_from_input(line);
        } else if OPCODE_COMMANDS.contains(&command) {
            self.run_pane_command(command, PaneCommand::Opcodes, parts);
        } else if SOURCE_COMMANDS.contains(&command) {
            self.run_pane_command(command, PaneCommand::Source, parts);
        } else if VARIABLES_COMMANDS.contains(&command) {
            self.run_pane_command(command, PaneCommand::Variables, parts);
        } else if STACK_COMMANDS.contains(&command) {
            self.run_pane_command(command, PaneCommand::Stack, parts);
        } else if DATA_COMMANDS.contains(&command) {
            self.run_pane_command(command, PaneCommand::Data, parts);
        } else if HELP_COMMANDS.contains(&command) {
            self.set_info(command_help());
        } else {
            self.set_error(format!("Unknown command `{command}`; try `help`"));
        }
    }

    fn run_buffer_command<'a>(
        &mut self,
        command: &str,
        buffer: BufferKind,
        mut args: impl Iterator<Item = &'a str>,
    ) {
        let Some(offset) = args.next() else {
            self.select_buffer(buffer);
            return;
        };
        if args.next().is_some() {
            return self.set_error(command_usage(command, "<offset>"));
        }
        self.goto_buffer_offset(buffer, offset);
    }

    fn run_storage_command<'a>(
        &mut self,
        command: &str,
        space: StorageSpace,
        mut args: impl Iterator<Item = &'a str>,
    ) {
        let Some(slot) = args.next() else {
            self.select_storage(space);
            return;
        };
        if args.next().is_some() {
            return self.set_error(command_usage(command, "<slot>"));
        }
        self.goto_storage_slot_from_input(slot, space);
    }

    fn select_buffer(&mut self, buffer: BufferKind) {
        self.active_buffer = buffer;
        self.active_storage = None;
        self.draw_memory.current_buf_startline = 0;
        self.set_info(format!("Active buffer: {}", self.active_buffer_name()));
    }

    fn select_storage(&mut self, space: StorageSpace) {
        self.active_storage = Some(space);
        self.draw_memory.current_storage_startline = 0;
        self.set_info(format!("Active data: {}", space.noun()));
    }

    fn goto_source_line_from_input(&mut self, input: &str) {
        let line = match input.parse::<usize>() {
            Ok(line) if line > 0 => line,
            _ => {
                self.set_error(format!(
                    "Invalid source line `{input}`; use a positive decimal line number"
                ));
                return;
            }
        };

        let (source_path, source_line, contract_name) = {
            let (_, source) = match self.src_map() {
                Ok(source) => source,
                Err(err) => {
                    self.set_error(err);
                    return;
                }
            };
            let Some(source_line) = source_line_range(&source.source, line) else {
                let line_count = source.source.lines().count().max(1);
                self.set_error(format!(
                    "Source line {line} is outside {} ({line_count} lines)",
                    source.path.display()
                ));
                return;
            };
            let contract_name = self
                .debugger_context
                .identified_contracts
                .get(self.address())
                .expect("source mapping requires an identified contract")
                .clone();
            (source.path.clone(), source_line, contract_name)
        };

        let sources = &self.debugger_context.contracts_sources;
        let Some(target) = find_step_target(
            self.debug_arena(),
            self.draw_memory.inner_call_index,
            self.current_step,
            |node, step| {
                let Some((source_element, source)) = sources.find_source_mapping(
                    &contract_name,
                    step.pc as u32,
                    node.kind.is_any_create(),
                ) else {
                    return false;
                };
                source.path == source_path
                    && source_line.contains(&(source_element.offset() as usize))
            },
        ) else {
            self.set_error(format!(
                "No opcode mapped to {}:{line} in current contract",
                source_path.display()
            ));
            return;
        };

        let already_at_target = self.draw_memory.inner_call_index == target.node_index
            && self.current_step == target.step_index;
        self.draw_memory.inner_call_index = target.node_index;
        self.current_step = target.step_index;
        self.draw_memory.current_buf_startline = 0;
        self.draw_memory.current_stack_startline = 0;
        self.update_scroll_positions();
        self.key_buffer.clear();

        let pc = self.current_step().pc;
        let action = if already_at_target { "Already at" } else { "Jumped to" };
        self.set_info(format!("{action} {}:{line} at PC 0x{pc:x} ({pc})", source_path.display()));
    }

    fn run_pane_command<'a>(
        &mut self,
        command: &str,
        pane: PaneCommand,
        mut args: impl Iterator<Item = &'a str>,
    ) {
        if args.next().is_some() {
            return self.set_error(command_usage(command, ""));
        }
        let shown = match pane {
            PaneCommand::Opcodes => {
                self.show_opcodes = !self.show_opcodes;
                self.show_opcodes
            }
            PaneCommand::Source => {
                self.show_source = !self.show_source;
                self.show_source
            }
            PaneCommand::Variables => {
                self.show_variables = !self.show_variables;
                self.show_variables
            }
            PaneCommand::Stack => {
                self.show_stack = !self.show_stack;
                self.show_stack
            }
            PaneCommand::Data => {
                self.show_data = !self.show_data;
                self.show_data
            }
        };
        let state = if shown { "shown" } else { "hidden" };
        self.set_info(format!("{} pane: {state}", pane.label()));
    }

    fn goto_storage_slot_from_input(&mut self, input: &str, space: StorageSpace) {
        let slot = match parse_storage_slot(input) {
            Ok(slot) => slot,
            Err(err) => {
                self.set_error(err);
                return;
            }
        };

        let Some(target) = find_storage_target(
            self.debug_arena(),
            self.draw_memory.inner_call_index,
            self.current_step,
            slot,
            space,
        ) else {
            self.set_error(format!(
                "{} slot {} not accessed in current call",
                space.label(),
                hex_u256(slot)
            ));
            return;
        };

        let access = target.access;
        self.draw_memory.inner_call_index = target.node_index;
        self.current_step = access.step_index();
        self.draw_memory.current_buf_startline = 0;
        self.draw_memory.current_stack_startline = 0;
        self.active_storage = Some(space);
        self.draw_memory.current_storage_startline =
            self.storage_accesses(space).get_index_of(&access.slot()).unwrap_or_default();
        self.update_scroll_positions();
        self.key_buffer.clear();
        self.set_info(format!(
            "Jumped to {} at PC 0x{:x} ({})",
            access.describe(),
            access.pc(),
            access.pc()
        ));
    }

    fn apply_buffer_offset(&mut self, offset: usize) {
        self.draw_memory.current_buf_startline = offset / 32;
        self.key_buffer.clear();
        let buffer_name = self.active_buffer_name();
        self.set_info(format!("Jumped to {buffer_name} offset 0x{offset:x} ({offset})"));
    }

    fn set_info(&mut self, text: String) {
        self.status = Some(StatusMessage { kind: StatusKind::Info, text });
    }

    fn set_error(&mut self, text: String) {
        self.status = Some(StatusMessage { kind: StatusKind::Error, text });
    }

    fn handle_breakpoint(&mut self, c: char) {
        self.key_buffer.clear();

        let Some((caller, pc)) = self.debugger_context.breakpoints.get(&c).copied() else {
            self.set_error(format!("Breakpoint '{c}' not found"));
            return;
        };

        let Some((inner_call_index, step_index)) = find_next_step_target(
            self.debug_arena(),
            self.draw_memory.inner_call_index,
            self.current_step,
            |node, step| node.address == caller && step.pc == pc,
        ) else {
            self.set_error(format!("Breakpoint '{c}' target not found in trace"));
            return;
        };

        let already_at_target = self.draw_memory.inner_call_index == inner_call_index
            && self.current_step == step_index;

        self.draw_memory.inner_call_index = inner_call_index;
        self.current_step = step_index;
        self.update_scroll_positions();

        let action = if already_at_target { "Already at" } else { "Jumped to" };
        self.set_info(format!("{action} breakpoint '{c}' at PC 0x{pc:x} ({pc})"));
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> ControlFlow<ExitReason> {
        if self.prompt.is_some() {
            return ControlFlow::Continue(());
        }

        match event.kind {
            MouseEventKind::ScrollUp => self.step_back(),
            MouseEventKind::ScrollDown => self.step(),
            _ => {}
        }

        ControlFlow::Continue(())
    }

    fn step_back(&mut self) {
        if self.current_step > 0 {
            self.current_step -= 1;
        } else if self.draw_memory.inner_call_index > 0 {
            self.draw_memory.inner_call_index -= 1;
            self.current_step = self.n_steps() - 1;
        }
        self.update_scroll_positions();
    }

    fn step(&mut self) {
        if self.current_step < self.n_steps() - 1 {
            self.current_step += 1;
        } else if self.draw_memory.inner_call_index < self.debug_arena().len() - 1 {
            self.draw_memory.inner_call_index += 1;
            self.current_step = 0;
        }
        self.update_scroll_positions();
    }

    fn update_scroll_positions(&mut self) {
        if let Some(stack) = &self.current_step().stack
            && !stack.is_empty()
        {
            self.draw_memory.current_stack_startline =
                self.draw_memory.current_stack_startline.min(stack.len().saturating_sub(1));
        }

        if self.active_buffer == BufferKind::Memory
            && let Some(line) = self.current_memory_write_line()
        {
            self.draw_memory.current_buf_startline = line;
        }

        let buffer_len = self.active_buffer().len();
        if buffer_len > 0 {
            let max_line = buffer_len.div_ceil(32) - 1;
            self.draw_memory.current_buf_startline =
                self.draw_memory.current_buf_startline.min(max_line);
        }
    }

    fn current_memory_write_line(&self) -> Option<usize> {
        let memory_len = self.current_step().memory.as_ref()?.len();

        if self.current_step > 0 {
            let prev_step = &self.debug_steps()[self.current_step - 1];
            if let Some(line) = bounded_memory_write_start_line(prev_step, memory_len) {
                return Some(line);
            }
        }

        bounded_memory_write_start_line(self.current_step(), memory_len)
    }

    /// Calls a closure `f` the number of times specified in the key buffer, and at least once.
    fn repeat(&mut self, mut f: impl FnMut(&mut Self)) {
        for _ in 0..buffer_as_number(&self.key_buffer) {
            f(self);
        }
    }

    fn n_steps(&self) -> usize {
        self.debug_steps().len()
    }

    fn cycle_layout(&mut self) {
        let layout = self.debugger_context.layout.next();
        self.debugger_context.layout = layout;
        self.status = Some(StatusMessage {
            kind: StatusKind::Info,
            text: format!("Debugger layout: {}", layout.as_str()),
        });
    }
}

impl TuiApp for TUIContext<'_> {
    type Exit = ExitReason;

    fn draw(&mut self, frame: &mut Frame<'_>) {
        self.draw_layout(frame);
    }

    fn handle_event(&mut self, event: Event) -> ControlFlow<Self::Exit> {
        TUIContext::handle_event(self, event)
    }
}

/// Grab number from buffer. Used for something like '10k' to move up 10 operations
fn buffer_as_number(s: &str) -> usize {
    const MIN: usize = 1;
    const MAX: usize = 100_000;
    s.parse().unwrap_or(MIN).clamp(MIN, MAX)
}

const fn toggle_state(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

const fn buffer_name(buffer: &BufferKind) -> &'static str {
    match buffer {
        BufferKind::Memory => "memory",
        BufferKind::Calldata => "calldata",
        BufferKind::Returndata => "returndata",
    }
}

const CONTINUE_COMMANDS: &[&str] = &["continue", "cont", "c"];
const PC_COMMANDS: &[&str] = &["pc", "p"];
const MEMORY_COMMANDS: &[&str] = &["mem", "memory"];
const CALLDATA_COMMANDS: &[&str] = &["calldata", "cd"];
const RETURNDATA_COMMANDS: &[&str] = &["returndata", "ret", "rd"];
const STORAGE_COMMANDS: &[&str] = &["storage", "store", "slot"];
const TRANSIENT_STORAGE_COMMANDS: &[&str] = &["transient", "tslot"];
const LINE_COMMANDS: &[&str] = &["line", "ln"];
const OPCODE_COMMANDS: &[&str] = &["opcodes", "opcode", "ops"];
const SOURCE_COMMANDS: &[&str] = &["source", "src"];
const VARIABLES_COMMANDS: &[&str] = &["variables", "vars"];
const STACK_COMMANDS: &[&str] = &["stack"];
const DATA_COMMANDS: &[&str] = &["data"];
const HELP_COMMANDS: &[&str] = &["help", "h"];

#[derive(Clone, Copy)]
enum PaneCommand {
    Opcodes,
    Source,
    Variables,
    Stack,
    Data,
}

impl PaneCommand {
    const fn label(self) -> &'static str {
        match self {
            Self::Opcodes => "Opcodes",
            Self::Source => "Source",
            Self::Variables => "Variables",
            Self::Stack => "Stack",
            Self::Data => "Data",
        }
    }
}

fn command_usage(command: &str, arg: &str) -> String {
    if arg.is_empty() { format!("Usage: :{command}") } else { format!("Usage: :{command} {arg}") }
}

fn command_help() -> String {
    format!(
        "Commands: {} <pc>, {} <pc>, {} [<offset>], {} [<offset>], {} [<offset>], {} [<slot>], {} [<slot>], {} <line>, {}, {}, {}, {}, {}",
        command_aliases(CONTINUE_COMMANDS),
        command_aliases(PC_COMMANDS),
        command_aliases(MEMORY_COMMANDS),
        command_aliases(CALLDATA_COMMANDS),
        command_aliases(RETURNDATA_COMMANDS),
        command_aliases(STORAGE_COMMANDS),
        command_aliases(TRANSIENT_STORAGE_COMMANDS),
        command_aliases(LINE_COMMANDS),
        command_aliases(OPCODE_COMMANDS),
        command_aliases(SOURCE_COMMANDS),
        command_aliases(VARIABLES_COMMANDS),
        command_aliases(STACK_COMMANDS),
        command_aliases(DATA_COMMANDS)
    )
}

fn command_aliases(commands: &[&str]) -> String {
    commands.iter().map(|command| format!(":{command}")).collect::<Vec<_>>().join("/")
}

fn is_buffer_offset_input_char(input: &str, c: char) -> bool {
    if !(c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | ':')) {
        return false;
    }

    let mut next = String::with_capacity(input.len() + c.len_utf8());
    next.push_str(input);
    next.push(c);
    is_buffer_offset_input_prefix(&next)
}

fn is_buffer_offset_input_prefix(input: &str) -> bool {
    if let Some(rest) = input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")) {
        return rest.chars().all(|c| c.is_ascii_hexdigit());
    }

    if let Some(rest) = input.strip_prefix("d:").or_else(|| input.strip_prefix("dec:")) {
        return rest.chars().all(|c| c.is_ascii_digit());
    }

    input.chars().all(|c| c.is_ascii_hexdigit())
        || "d:".starts_with(input)
        || "dec:".starts_with(input)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PcBase {
    Hex,
    Decimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PcCandidate {
    pc: usize,
    base: PcBase,
}

impl PcCandidate {
    fn describe(self) -> String {
        match self.base {
            PcBase::Hex => format!("hex 0x{:x}", self.pc),
            PcBase::Decimal => format!("decimal {}", self.pc),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepTargetScope {
    CurrentNode,
    SameCodeContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StepTarget {
    node_index: usize,
    step_index: usize,
    scope: StepTargetScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StorageTarget {
    node_index: usize,
    access: StorageAccess,
}

fn parse_pc_candidates(input: &str) -> Result<Vec<PcCandidate>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a program counter".to_string());
    }

    if let Some(rest) = input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")) {
        return parse_pc_candidate(rest, 16, PcBase::Hex, input);
    }

    if let Some(rest) = input.strip_prefix("d:").or_else(|| input.strip_prefix("dec:")) {
        return parse_pc_candidate(rest, 10, PcBase::Decimal, input);
    }

    if input.chars().any(|c| c.is_ascii_hexdigit() && c.is_ascii_alphabetic()) {
        return parse_pc_candidate(input, 16, PcBase::Hex, input);
    }

    if input.chars().all(|c| c.is_ascii_digit()) {
        let decimal = parse_pc(input, 10, input)?;
        let hex = parse_pc(input, 16, input)?;
        if decimal == hex {
            return Ok(vec![PcCandidate { pc: decimal, base: PcBase::Decimal }]);
        }
        return Ok(vec![
            PcCandidate { pc: decimal, base: PcBase::Decimal },
            PcCandidate { pc: hex, base: PcBase::Hex },
        ]);
    }

    Err(format!("Invalid PC `{input}`; use 0x2a, 2a, or d:42"))
}

fn parse_pc_candidate(
    input: &str,
    radix: u32,
    base: PcBase,
    original: &str,
) -> Result<Vec<PcCandidate>, String> {
    Ok(vec![PcCandidate { pc: parse_pc(input, radix, original)?, base }])
}

fn parse_pc(input: &str, radix: u32, original: &str) -> Result<usize, String> {
    if input.is_empty() {
        return Err(format!("Invalid PC `{original}`; use 0x2a, 2a, or d:42"));
    }
    usize::from_str_radix(input, radix)
        .map_err(|_| format!("Invalid PC `{original}`; use 0x2a, 2a, or d:42"))
}

fn parse_buffer_offset(input: &str) -> Result<usize, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a buffer offset".to_string());
    }

    let (digits, radix) =
        if let Some(rest) = input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")) {
            (rest, 16)
        } else if let Some(rest) = input.strip_prefix("d:").or_else(|| input.strip_prefix("dec:")) {
            (rest, 10)
        } else {
            (input, 16)
        };

    if digits.is_empty() {
        return Err(invalid_buffer_offset(input));
    }

    usize::from_str_radix(digits, radix).map_err(|_| invalid_buffer_offset(input))
}

fn invalid_buffer_offset(input: &str) -> String {
    format!("Invalid buffer offset `{input}`; use hex 0x20/20 or decimal d:32")
}

fn parse_storage_slot(input: &str) -> Result<U256, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a storage slot".to_string());
    }

    let (digits, radix) =
        if let Some(rest) = input.strip_prefix("0x").or_else(|| input.strip_prefix("0X")) {
            (rest, 16)
        } else if let Some(rest) = input.strip_prefix("d:").or_else(|| input.strip_prefix("dec:")) {
            (rest, 10)
        } else {
            (input, 16)
        };

    let valid_digits = match radix {
        10 => digits.bytes().all(|b| b.is_ascii_digit()),
        16 => digits.bytes().all(|b| b.is_ascii_hexdigit()),
        _ => unreachable!(),
    };
    if digits.is_empty() || !valid_digits {
        return Err(invalid_storage_slot(input));
    }

    U256::from_str_radix(digits, radix).map_err(|_| invalid_storage_slot(input))
}

fn invalid_storage_slot(input: &str) -> String {
    format!("Invalid storage slot `{input}`; use hex 0x20/20 or decimal d:32")
}

fn find_storage_target(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    slot: U256,
    space: StorageSpace,
) -> Option<StorageTarget> {
    let current_node = arena.get(current_node_index)?;
    let trace_node_idx = current_node.trace_node_idx;
    let current_absolute_step = current_node.step_offset.saturating_add(current_step);

    storage_target_at(arena, current_node_index, current_step, slot, space)
        .or_else(|| {
            find_storage_target_after(arena, trace_node_idx, current_absolute_step, slot, space)
        })
        .or_else(|| {
            find_storage_target_before(arena, trace_node_idx, current_absolute_step, slot, space)
        })
}

fn storage_target_at(
    arena: &[DebugNode],
    node_index: usize,
    step_index: usize,
    slot: U256,
    space: StorageSpace,
) -> Option<StorageTarget> {
    let node = arena.get(node_index)?;
    storage_access_at(&node.steps, step_index)
        .filter(|access| access.slot() == slot && access.space() == space)
        .map(|access| StorageTarget { node_index, access })
}

fn find_storage_target_after(
    arena: &[DebugNode],
    trace_node_idx: usize,
    current_absolute_step: usize,
    slot: U256,
    space: StorageSpace,
) -> Option<StorageTarget> {
    let mut best = None;

    for (node_index, node) in arena.iter().enumerate() {
        if node.trace_node_idx != trace_node_idx {
            continue;
        }

        for step_index in 0..node.steps.len() {
            let absolute_step = node.step_offset.saturating_add(step_index);
            if absolute_step <= current_absolute_step {
                continue;
            }

            let Some(access) = storage_access_at(&node.steps, step_index)
                .filter(|access| access.slot() == slot && access.space() == space)
            else {
                continue;
            };

            match best {
                Some((best_absolute_step, _, _)) if absolute_step >= best_absolute_step => {}
                _ => best = Some((absolute_step, node_index, access)),
            }
            break;
        }
    }

    best.map(|(_, node_index, access)| StorageTarget { node_index, access })
}

fn find_storage_target_before(
    arena: &[DebugNode],
    trace_node_idx: usize,
    current_absolute_step: usize,
    slot: U256,
    space: StorageSpace,
) -> Option<StorageTarget> {
    let mut best = None;

    for (node_index, node) in arena.iter().enumerate() {
        if node.trace_node_idx != trace_node_idx {
            continue;
        }

        for step_index in (0..node.steps.len()).rev() {
            let absolute_step = node.step_offset.saturating_add(step_index);
            if absolute_step >= current_absolute_step {
                continue;
            }

            let Some(access) = storage_access_at(&node.steps, step_index)
                .filter(|access| access.slot() == slot && access.space() == space)
            else {
                continue;
            };

            match best {
                Some((best_absolute_step, _, _)) if absolute_step <= best_absolute_step => {}
                _ => best = Some((absolute_step, node_index, access)),
            }
            break;
        }
    }

    best.map(|(_, node_index, access)| StorageTarget { node_index, access })
}

fn find_pc_target(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    pc: usize,
) -> Option<StepTarget> {
    find_step_target(arena, current_node_index, current_step, |_, step| step.pc == pc)
}

fn find_next_pc_target(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    pc: usize,
) -> Option<StepTarget> {
    let current_node = arena.get(current_node_index)?;
    let (node_index, step_index) =
        find_next_step_target(arena, current_node_index, current_step, |node, step| {
            same_code_context(current_node, node) && step.pc == pc
        })?;
    let scope = if node_index == current_node_index {
        StepTargetScope::CurrentNode
    } else {
        StepTargetScope::SameCodeContext
    };

    Some(StepTarget { node_index, step_index, scope })
}

fn find_next_step_target(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    mut matches: impl FnMut(&DebugNode, &CallTraceStep) -> bool,
) -> Option<(usize, usize)> {
    let current_node = arena.get(current_node_index)?;

    if let Some(step_index) = current_node
        .steps
        .iter()
        .enumerate()
        .skip(current_step.saturating_add(1))
        .find_map(|(i, step)| matches(current_node, step).then_some(i))
    {
        return Some((current_node_index, step_index));
    }

    for (node_index, node) in arena.iter().enumerate().skip(current_node_index + 1) {
        if let Some(step_index) = node.steps.iter().position(|step| matches(node, step)) {
            return Some((node_index, step_index));
        }
    }

    for (node_index, node) in arena.iter().enumerate().take(current_node_index) {
        if let Some(step_index) = node.steps.iter().position(|step| matches(node, step)) {
            return Some((node_index, step_index));
        }
    }

    current_node
        .steps
        .iter()
        .enumerate()
        .take(current_step.saturating_add(1))
        .find_map(|(i, step)| matches(current_node, step).then_some((current_node_index, i)))
}

fn find_step_target(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    mut matches: impl FnMut(&DebugNode, &CallTraceStep) -> bool,
) -> Option<StepTarget> {
    let current_node = arena.get(current_node_index)?;

    if let Some(step_index) = find_step_in_current_node(current_node, current_step, &mut matches) {
        return Some(StepTarget {
            node_index: current_node_index,
            step_index,
            scope: StepTargetScope::CurrentNode,
        });
    }

    for (node_index, node) in arena.iter().enumerate().skip(current_node_index + 1) {
        if same_code_context(current_node, node)
            && let Some(step_index) = node.steps.iter().position(|step| matches(node, step))
        {
            return Some(StepTarget {
                node_index,
                step_index,
                scope: StepTargetScope::SameCodeContext,
            });
        }
    }

    for (node_index, node) in arena.iter().enumerate().take(current_node_index).rev() {
        if same_code_context(current_node, node)
            && let Some(step_index) = node.steps.iter().rposition(|step| matches(node, step))
        {
            return Some(StepTarget {
                node_index,
                step_index,
                scope: StepTargetScope::SameCodeContext,
            });
        }
    }

    None
}

fn find_step_in_current_node(
    node: &DebugNode,
    current_step: usize,
    matches: &mut impl FnMut(&DebugNode, &CallTraceStep) -> bool,
) -> Option<usize> {
    if node.steps.get(current_step).is_some_and(|step| matches(node, step)) {
        return Some(current_step);
    }

    node.steps
        .iter()
        .enumerate()
        .skip(current_step.saturating_add(1))
        .find_map(|(i, step)| matches(node, step).then_some(i))
        .or_else(|| {
            node.steps[..current_step.min(node.steps.len())]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, step)| matches(node, step).then_some(i))
        })
}

fn source_line_range(source: &str, line: usize) -> Option<std::ops::Range<usize>> {
    if line == 0 {
        return None;
    }

    let mut start = 0;
    for _ in 1..line {
        start += source.get(start..)?.find('\n')? + 1;
    }
    if start >= source.len() {
        return None;
    }
    let end = source[start..].find('\n').map_or(source.len(), |offset| start + offset + 1);
    Some(start..end)
}

fn same_code_context(a: &DebugNode, b: &DebugNode) -> bool {
    a.address == b.address && a.kind.is_any_create() == b.kind.is_any_create()
}

fn pc_exists_outside_code_context(arena: &[DebugNode], current: &DebugNode, pc: usize) -> bool {
    arena.iter().any(|node| {
        !same_code_context(current, node) && node.steps.iter().any(|step| step.pc == pc)
    })
}

fn find_opcode_match(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    query: &str,
    direction: SearchDirection,
) -> Option<(usize, usize)> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }

    let current_node = arena.get(current_node_index)?;
    let trace_node_idx = current_node.trace_node_idx;
    let current_absolute_step = current_node.step_offset.saturating_add(current_step);
    let steps = || {
        arena.iter().enumerate().filter(|(_, node)| node.trace_node_idx == trace_node_idx).flat_map(
            |(node_index, node)| {
                node.steps.iter().enumerate().map(move |(step_index, step)| {
                    (node_index, step_index, node.step_offset.saturating_add(step_index), step)
                })
            },
        )
    };
    let target = match direction {
        SearchDirection::Forward => steps()
            .filter(|(_, _, absolute_step, _)| *absolute_step > current_absolute_step)
            .chain(
                steps().filter(|(_, _, absolute_step, _)| *absolute_step <= current_absolute_step),
            )
            .find(|(_, _, _, step)| pretty_opcode(step).to_ascii_lowercase().contains(&needle)),
        SearchDirection::Backward => steps()
            .rev()
            .filter(|(_, _, absolute_step, _)| *absolute_step < current_absolute_step)
            .chain(
                steps()
                    .rev()
                    .filter(|(_, _, absolute_step, _)| *absolute_step >= current_absolute_step),
            )
            .find(|(_, _, _, step)| pretty_opcode(step).to_ascii_lowercase().contains(&needle)),
    };
    target.map(|(node_index, step_index, _, _)| (node_index, step_index))
}

pub(super) fn pretty_opcode(step: &CallTraceStep) -> String {
    let mut buf = String::new();
    write_pretty_opcode(&mut buf, step);
    buf
}

pub(super) fn write_pretty_opcode(buf: &mut String, step: &CallTraceStep) {
    if let Some(immediate) = step.immediate_bytes.as_ref().filter(|b| !b.is_empty()) {
        write!(buf, "{}(0x{})", step.op, hex::encode(immediate)).unwrap();
    } else {
        write!(buf, "{}", step.op).unwrap();
    }
}

fn memory_write_start_line(step: &CallTraceStep) -> Option<usize> {
    let stack = step.stack.as_ref()?;
    let access = get_buffer_accesses(step.op.get(), stack)?.write?;
    if access.len == 0 {
        return None;
    }
    Some(access.offset / 32)
}

fn bounded_memory_write_start_line(step: &CallTraceStep, memory_len: usize) -> Option<usize> {
    let line = memory_write_start_line(step)?;
    (line < memory_len.div_ceil(32)).then_some(line)
}

fn is_jump(step: &CallTraceStep, prev: &CallTraceStep) -> bool {
    if !matches!(prev.op, OpCode::JUMP | OpCode::JUMPI) {
        return false;
    }

    let immediate_len = prev.immediate_bytes.as_ref().map_or(0, |b| b.len());

    step.pc != prev.pc + 1 + immediate_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::storage::storage_values;
    use alloy_primitives::{Bytes, keccak256, map::AddressHashMap};
    use foundry_common::slot_identifier::{ENCODING_BYTES, SlotIdentifier};
    use foundry_compilers::artifacts::{Storage, StorageLayout, StorageType, sourcemap::Parser};
    use foundry_evm_core::{Breakpoints, ic::PcIcMap};
    use foundry_evm_traces::debug::{ArtifactData, ContractSources};
    use revm::interpreter::InstructionResult;
    use revm_inspectors::tracing::types::{StorageChange, StorageChangeReason};
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

    fn step(pc: usize) -> CallTraceStep {
        step_with_stack(pc, OpCode::STOP, &[])
    }

    fn step_with_immediate(pc: usize, op: OpCode, immediate: &'static [u8]) -> CallTraceStep {
        CallTraceStep {
            immediate_bytes: Some(Bytes::from_static(immediate)),
            ..step_with_stack(pc, op, &[])
        }
    }

    fn step_with_stack(pc: usize, op: OpCode, stack: &[usize]) -> CallTraceStep {
        CallTraceStep {
            pc,
            op,
            stack: (!stack.is_empty()).then(|| {
                stack.iter().copied().map(U256::from).collect::<Vec<_>>().into_boxed_slice()
            }),
            push_stack: None,
            memory: None,
            returndata: Bytes::new(),
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_used: 0,
            gas_cost: 0,
            storage_change: None,
            status: Some(InstructionResult::Stop),
            immediate_bytes: None,
            decoded: None,
        }
    }

    fn node(address: Address, kind: CallKind, pcs: &[usize]) -> DebugNode {
        DebugNode::new(
            address,
            kind,
            pcs.iter().copied().map(step).collect(),
            Bytes::new(),
            0,
            None,
        )
    }

    fn context_with_arena(arena: Vec<DebugNode>) -> DebuggerContext {
        DebuggerContext {
            debug_arena: arena,
            stats: None,
            identified_contracts: Default::default(),
            slot_identifiers: None,
            contracts_sources: ContractSources::default(),
            breakpoints: Breakpoints::default(),
            layout: Default::default(),
        }
    }

    fn context_with_source_lines(address: Address) -> DebuggerContext {
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[0, 1, 2])]);
        context.identified_contracts.insert(address, "Test".to_string());

        let build_id = "test-build".to_string();
        context.contracts_sources.sources_by_id.entry(build_id.clone()).or_default().insert(
            0,
            Arc::new(SourceData {
                source: Arc::new("line one\nline two\nline three\n".to_string()),
                language: Default::default(),
                path: PathBuf::from("src/Test.sol"),
                contract_definitions: Vec::new(),
                debug_scopes: Vec::new(),
            }),
        );
        context.contracts_sources.artifacts_by_name.insert(
            "Test".to_string(),
            vec![ArtifactData {
                source_map: None,
                source_map_runtime: Some(
                    Parser::new("0:8:0;9:8:0;18:10:0").collect::<Result<_, _>>().unwrap(),
                ),
                pc_ic_map: None,
                pc_ic_map_runtime: Some(PcIcMap::new(&[0x00, 0x00, 0x00])),
                build_id,
                file_id: 0,
            }],
        );
        context
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn layout_shortcut_cycles_only_concrete_layouts() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        assert_eq!(tui.debugger_context.layout, DebuggerLayout::Auto);

        let _ = tui.handle_key_event(key(KeyCode::Char('l')));
        assert_eq!(tui.debugger_context.layout, DebuggerLayout::Horizontal);
        assert_eq!(tui.status.as_ref().unwrap().text, "Debugger layout: horizontal");

        let _ = tui.handle_key_event(key(KeyCode::Char('l')));
        assert_eq!(tui.debugger_context.layout, DebuggerLayout::Vertical);
        assert_eq!(tui.status.as_ref().unwrap().text, "Debugger layout: vertical");

        let _ = tui.handle_key_event(key(KeyCode::Char('l')));
        assert_eq!(tui.debugger_context.layout, DebuggerLayout::Horizontal);
        assert_eq!(tui.status.as_ref().unwrap().text, "Debugger layout: horizontal");
    }

    #[test]
    fn view_shortcuts_report_status() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        assert_eq!(tui.active_buffer, BufferKind::Memory);
        let _ = tui.handle_key_event(key(KeyCode::Char('b')));
        assert_eq!(tui.active_buffer, BufferKind::Calldata);
        assert_eq!(tui.status.as_ref().unwrap().text, "Active buffer: calldata");

        let _ = tui.handle_key_event(key(KeyCode::Char('t')));
        assert!(tui.stack_labels);
        assert_eq!(tui.status.as_ref().unwrap().text, "Stack labels: on");
        let _ = tui.handle_key_event(key(KeyCode::Char('t')));
        assert!(!tui.stack_labels);
        assert_eq!(tui.status.as_ref().unwrap().text, "Stack labels: off");

        let _ = tui.handle_key_event(key(KeyCode::Char('m')));
        assert!(tui.buf_utf);
        assert_eq!(tui.status.as_ref().unwrap().text, "UTF-8 decoding: on");
        let _ = tui.handle_key_event(key(KeyCode::Char('m')));
        assert!(!tui.buf_utf);
        assert_eq!(tui.status.as_ref().unwrap().text, "UTF-8 decoding: off");

        let _ = tui.handle_key_event(key(KeyCode::Char('h')));
        assert!(!tui.show_shortcuts);
        assert_eq!(tui.status.as_ref().unwrap().text, "Shortcut help: hidden");
        let _ = tui.handle_key_event(key(KeyCode::Char('h')));
        assert!(tui.show_shortcuts);
        assert_eq!(tui.status.as_ref().unwrap().text, "Shortcut help: shown");
    }

    #[test]
    fn previous_call_shortcut_respects_root_boundary() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![
            node(address, CallKind::Call, &[1, 2]),
            node(address, CallKind::Call, &[3]),
        ]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('c')));
        assert_eq!((tui.draw_memory.inner_call_index, tui.current_step), (0, 0));

        tui.draw_memory.inner_call_index = 1;
        let _ = tui.handle_key_event(key(KeyCode::Char('c')));
        assert_eq!((tui.draw_memory.inner_call_index, tui.current_step), (0, 1));
    }

    #[test]
    fn breakpoint_shortcut_cycles_trace_hits() {
        let address = Address::repeat_byte(1);
        let other = Address::repeat_byte(2);
        let mut context = context_with_arena(vec![
            node(other, CallKind::Call, &[1]),
            node(address, CallKind::Call, &[42, 7, 42]),
            node(address, CallKind::Call, &[8, 42]),
        ]);
        context.breakpoints.insert('a', (address, 42));
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));

        assert_eq!(tui.draw_memory.inner_call_index, 1);
        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Jumped to breakpoint 'a' at PC 0x2a (42)");

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        assert_eq!((tui.draw_memory.inner_call_index, tui.current_step), (1, 2));

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        assert_eq!((tui.draw_memory.inner_call_index, tui.current_step), (2, 1));

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        assert_eq!((tui.draw_memory.inner_call_index, tui.current_step), (1, 0));
    }

    #[test]
    fn breakpoint_shortcut_reports_missing_key() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('z')));

        assert_eq!(tui.draw_memory.inner_call_index, 0);
        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Breakpoint 'z' not found");
    }

    #[test]
    fn breakpoint_shortcut_reports_missing_trace_target() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        context.breakpoints.insert('a', (address, 42));
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));

        assert_eq!(tui.draw_memory.inner_call_index, 0);
        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Breakpoint 'a' target not found in trace");
    }

    #[test]
    fn parses_prefixed_hex_pc() {
        assert_eq!(
            parse_pc_candidates("0x2a").unwrap(),
            vec![PcCandidate { pc: 42, base: PcBase::Hex }]
        );
        assert_eq!(
            parse_pc_candidates("0X2A").unwrap(),
            vec![PcCandidate { pc: 42, base: PcBase::Hex }]
        );
    }

    #[test]
    fn parses_bare_hex_pc_with_letters() {
        assert_eq!(
            parse_pc_candidates("2a").unwrap(),
            vec![PcCandidate { pc: 42, base: PcBase::Hex }]
        );
    }

    #[test]
    fn parses_explicit_decimal_pc() {
        assert_eq!(
            parse_pc_candidates("d:42").unwrap(),
            vec![PcCandidate { pc: 42, base: PcBase::Decimal }]
        );
        assert_eq!(
            parse_pc_candidates("dec:42").unwrap(),
            vec![PcCandidate { pc: 42, base: PcBase::Decimal }]
        );
    }

    #[test]
    fn parses_bare_digits_as_decimal_and_hex_candidates() {
        assert_eq!(
            parse_pc_candidates("10").unwrap(),
            vec![
                PcCandidate { pc: 10, base: PcBase::Decimal },
                PcCandidate { pc: 16, base: PcBase::Hex },
            ]
        );
        assert_eq!(
            parse_pc_candidates("9").unwrap(),
            vec![PcCandidate { pc: 9, base: PcBase::Decimal }]
        );
    }

    #[test]
    fn rejects_invalid_pc_input() {
        assert!(parse_pc_candidates("").is_err());
        assert!(parse_pc_candidates("0x").is_err());
        assert!(parse_pc_candidates("xyz").is_err());
        assert!(parse_pc_candidates("184467440737095516160").is_err());
    }

    #[test]
    fn parses_buffer_offsets_as_visible_hex_labels() {
        assert_eq!(parse_buffer_offset("0x20").unwrap(), 32);
        assert_eq!(parse_buffer_offset("d:32").unwrap(), 32);
        assert_eq!(parse_buffer_offset("dec:32").unwrap(), 32);
        assert_eq!(parse_buffer_offset("20").unwrap(), 32);
        assert_eq!(parse_buffer_offset("2a").unwrap(), 42);
        assert_eq!(parse_buffer_offset("a").unwrap(), 10);

        assert_eq!(parse_buffer_offset("").unwrap_err(), "Enter a buffer offset");
        assert_eq!(
            parse_buffer_offset("0x").unwrap_err(),
            "Invalid buffer offset `0x`; use hex 0x20/20 or decimal d:32"
        );
        assert_eq!(
            parse_buffer_offset("2x3").unwrap_err(),
            "Invalid buffer offset `2x3`; use hex 0x20/20 or decimal d:32"
        );
    }

    #[test]
    fn parses_storage_slots_as_visible_hex_labels() {
        assert_eq!(parse_storage_slot("0x20").unwrap(), U256::from(32));
        assert_eq!(parse_storage_slot("d:32").unwrap(), U256::from(32));
        assert_eq!(parse_storage_slot("dec:32").unwrap(), U256::from(32));
        assert_eq!(parse_storage_slot("20").unwrap(), U256::from(32));
        assert_eq!(parse_storage_slot("2a").unwrap(), U256::from(42));
        assert_eq!(parse_storage_slot("a").unwrap(), U256::from(10));

        assert_eq!(parse_storage_slot("").unwrap_err(), "Enter a storage slot");
        assert_eq!(
            parse_storage_slot("0x").unwrap_err(),
            "Invalid storage slot `0x`; use hex 0x20/20 or decimal d:32"
        );
        assert_eq!(
            parse_storage_slot("2x3").unwrap_err(),
            "Invalid storage slot `2x3`; use hex 0x20/20 or decimal d:32"
        );
        assert_eq!(
            parse_storage_slot("1_0").unwrap_err(),
            "Invalid storage slot `1_0`; use hex 0x20/20 or decimal d:32"
        );
        assert_eq!(
            parse_storage_slot("_").unwrap_err(),
            "Invalid storage slot `_`; use hex 0x20/20 or decimal d:32"
        );
        assert_eq!(
            parse_storage_slot("0x_").unwrap_err(),
            "Invalid storage slot `0x_`; use hex 0x20/20 or decimal d:32"
        );
        assert_eq!(
            parse_storage_slot("d:_").unwrap_err(),
            "Invalid storage slot `d:_`; use hex 0x20/20 or decimal d:32"
        );
    }

    #[test]
    fn filters_buffer_offset_input_to_parser_prefixes() {
        assert!(is_buffer_offset_input_char("", '0'));
        assert!(is_buffer_offset_input_char("0", 'x'));
        assert!(is_buffer_offset_input_char("0x", '2'));
        assert!(is_buffer_offset_input_char("2", 'a'));
        assert!(is_buffer_offset_input_char("d", ':'));
        assert!(is_buffer_offset_input_char("dec", ':'));
        assert!(is_buffer_offset_input_char("dec:", '3'));

        assert!(!is_buffer_offset_input_char("", 'x'));
        assert!(!is_buffer_offset_input_char("2", 'x'));
        assert!(!is_buffer_offset_input_char("1", ':'));
        assert!(!is_buffer_offset_input_char("DEC", ':'));
        assert!(!is_buffer_offset_input_char("d:", 'a'));
    }

    #[test]
    fn finds_pc_in_current_node() {
        let address = Address::repeat_byte(1);
        let arena = vec![node(address, CallKind::Call, &[1, 2, 3])];

        assert_eq!(
            find_pc_target(&arena, 0, 0, 3),
            Some(StepTarget { node_index: 0, step_index: 2, scope: StepTargetScope::CurrentNode })
        );
    }

    #[test]
    fn repeated_pc_stays_current_then_prefers_next_then_previous() {
        let address = Address::repeat_byte(1);
        let arena = vec![node(address, CallKind::Call, &[1, 2, 3, 2])];

        assert_eq!(find_pc_target(&arena, 0, 1, 2).unwrap().step_index, 1);
        assert_eq!(find_pc_target(&arena, 0, 0, 2).unwrap().step_index, 1);
        assert_eq!(find_pc_target(&arena, 0, 3, 2).unwrap().step_index, 3);
        assert_eq!(find_pc_target(&arena, 0, 2, 2).unwrap().step_index, 3);
    }

    #[test]
    fn searches_later_then_earlier_same_code_context() {
        let address = Address::repeat_byte(1);
        let arena = vec![
            node(address, CallKind::Call, &[1]),
            node(address, CallKind::Call, &[2]),
            node(address, CallKind::Call, &[3]),
        ];

        assert_eq!(find_pc_target(&arena, 1, 0, 3).unwrap().node_index, 2);
        assert_eq!(find_pc_target(&arena, 1, 0, 1).unwrap().node_index, 0);
    }

    #[test]
    fn does_not_search_different_address_or_creation_context() {
        let address = Address::repeat_byte(1);
        let other = Address::repeat_byte(2);
        let arena = vec![
            node(address, CallKind::Call, &[1]),
            node(other, CallKind::Call, &[2]),
            node(address, CallKind::Create, &[3]),
        ];

        assert!(find_pc_target(&arena, 0, 0, 2).is_none());
        assert!(find_pc_target(&arena, 0, 0, 3).is_none());
        assert!(pc_exists_outside_code_context(&arena, &arena[0], 2));
        assert!(pc_exists_outside_code_context(&arena, &arena[0], 3));
    }

    #[test]
    fn goto_resolves_unambiguous_bare_digits_and_reports_ambiguity() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[10, 16, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.goto_pc_from_input("2a");
        assert_eq!(tui.current_step, 2);
        assert_eq!(tui.status.as_ref().unwrap().kind, StatusKind::Info);

        tui.current_step = 0;
        tui.goto_pc_from_input("10");
        assert_eq!(tui.current_step, 0);
        assert!(tui.status.as_ref().unwrap().text.contains("Ambiguous PC"));

        tui.goto_pc_from_input("d:10");
        assert_eq!(tui.current_step, 0);
        assert_eq!(tui.status.as_ref().unwrap().kind, StatusKind::Info);
    }

    #[test]
    fn goto_reports_pc_in_other_contract_without_moving() {
        let address = Address::repeat_byte(1);
        let other = Address::repeat_byte(2);
        let mut context = context_with_arena(vec![
            node(address, CallKind::Call, &[1]),
            node(other, CallKind::Call, &[42]),
        ]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.goto_pc_from_input("2a");
        assert_eq!(tui.draw_memory.inner_call_index, 0);
        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert!(status.text.contains("exists in another contract"));
    }

    #[test]
    fn goto_reports_ambiguous_input_in_other_contract_without_choosing_first_candidate() {
        let address = Address::repeat_byte(1);
        let other = Address::repeat_byte(2);
        let mut context = context_with_arena(vec![
            node(address, CallKind::Call, &[1]),
            node(other, CallKind::Call, &[16]),
        ]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.goto_pc_from_input("10");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert!(status.text.starts_with("PC `10` not found"));
        assert!(status.text.contains("exists in another contract"));
    }

    #[test]
    fn prompts_preserve_character_filters_and_block_navigation() {
        for (shortcut, expected) in [
            ('p', Prompt::Pc("a".into())),
            ('o', Prompt::BufferOffset("a".into())),
            (':', Prompt::Command("aé".into())),
            ('/', Prompt::OpcodeSearch("aé\n".into())),
        ] {
            let address = Address::repeat_byte(1);
            let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
            let mut tui = TUIContext::new(&mut context);
            tui.init();

            let _ = tui.handle_key_event(key(KeyCode::Char(shortcut)));
            for c in ['a', 'é', '\n'] {
                let _ = tui.handle_key_event(key(KeyCode::Char(c)));
            }
            let _ = tui.handle_key_event(ctrl_key(KeyCode::Char('c')));
            let _ = tui.handle_key_event(key(KeyCode::Down));
            let _ = tui.handle_mouse_event(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            });
            assert_eq!(tui.prompt, Some(expected));
            assert_eq!(tui.current_step, 0);

            let _ = tui.handle_key_event(key(KeyCode::Esc));
            assert_eq!(tui.prompt, None);
            assert_eq!(tui.current_step, 0);
            assert_eq!(tui.status, None);
            let _ = tui.handle_key_event(key(KeyCode::Down));
            assert_eq!(tui.current_step, 1);
        }
    }

    #[test]
    fn pc_input_mode_handles_keys_and_blocks_normal_commands() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        assert!(matches!(tui.handle_key_event(key(KeyCode::Char('p'))), ControlFlow::Continue(())));
        assert_eq!(tui.prompt, Some(Prompt::Pc(String::new())));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.prompt, Some(Prompt::Pc(String::new())));
        assert_eq!(tui.current_step, 0);

        let _ = tui.handle_key_event(key(KeyCode::Char('2')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        assert_eq!(tui.prompt, Some(Prompt::Pc("2a".into())));

        let _ = tui.handle_key_event(key(KeyCode::Backspace));
        assert_eq!(tui.prompt, Some(Prompt::Pc("2".into())));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.status.as_ref().unwrap().kind, StatusKind::Info);
    }

    #[test]
    fn pc_input_escape_cancels_without_moving() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('p')));
        let _ = tui.handle_key_event(key(KeyCode::Char('2')));
        let _ = tui.handle_key_event(key(KeyCode::Esc));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.current_step, 0);
        assert_eq!(tui.status, None);
    }

    #[test]
    fn command_input_mode_handles_keys_and_blocks_normal_commands() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        assert!(matches!(tui.handle_key_event(key(KeyCode::Char(':'))), ControlFlow::Continue(())));
        assert_eq!(tui.prompt, Some(Prompt::Command(String::new())));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.prompt, Some(Prompt::Command("q".into())));
        assert_eq!(tui.current_step, 0);

        let _ = tui.handle_key_event(key(KeyCode::Backspace));
        for c in "continue 2a".chars() {
            let _ = tui.handle_key_event(key(KeyCode::Char(c)));
        }
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.current_step, 1);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Jumped to PC 0x2a (42) in current trace");
    }

    #[test]
    fn command_prompt_jumps_to_named_buffer_offset() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1)],
            Bytes::from(vec![0; 96]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("calldata 40");

        assert_eq!(tui.active_buffer, BufferKind::Calldata);
        assert_eq!(tui.draw_memory.current_buf_startline, 2);
        assert_eq!(tui.status.as_ref().unwrap().text, "Jumped to calldata offset 0x40 (64)");
    }

    #[test]
    fn labels_long_string_data_slots_written_before_length() {
        let address = Address::repeat_byte(1);
        let type_id = "t_string_storage".to_string();
        let identifier = SlotIdentifier::new(Arc::new(StorageLayout {
            storage: vec![Storage {
                ast_id: 1,
                contract: "StorageTest".to_string(),
                label: "text".to_string(),
                offset: 0,
                slot: "0".to_string(),
                storage_type: type_id.clone(),
            }],
            types: BTreeMap::from([(
                type_id,
                StorageType {
                    encoding: ENCODING_BYTES.to_string(),
                    key: None,
                    label: "string".to_string(),
                    number_of_bytes: "32".to_string(),
                    value: None,
                    other: BTreeMap::new(),
                },
            )]),
        }));
        let data_slot = U256::from_be_bytes(keccak256(B256::ZERO).0);
        let mut data_access = step(1);
        data_access.storage_change = Some(Box::new(StorageChange {
            key: data_slot,
            value: U256::ZERO,
            had_value: None,
            reason: StorageChangeReason::SSTORE,
        }));
        let mut base_access = step(2);
        base_access.storage_change = Some(Box::new(StorageChange {
            key: U256::ZERO,
            value: U256::from(40 * 2 + 1),
            had_value: None,
            reason: StorageChangeReason::SSTORE,
        }));
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![data_access, base_access],
            Bytes::new(),
            0,
            None,
        )]);
        context.slot_identifiers = Some(AddressHashMap::from_iter([(address, identifier)]));
        let mut tui = TUIContext::new(&mut context);
        tui.current_step = 0;
        let accesses = tui.storage_accesses(StorageSpace::Persistent);
        let values = storage_values(&accesses);
        let next_values = tui.next_storage_write_values();

        assert_eq!(
            tui.storage_label(
                StorageSpace::Persistent,
                data_slot,
                Some(&values),
                Some(&next_values),
            )
            .as_deref(),
            Some("text[0]")
        );
    }

    #[test]
    fn command_prompt_jumps_to_storage_slot_access() {
        let address = Address::repeat_byte(1);
        let mut first_store = step(2);
        first_store.storage_change = Some(Box::new(StorageChange {
            key: U256::ZERO,
            value: U256::from(7),
            had_value: None,
            reason: StorageChangeReason::SSTORE,
        }));
        let mut store = step(42);
        store.storage_change = Some(Box::new(StorageChange {
            key: U256::from(1),
            value: U256::from(42),
            had_value: Some(U256::from(7)),
            reason: StorageChangeReason::SSTORE,
        }));
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1), first_store, store],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("storage 1");

        assert_eq!(tui.current_step, 2);
        assert_eq!(tui.active_storage, Some(StorageSpace::Persistent));
        assert_eq!(tui.storage_accesses(StorageSpace::Persistent).len(), 2);
        assert_eq!(
            tui.status.as_ref().unwrap().text,
            "Jumped to storage SSTORE slot 0x1: 0x7 -> 0x2a at PC 0x2a (42)"
        );
    }

    #[test]
    fn command_prompt_jumps_to_transient_storage_slot_access() {
        let address = Address::repeat_byte(1);
        let steps = vec![step(1), step_with_stack(42, OpCode::TSTORE, &[0xbeef, 0x2a])];
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            steps,
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("transient 2a");

        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.active_storage, Some(StorageSpace::Transient));
        assert_eq!(
            tui.status.as_ref().unwrap().text,
            "Jumped to transient storage TSTORE slot 0x2a = 0xbeef at PC 0x2a (42)"
        );

        tui.run_command_from_input("storage 2a");
        assert_eq!(tui.current_step, 1);
        assert_eq!(
            tui.status.as_ref().unwrap().text,
            "Storage slot 0x2a not accessed in current call"
        );
    }

    #[test]
    fn command_prompt_searches_storage_across_split_call_segments() {
        let address = Address::repeat_byte(1);
        let mut store = step(42);
        store.storage_change = Some(Box::new(StorageChange {
            key: U256::from(1),
            value: U256::from(42),
            had_value: None,
            reason: StorageChangeReason::SSTORE,
        }));

        let mut first_store = step(1);
        first_store.storage_change = Some(Box::new(StorageChange {
            key: U256::ZERO,
            value: U256::from(7),
            had_value: None,
            reason: StorageChangeReason::SSTORE,
        }));
        let mut first =
            DebugNode::new(address, CallKind::Call, vec![first_store], Bytes::new(), 0, None);
        first.trace_node_idx = 7;
        first.step_offset = 0;

        let mut child_store = step(2);
        child_store.storage_change = Some(Box::new(StorageChange {
            key: U256::from(9),
            value: U256::from(99),
            had_value: None,
            reason: StorageChangeReason::SSTORE,
        }));
        let mut child = DebugNode::new(
            Address::repeat_byte(2),
            CallKind::Call,
            vec![child_store],
            Bytes::new(),
            0,
            None,
        );
        child.trace_node_idx = 8;
        child.step_offset = 1;

        let mut second =
            DebugNode::new(address, CallKind::Call, vec![store], Bytes::new(), 0, None);
        second.trace_node_idx = 7;
        second.step_offset = 2;

        let mut context = context_with_arena(vec![first, child, second]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("storage 1");

        assert_eq!(tui.draw_memory.inner_call_index, 2);
        assert_eq!(tui.current_step, 0);
        let accesses = tui.storage_accesses(StorageSpace::Persistent);
        assert_eq!(accesses.len(), 2);
        assert!(!accesses.contains_key(&U256::from(9)));
        assert_eq!(
            tui.status.as_ref().unwrap().text,
            "Jumped to storage SSTORE slot 0x1 = 0x2a at PC 0x2a (42)"
        );
    }

    #[test]
    fn command_prompt_finds_warm_sload_from_stack_snapshots() {
        let address = Address::repeat_byte(1);
        let steps =
            vec![step_with_stack(1, OpCode::SLOAD, &[1]), step_with_stack(2, OpCode::STOP, &[42])];
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            steps,
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("store 1");

        assert_eq!(tui.current_step, 0);
        assert_eq!(
            tui.status.as_ref().unwrap().text,
            "Jumped to storage SLOAD slot 0x1 = 0x2a at PC 0x1 (1)"
        );
    }

    #[test]
    fn command_prompt_finds_warm_sstore_from_stack_snapshot() {
        let address = Address::repeat_byte(1);
        let steps = vec![step_with_stack(42, OpCode::SSTORE, &[42, 1])];
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            steps,
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("slot 1");

        assert_eq!(tui.current_step, 0);
        assert_eq!(
            tui.status.as_ref().unwrap().text,
            "Jumped to storage SSTORE slot 0x1 = 0x2a at PC 0x2a (42)"
        );
    }

    #[test]
    fn command_prompt_ignores_failed_sstore_stack_snapshot() {
        let address = Address::repeat_byte(1);
        let mut store = step_with_stack(42, OpCode::SSTORE, &[42, 1]);
        store.status = Some(InstructionResult::StateChangeDuringStaticCall);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![store],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("slot 1");

        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Storage slot 0x1 not accessed in current call");
    }

    #[test]
    fn command_prompt_ignores_failed_sstore_storage_change() {
        let address = Address::repeat_byte(1);
        let mut store = step_with_stack(42, OpCode::SSTORE, &[42, 1]);
        store.storage_change = Some(Box::new(StorageChange {
            key: U256::from(1),
            value: U256::from(42),
            had_value: Some(U256::ZERO),
            reason: StorageChangeReason::SSTORE,
        }));
        store.status = Some(InstructionResult::OutOfGas);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![store],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("slot 1");

        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Storage slot 0x1 not accessed in current call");
    }

    #[test]
    fn command_prompt_accepts_optional_leading_colon() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input(":pc 2a");

        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.status.as_ref().unwrap().text, "Jumped to PC 0x2a (42) in current trace");
    }

    #[test]
    fn command_prompt_continues_to_next_pc_hit() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42, 2, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.current_step = 1;

        tui.run_command_from_input("pc 2a");
        assert_eq!(tui.current_step, 1);

        tui.run_command_from_input("continue 2a");
        assert_eq!(tui.current_step, 3);
    }

    #[test]
    fn command_prompt_jumps_to_source_line() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_source_lines(address);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("line 2");

        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.status.as_ref().unwrap().text, "Jumped to src/Test.sol:2 at PC 0x1 (1)");
    }

    #[test]
    fn command_prompt_reports_help_and_usage_errors() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("help");
        assert_eq!(tui.status.as_ref().unwrap().kind, StatusKind::Info);
        let help = &tui.status.as_ref().unwrap().text;
        for commands in [
            CONTINUE_COMMANDS,
            PC_COMMANDS,
            MEMORY_COMMANDS,
            CALLDATA_COMMANDS,
            RETURNDATA_COMMANDS,
            STORAGE_COMMANDS,
            TRANSIENT_STORAGE_COMMANDS,
            LINE_COMMANDS,
            OPCODE_COMMANDS,
            SOURCE_COMMANDS,
            VARIABLES_COMMANDS,
            STACK_COMMANDS,
            DATA_COMMANDS,
        ] {
            assert!(help.contains(&command_aliases(commands)));
        }

        tui.run_command_from_input("mem");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Active buffer: memory");

        tui.run_command_from_input("store");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Active data: storage");

        tui.run_command_from_input("store 1 2");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Usage: :store <slot>");

        tui.run_command_from_input("line");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Usage: :line <line>");
    }

    #[test]
    fn command_prompt_toggles_panes() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        tui.run_command_from_input("opcodes");
        assert!(!tui.show_opcodes);
        assert_eq!(tui.status.as_ref().unwrap().text, "Opcodes pane: hidden");

        tui.run_command_from_input(":ops");
        assert!(tui.show_opcodes);
        assert_eq!(tui.status.as_ref().unwrap().text, "Opcodes pane: shown");

        tui.run_command_from_input("source");
        assert!(!tui.show_source);
        assert_eq!(tui.status.as_ref().unwrap().text, "Source pane: hidden");

        tui.run_command_from_input(":src");
        assert!(tui.show_source);
        assert_eq!(tui.status.as_ref().unwrap().text, "Source pane: shown");

        tui.run_command_from_input("variables");
        assert!(!tui.show_variables);
        assert_eq!(tui.status.as_ref().unwrap().text, "Variables pane: hidden");

        tui.run_command_from_input(":vars");
        assert!(tui.show_variables);
        assert_eq!(tui.status.as_ref().unwrap().text, "Variables pane: shown");

        tui.run_command_from_input("stack");
        assert!(!tui.show_stack);
        assert_eq!(tui.status.as_ref().unwrap().text, "Stack pane: hidden");

        tui.run_command_from_input(":stack");
        assert!(tui.show_stack);
        assert_eq!(tui.status.as_ref().unwrap().text, "Stack pane: shown");

        tui.run_command_from_input("data");
        assert!(!tui.show_data);
        assert_eq!(tui.status.as_ref().unwrap().text, "Data pane: hidden");

        tui.run_command_from_input(":data");
        assert!(tui.show_data);
        assert_eq!(tui.status.as_ref().unwrap().text, "Data pane: shown");

        tui.run_command_from_input("stack extra");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Usage: :stack");
    }

    #[test]
    fn buffer_offset_input_mode_handles_calldata_offsets_and_blocks_normal_commands() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1)],
            Bytes::from(vec![0; 96]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;

        assert!(matches!(tui.handle_key_event(key(KeyCode::Char('o'))), ControlFlow::Continue(())));
        assert_eq!(tui.prompt, Some(Prompt::BufferOffset(String::new())));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.prompt, Some(Prompt::BufferOffset(String::new())));
        assert_eq!(tui.draw_memory.current_buf_startline, 0);

        for c in "40".chars() {
            let _ = tui.handle_key_event(key(KeyCode::Char(c)));
        }
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.draw_memory.current_buf_startline, 2);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Jumped to calldata offset 0x40 (64)");
    }

    #[test]
    fn buffer_offset_jumps_in_active_calldata_buffer() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1)],
            Bytes::from(vec![0; 96]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;

        tui.goto_buffer_offset_from_input("20");

        assert_eq!(tui.draw_memory.current_buf_startline, 1);
        assert_eq!(tui.status.as_ref().unwrap().text, "Jumped to calldata offset 0x20 (32)");
    }

    #[test]
    fn buffer_offset_jumps_to_visible_hex_label_on_partial_last_line() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1)],
            Bytes::from(vec![0; 65]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;

        tui.goto_buffer_offset_from_input("40");

        assert_eq!(tui.draw_memory.current_buf_startline, 2);
        assert_eq!(tui.status.as_ref().unwrap().text, "Jumped to calldata offset 0x40 (64)");
    }

    #[test]
    fn buffer_offset_reports_out_of_range_offsets_without_moving() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1)],
            Bytes::from(vec![0; 64]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;
        tui.draw_memory.current_buf_startline = 1;

        tui.goto_buffer_offset_from_input("20");
        assert_eq!(tui.draw_memory.current_buf_startline, 1);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Jumped to calldata offset 0x20 (32)");

        tui.draw_memory.current_buf_startline = 0;
        tui.goto_buffer_offset_from_input("0x80");
        assert_eq!(tui.draw_memory.current_buf_startline, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "calldata offset 0x80 (128) is outside the 64-byte buffer");
    }

    #[test]
    fn buffer_offset_escape_cancels_and_empty_buffer_reports_error() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('o')));
        let _ = tui.handle_key_event(key(KeyCode::Char('2')));
        let _ = tui.handle_key_event(key(KeyCode::Esc));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.draw_memory.current_buf_startline, 0);
        assert_eq!(tui.status, None);

        tui.goto_buffer_offset_from_input("0");
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Current memory buffer is empty");
    }

    #[test]
    fn buffer_scroll_reaches_partial_last_line() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step(1)],
            Bytes::from(vec![0; 65]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;

        let _ = tui.handle_key_event(ctrl_key(KeyCode::Char('j')));
        assert_eq!(tui.draw_memory.current_buf_startline, 1);
        let _ = tui.handle_key_event(ctrl_key(KeyCode::Char('j')));
        assert_eq!(tui.draw_memory.current_buf_startline, 2);
        let _ = tui.handle_key_event(ctrl_key(KeyCode::Char('j')));
        assert_eq!(tui.draw_memory.current_buf_startline, 2);
    }

    #[test]
    fn storage_scroll_repeats_without_exceeding_last_slot() {
        let steps =
            (0..3).map(|slot| step_with_stack(slot, OpCode::TSTORE, &[slot, slot])).collect();
        let mut context = context_with_arena(vec![DebugNode::new(
            Address::ZERO,
            CallKind::Call,
            steps,
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.current_step = 2;
        tui.run_command_from_input("transient");

        let _ = tui.handle_key_event(key(KeyCode::Char('2')));
        let _ = tui.handle_key_event(ctrl_key(KeyCode::Char('j')));
        assert_eq!(tui.draw_memory.current_storage_startline, 2);

        let _ = tui.handle_key_event(ctrl_key(KeyCode::Char('j')));
        assert_eq!(tui.draw_memory.current_storage_startline, 2);
    }

    #[test]
    fn opcode_search_wraps_and_is_case_insensitive() {
        let arena = vec![DebugNode::new(
            Address::repeat_byte(1),
            CallKind::Call,
            vec![
                step(1),
                step_with_immediate(2, OpCode::PUSH4, &[0x95, 0xd8, 0x9b, 0x41]),
                step_with_stack(3, OpCode::MSTORE, &[]),
            ],
            Bytes::new(),
            0,
            None,
        )];

        assert_eq!(
            find_opcode_match(&arena, 0, 0, "push4", SearchDirection::Forward),
            Some((0, 1))
        );
        assert_eq!(
            find_opcode_match(&arena, 0, 0, "95D89B41", SearchDirection::Forward),
            Some((0, 1))
        );
        assert_eq!(
            find_opcode_match(&arena, 0, 0, "mstore", SearchDirection::Backward),
            Some((0, 2))
        );
        assert_eq!(find_opcode_match(&arena, 0, 0, "sload", SearchDirection::Forward), None);
    }

    #[test]
    fn opcode_search_input_mode_handles_keys_and_blocks_normal_commands() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![
                step(1),
                step_with_immediate(2, OpCode::PUSH4, &[0x95, 0xd8, 0x9b, 0x41]),
                step_with_stack(3, OpCode::MSTORE, &[]),
            ],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        assert!(matches!(tui.handle_key_event(key(KeyCode::Char('/'))), ControlFlow::Continue(())));
        assert_eq!(tui.prompt, Some(Prompt::OpcodeSearch(String::new())));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.prompt, Some(Prompt::OpcodeSearch("q".into())));
        assert_eq!(tui.current_step, 0);

        let _ = tui.handle_key_event(key(KeyCode::Backspace));
        let _ = tui.handle_key_event(key(KeyCode::Char('9')));
        let _ = tui.handle_key_event(key(KeyCode::Char('5')));
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.last_opcode_search.as_deref(), Some("95"));
        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.status.as_ref().unwrap().kind, StatusKind::Info);
    }

    #[test]
    fn opcode_search_repeats_forward_and_backward() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![
                step_with_stack(1, OpCode::MSTORE, &[]),
                step(2),
                step_with_stack(3, OpCode::MSTORE, &[]),
            ],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('/')));
        for c in "mstore".chars() {
            let _ = tui.handle_key_event(key(KeyCode::Char(c)));
        }
        let _ = tui.handle_key_event(key(KeyCode::Enter));
        assert_eq!(tui.current_step, 2);

        let _ = tui.handle_key_event(key(KeyCode::Char('n')));
        assert_eq!(tui.current_step, 0);

        let _ = tui.handle_key_event(key(KeyCode::Char('N')));
        assert_eq!(tui.current_step, 2);
    }

    #[test]
    fn opcode_search_crosses_split_call_segments() {
        let address = Address::repeat_byte(1);
        let node = |address, trace_node_idx, step_offset, steps| {
            let mut node = DebugNode::new(address, CallKind::Call, steps, Bytes::new(), 0, None);
            node.trace_node_idx = trace_node_idx;
            node.step_offset = step_offset;
            node
        };
        let arena = vec![
            node(address, 0, 0, vec![step_with_stack(1, OpCode::MSTORE, &[]), step(2)]),
            node(Address::repeat_byte(2), 1, 0, vec![step_with_stack(3, OpCode::MSTORE, &[])]),
            node(address, 0, 2, vec![step(4), step_with_stack(5, OpCode::MSTORE, &[])]),
        ];

        assert_eq!(
            find_opcode_match(&arena, 0, 0, "mstore", SearchDirection::Forward),
            Some((2, 1))
        );
        assert_eq!(
            find_opcode_match(&arena, 2, 1, "mstore", SearchDirection::Forward),
            Some((0, 0))
        );
        assert_eq!(
            find_opcode_match(&arena, 0, 0, "mstore", SearchDirection::Backward),
            Some((2, 1))
        );
    }

    #[test]
    fn opcode_search_escape_cancels_without_moving() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('/')));
        let _ = tui.handle_key_event(key(KeyCode::Char('s')));
        let _ = tui.handle_key_event(key(KeyCode::Esc));

        assert_eq!(tui.prompt, None);
        assert_eq!(tui.last_opcode_search, None);
        assert_eq!(tui.current_step, 0);
        assert_eq!(tui.status, None);
    }

    #[test]
    fn opcode_search_reports_empty_input_without_remembering_search() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('/')));
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.current_step, 0);
        assert_eq!(tui.last_opcode_search, None);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "Enter an opcode search term");
    }

    #[test]
    fn opcode_search_reports_repeat_without_previous_search() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('n')));

        assert_eq!(tui.current_step, 0);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "No previous opcode search");
    }

    #[test]
    fn opcode_search_reports_no_match_without_moving() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('/')));
        for c in "sload".chars() {
            let _ = tui.handle_key_event(key(KeyCode::Char(c)));
        }
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.current_step, 0);
        assert_eq!(tui.last_opcode_search.as_deref(), Some("sload"));
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Error);
        assert_eq!(status.text, "No opcode matching `sload` in current call");
    }

    #[test]
    fn memory_write_start_line_uses_write_offset() {
        assert_eq!(memory_write_start_line(&step_with_stack(0, OpCode::MSTORE, &[0, 96])), Some(3));
        assert_eq!(
            memory_write_start_line(&step_with_stack(0, OpCode::MSTORE8, &[0, 33])),
            Some(1)
        );
        assert_eq!(memory_write_start_line(&step(0)), None);
    }

    #[test]
    fn bounded_memory_write_start_line_requires_visible_non_empty_write() {
        let write_at_128 = step_with_stack(0, OpCode::MSTORE, &[0, 128]);
        assert_eq!(bounded_memory_write_start_line(&write_at_128, 160), Some(4));
        assert_eq!(bounded_memory_write_start_line(&write_at_128, 128), None);

        let zero_len_copy = step_with_stack(0, OpCode::CALLDATACOPY, &[0, 0, 1_000_000]);
        assert_eq!(memory_write_start_line(&zero_len_copy), None);
        assert_eq!(bounded_memory_write_start_line(&zero_len_copy, 32), None);
    }

    #[test]
    fn stepping_past_memory_write_without_memory_snapshot_keeps_scroll_position() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step_with_stack(1, OpCode::MSTORE, &[0, 128]), step(2)],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.draw_memory.current_buf_startline = 99;

        tui.step();

        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.draw_memory.current_buf_startline, 99);
    }

    #[test]
    fn memory_write_autoscroll_only_applies_to_memory_buffer() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![step_with_stack(1, OpCode::MSTORE, &[0, 128]), step(2)],
            Bytes::from(vec![0; 256]),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;
        tui.draw_memory.current_buf_startline = 7;

        tui.step();

        assert_eq!(tui.current_step, 1);
        assert_eq!(tui.draw_memory.current_buf_startline, 7);
    }

    #[test]
    fn navigation_clamps_scroll_positions_to_non_empty_data() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![
            DebugNode::new(
                address,
                CallKind::Call,
                vec![step_with_stack(1, OpCode::STOP, &[0, 1, 2])],
                Bytes::from(vec![0; 64]),
                0,
                None,
            ),
            DebugNode::new(
                address,
                CallKind::Call,
                vec![step_with_stack(2, OpCode::STOP, &[0])],
                Bytes::from(vec![0; 4]),
                0,
                None,
            ),
        ]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.active_buffer = BufferKind::Calldata;
        tui.draw_memory.current_buf_startline = 1;
        tui.draw_memory.current_stack_startline = 2;

        let _ = tui.handle_key_event(key(KeyCode::Char('C')));

        assert_eq!(tui.draw_memory.inner_call_index, 1);
        assert_eq!(tui.draw_memory.current_buf_startline, 0);
        assert_eq!(tui.draw_memory.current_stack_startline, 0);
    }

    #[test]
    fn navigation_preserves_stack_scroll_across_empty_snapshot() {
        let address = Address::repeat_byte(1);
        let mut empty_stack = step(2);
        empty_stack.stack = Some(Vec::new().into_boxed_slice());
        let mut context = context_with_arena(vec![DebugNode::new(
            address,
            CallKind::Call,
            vec![
                step_with_stack(1, OpCode::STOP, &[0, 1, 2]),
                empty_stack,
                step_with_stack(3, OpCode::STOP, &[0, 1, 2]),
            ],
            Bytes::new(),
            0,
            None,
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();
        tui.draw_memory.current_stack_startline = 2;

        tui.step();
        assert_eq!(tui.draw_memory.current_stack_startline, 2);

        tui.step();
        assert_eq!(tui.draw_memory.current_stack_startline, 2);
    }
}
