//! Debugger context and event handler implementation.

use crate::{DebugNode, DebuggerLayout, ExitReason, debugger::DebuggerContext};
use alloy_primitives::{Address, hex};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use foundry_evm_core::buffer::{BufferKind, get_buffer_accesses};
use foundry_tui::TuiApp;
use ratatui::Frame;
use revm::bytecode::opcode::OpCode;
use revm_inspectors::tracing::types::{CallKind, CallTraceStep};
use std::ops::ControlFlow;

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
    pub(crate) current_stack_startline: usize,
    pub(crate) active_internal_call: Option<ActiveInternalCallCache>,
}

pub(crate) struct TUIContext<'a> {
    pub(crate) debugger_context: &'a mut DebuggerContext,

    /// Buffer for keys prior to execution, i.e. '10' + 'k' => move up 10 operations.
    pub(crate) key_buffer: String,
    /// Current goto program counter prompt contents, if the prompt is active.
    pub(crate) pc_input: Option<String>,
    /// Current active-buffer byte offset prompt contents, if the prompt is active.
    pub(crate) buffer_offset_input: Option<String>,
    /// Current opcode search prompt contents, if the prompt is active.
    pub(crate) opcode_search_input: Option<String>,
    /// Last opcode search term, used by repeat-search shortcuts.
    pub(crate) last_opcode_search: Option<String>,
    /// Last status or error message to show in the footer.
    pub(crate) status: Option<StatusMessage>,
    /// Current step in the debug steps.
    pub(crate) current_step: usize,
    pub(crate) draw_memory: DrawMemory,
    pub(crate) opcode_list: Vec<String>,
    pub(crate) last_index: usize,

    pub(crate) stack_labels: bool,
    /// Whether to decode active buffer as utf8 or not.
    pub(crate) buf_utf: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) show_source: bool,
    pub(crate) show_variables: bool,
    /// The currently active buffer (memory, calldata, returndata) to be drawn.
    pub(crate) active_buffer: BufferKind,
}

impl<'a> TUIContext<'a> {
    pub(crate) fn new(debugger_context: &'a mut DebuggerContext) -> Self {
        TUIContext {
            debugger_context,

            key_buffer: String::with_capacity(64),
            pc_input: None,
            buffer_offset_input: None,
            opcode_search_input: None,
            last_opcode_search: None,
            status: None,
            current_step: 0,
            draw_memory: DrawMemory::default(),
            opcode_list: Vec::new(),
            last_index: 0,

            stack_labels: false,
            buf_utf: false,
            show_shortcuts: true,
            show_source: true,
            show_variables: true,
            active_buffer: BufferKind::Memory,
        }
    }

    pub(crate) fn init(&mut self) {
        self.gen_opcode_list();
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

    fn gen_opcode_list(&mut self) {
        self.opcode_list.clear();
        let debug_steps =
            &self.debugger_context.debug_arena[self.draw_memory.inner_call_index].steps;
        for step in debug_steps {
            self.opcode_list.push(pretty_opcode(step));
        }
    }

    fn gen_opcode_list_if_necessary(&mut self) {
        if self.last_index != self.draw_memory.inner_call_index {
            self.gen_opcode_list();
            self.last_index = self.draw_memory.inner_call_index;
        }
    }

    fn active_buffer(&self) -> &[u8] {
        match self.active_buffer {
            BufferKind::Memory => self.current_step().memory.as_ref().map_or(&[], |m| m.as_bytes()),
            BufferKind::Calldata => &self.debug_call().calldata,
            BufferKind::Returndata => &self.current_step().returndata,
        }
    }

    pub(crate) const fn active_buffer_name(&self) -> &'static str {
        match self.active_buffer {
            BufferKind::Memory => "memory",
            BufferKind::Calldata => "calldata",
            BufferKind::Returndata => "returndata",
        }
    }
}

impl TUIContext<'_> {
    pub(crate) fn handle_event(&mut self, event: Event) -> ControlFlow<ExitReason> {
        let ret = match event {
            Event::Key(event) => self.handle_key_event(event),
            Event::Mouse(event) => self.handle_mouse_event(event),
            _ => ControlFlow::Continue(()),
        };
        // Generate the list after the event has been handled.
        self.gen_opcode_list_if_necessary();
        ret
    }

    fn handle_key_event(&mut self, event: KeyEvent) -> ControlFlow<ExitReason> {
        if self.opcode_search_input.is_some() {
            self.handle_opcode_search_input_key_event(event);
            return ControlFlow::Continue(());
        }

        if self.pc_input.is_some() {
            self.handle_pc_input_key_event(event);
            return ControlFlow::Continue(());
        }

        if self.buffer_offset_input.is_some() {
            self.handle_buffer_offset_input_key_event(event);
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

            // Scroll up the memory buffer
            KeyCode::Char('k') | KeyCode::Up if control => self.repeat(|this| {
                this.draw_memory.current_buf_startline =
                    this.draw_memory.current_buf_startline.saturating_sub(1);
            }),
            // Scroll down the memory buffer
            KeyCode::Char('j') | KeyCode::Down if control => self.repeat(|this| {
                let max_buf = this.active_buffer().len().div_ceil(32).saturating_sub(1);
                if this.draw_memory.current_buf_startline < max_buf {
                    this.draw_memory.current_buf_startline += 1;
                }
            }),

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
                self.active_buffer = self.active_buffer.next();
                self.draw_memory.current_buf_startline = 0;
                self.set_info(format!("Active buffer: {}", self.active_buffer_name()));
            }

            // Cycle layout
            KeyCode::Char('l') => self.cycle_layout(),

            // Go to top of file
            KeyCode::Char('g') => {
                self.draw_memory.inner_call_index = 0;
                self.current_step = 0;
                self.scroll_memory_to_current_write();
            }

            // Go to bottom of file
            KeyCode::Char('G') => {
                self.draw_memory.inner_call_index = self.debug_arena().len() - 1;
                self.current_step = self.n_steps() - 1;
                self.scroll_memory_to_current_write();
            }

            // Go to previous call
            KeyCode::Char('c') => {
                self.draw_memory.inner_call_index =
                    self.draw_memory.inner_call_index.saturating_sub(1);
                self.current_step = self.n_steps() - 1;
                self.scroll_memory_to_current_write();
            }

            // Go to next call
            KeyCode::Char('C')
                if self.debug_arena().len() > self.draw_memory.inner_call_index + 1 =>
            {
                self.draw_memory.inner_call_index += 1;
                self.current_step = 0;
                self.scroll_memory_to_current_write();
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
                    this.scroll_memory_to_current_write();
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
                this.scroll_memory_to_current_write();
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

            // Toggle source pane
            KeyCode::Char('v') => {
                self.show_source = !self.show_source;
                let state = if self.show_source { "shown" } else { "hidden" };
                self.set_info(format!("Source pane: {state}"));
            }

            // Toggle variables pane
            KeyCode::Char('V') => {
                self.show_variables = !self.show_variables;
                let state = if self.show_variables { "shown" } else { "hidden" };
                self.set_info(format!("Variables pane: {state}"));
            }

            // Go to program counter
            KeyCode::Char('p') => {
                self.key_buffer.clear();
                self.status = None;
                self.pc_input = Some(String::new());
            }

            // Go to byte offset in the active buffer
            KeyCode::Char('o') => {
                self.key_buffer.clear();
                self.status = None;
                self.buffer_offset_input = Some(String::new());
            }

            // Search opcodes in the current call
            KeyCode::Char('/') => {
                self.key_buffer.clear();
                self.status = None;
                self.opcode_search_input = Some(String::new());
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

    fn handle_pc_input_key_event(&mut self, event: KeyEvent) {
        if let Some(input) =
            handle_prompt_input_key_event(&mut self.pc_input, event, |_, c| is_pc_input_char(c))
        {
            self.goto_pc_from_input(&input);
        }
    }

    fn handle_buffer_offset_input_key_event(&mut self, event: KeyEvent) {
        if let Some(input) = handle_prompt_input_key_event(
            &mut self.buffer_offset_input,
            event,
            is_buffer_offset_input_char,
        ) {
            self.goto_buffer_offset_from_input(&input);
        }
    }

    fn handle_opcode_search_input_key_event(&mut self, event: KeyEvent) {
        match event.code {
            KeyCode::Esc => {
                self.opcode_search_input = None;
            }
            KeyCode::Enter => {
                let input = self.opcode_search_input.take().unwrap_or_default();
                self.search_opcode_from_input(&input);
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut self.opcode_search_input {
                    input.pop();
                }
            }
            KeyCode::Char(c) if !event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = &mut self.opcode_search_input {
                    input.push(c);
                }
            }
            _ => {}
        }
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
        let Some(step_index) =
            find_opcode_match(&self.opcode_list, self.current_step, query, direction)
        else {
            self.set_error(format!("No opcode matching `{query}` in current call"));
            return;
        };

        self.current_step = step_index;
        self.scroll_memory_to_current_write();

        let pc = self.current_step().pc;
        let opcode = self.opcode_list.get(step_index).map(String::as_str).unwrap_or_default();
        self.set_info(format!("Found `{query}` at PC 0x{pc:x} ({pc}): {opcode}"));
    }

    fn goto_pc_from_input(&mut self, input: &str) {
        let candidates = match parse_pc_candidates(input) {
            Ok(candidates) => candidates,
            Err(err) => {
                self.set_error(err);
                return;
            }
        };

        let mut found = Vec::new();
        for &candidate in &candidates {
            if let Some(target) = find_pc_target(
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

    fn apply_pc_target(&mut self, candidate: PcCandidate, target: PcTarget) {
        let already_at_target = self.draw_memory.inner_call_index == target.node_index
            && self.current_step == target.step_index;

        self.draw_memory.inner_call_index = target.node_index;
        self.current_step = target.step_index;
        self.draw_memory.current_buf_startline = 0;
        self.draw_memory.current_stack_startline = 0;
        self.scroll_memory_to_current_write();
        self.key_buffer.clear();

        let pc = candidate.pc;
        let scope = match target.scope {
            PcTargetScope::CurrentNode => "current trace",
            PcTargetScope::SameCodeContext => "same contract",
        };
        let action = if already_at_target { "Already at" } else { "Jumped to" };
        self.set_info(format!("{action} PC 0x{pc:x} ({pc}) in {scope}"));
    }

    fn goto_buffer_offset_from_input(&mut self, input: &str) {
        let offset = match parse_buffer_offset(input) {
            Ok(offset) => offset,
            Err(err) => {
                self.set_error(err);
                return;
            }
        };

        let buffer_name = self.active_buffer_name();
        let buffer_len = self.active_buffer().len();
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

        self.apply_buffer_offset(offset);
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

        // Find the location of the called breakpoint in the whole debug arena (at this address with
        // this pc)
        let Some((inner_call_index, step_index)) =
            self.debug_arena().iter().enumerate().find_map(|(i, node)| {
                (node.address == caller)
                    .then(|| node.steps.iter().position(|step| step.pc == pc).map(|step| (i, step)))
                    .flatten()
            })
        else {
            self.set_error(format!("Breakpoint '{c}' target not found in trace"));
            return;
        };

        let already_at_target = self.draw_memory.inner_call_index == inner_call_index
            && self.current_step == step_index;

        self.draw_memory.inner_call_index = inner_call_index;
        self.current_step = step_index;
        self.scroll_memory_to_current_write();

        let action = if already_at_target { "Already at" } else { "Jumped to" };
        self.set_info(format!("{action} breakpoint '{c}' at PC 0x{pc:x} ({pc})"));
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> ControlFlow<ExitReason> {
        if self.pc_input.is_some()
            || self.buffer_offset_input.is_some()
            || self.opcode_search_input.is_some()
        {
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
        self.scroll_memory_to_current_write();
    }

    fn step(&mut self) {
        if self.current_step < self.n_steps() - 1 {
            self.current_step += 1;
        } else if self.draw_memory.inner_call_index < self.debug_arena().len() - 1 {
            self.draw_memory.inner_call_index += 1;
            self.current_step = 0;
        }
        self.scroll_memory_to_current_write();
    }

    fn scroll_memory_to_current_write(&mut self) {
        if self.active_buffer != BufferKind::Memory {
            return;
        }

        if let Some(line) = self.current_memory_write_line() {
            self.draw_memory.current_buf_startline = line;
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

fn handle_prompt_input_key_event(
    input: &mut Option<String>,
    event: KeyEvent,
    is_input_char: impl Fn(&str, char) -> bool,
) -> Option<String> {
    match event.code {
        KeyCode::Esc => {
            *input = None;
        }
        KeyCode::Enter => {
            return Some(input.take().unwrap_or_default());
        }
        KeyCode::Backspace => {
            if let Some(input) = input {
                input.pop();
            }
        }
        KeyCode::Char(c) if !event.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(input) = input
                && is_input_char(input, c)
            {
                input.push(c);
            }
        }
        _ => {}
    }

    None
}

const fn is_pc_input_char(c: char) -> bool {
    c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | ':')
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
enum PcTargetScope {
    CurrentNode,
    SameCodeContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PcTarget {
    node_index: usize,
    step_index: usize,
    scope: PcTargetScope,
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

fn find_pc_target(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step: usize,
    pc: usize,
) -> Option<PcTarget> {
    let current_node = arena.get(current_node_index)?;

    if let Some(step_index) = find_pc_in_current_node(&current_node.steps, current_step, pc) {
        return Some(PcTarget {
            node_index: current_node_index,
            step_index,
            scope: PcTargetScope::CurrentNode,
        });
    }

    for (node_index, node) in arena.iter().enumerate().skip(current_node_index + 1) {
        if same_code_context(current_node, node)
            && let Some(step_index) = node.steps.iter().position(|step| step.pc == pc)
        {
            return Some(PcTarget {
                node_index,
                step_index,
                scope: PcTargetScope::SameCodeContext,
            });
        }
    }

    for (node_index, node) in arena.iter().enumerate().take(current_node_index).rev() {
        if same_code_context(current_node, node)
            && let Some(step_index) = node.steps.iter().rposition(|step| step.pc == pc)
        {
            return Some(PcTarget {
                node_index,
                step_index,
                scope: PcTargetScope::SameCodeContext,
            });
        }
    }

    None
}

fn find_pc_in_current_node(
    steps: &[CallTraceStep],
    current_step: usize,
    pc: usize,
) -> Option<usize> {
    if steps.get(current_step).is_some_and(|step| step.pc == pc) {
        return Some(current_step);
    }

    steps
        .iter()
        .enumerate()
        .skip(current_step.saturating_add(1))
        .find_map(|(i, step)| (step.pc == pc).then_some(i))
        .or_else(|| {
            steps[..current_step.min(steps.len())]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, step)| (step.pc == pc).then_some(i))
        })
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
    opcodes: &[String],
    current_step: usize,
    query: &str,
    direction: SearchDirection,
) -> Option<usize> {
    if opcodes.is_empty() {
        return None;
    }

    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }

    let current = current_step.min(opcodes.len() - 1);
    let matches = |i: usize| opcodes[i].to_ascii_lowercase().contains(&needle);

    match direction {
        SearchDirection::Forward => {
            ((current + 1)..opcodes.len()).chain(0..=current).find(|&i| matches(i))
        }
        SearchDirection::Backward => {
            (0..current).rev().chain((current..opcodes.len()).rev()).find(|&i| matches(i))
        }
    }
}

fn pretty_opcode(step: &CallTraceStep) -> String {
    if let Some(immediate) = step.immediate_bytes.as_ref().filter(|b| !b.is_empty()) {
        format!("{}(0x{})", step.op, hex::encode(immediate))
    } else {
        step.op.to_string()
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
    use alloy_primitives::{Bytes, U256};
    use foundry_evm_core::Breakpoints;
    use foundry_evm_traces::debug::ContractSources;
    use revm::interpreter::InstructionResult;

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
            contracts_sources: ContractSources::default(),
            breakpoints: Breakpoints::default(),
            layout: Default::default(),
        }
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

        let _ = tui.handle_key_event(key(KeyCode::Char('v')));
        assert!(!tui.show_source);
        assert_eq!(tui.status.as_ref().unwrap().text, "Source pane: hidden");
        let _ = tui.handle_key_event(key(KeyCode::Char('v')));
        assert!(tui.show_source);
        assert_eq!(tui.status.as_ref().unwrap().text, "Source pane: shown");

        let _ = tui.handle_key_event(key(KeyCode::Char('V')));
        assert!(!tui.show_variables);
        assert_eq!(tui.status.as_ref().unwrap().text, "Variables pane: hidden");
        let _ = tui.handle_key_event(key(KeyCode::Char('V')));
        assert!(tui.show_variables);
        assert_eq!(tui.status.as_ref().unwrap().text, "Variables pane: shown");

        let _ = tui.handle_key_event(key(KeyCode::Char('h')));
        assert!(!tui.show_shortcuts);
        assert_eq!(tui.status.as_ref().unwrap().text, "Shortcut help: hidden");
        let _ = tui.handle_key_event(key(KeyCode::Char('h')));
        assert!(tui.show_shortcuts);
        assert_eq!(tui.status.as_ref().unwrap().text, "Shortcut help: shown");
    }

    #[test]
    fn breakpoint_shortcut_jumps_and_reports_status() {
        let address = Address::repeat_byte(1);
        let other = Address::repeat_byte(2);
        let mut context = context_with_arena(vec![
            node(other, CallKind::Call, &[1]),
            node(address, CallKind::Call, &[7, 42]),
        ]);
        context.breakpoints.insert('a', (address, 42));
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('\'')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));

        assert_eq!(tui.draw_memory.inner_call_index, 1);
        assert_eq!(tui.current_step, 1);
        let status = tui.status.as_ref().unwrap();
        assert_eq!(status.kind, StatusKind::Info);
        assert_eq!(status.text, "Jumped to breakpoint 'a' at PC 0x2a (42)");
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
            Some(PcTarget { node_index: 0, step_index: 2, scope: PcTargetScope::CurrentNode })
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
    fn pc_input_mode_handles_keys_and_blocks_normal_commands() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        assert!(matches!(tui.handle_key_event(key(KeyCode::Char('p'))), ControlFlow::Continue(())));
        assert_eq!(tui.pc_input.as_deref(), Some(""));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.pc_input.as_deref(), Some(""));
        assert_eq!(tui.current_step, 0);

        let _ = tui.handle_key_event(key(KeyCode::Char('2')));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        assert_eq!(tui.pc_input.as_deref(), Some("2a"));

        let _ = tui.handle_key_event(key(KeyCode::Backspace));
        assert_eq!(tui.pc_input.as_deref(), Some("2"));
        let _ = tui.handle_key_event(key(KeyCode::Char('a')));
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.pc_input, None);
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

        assert_eq!(tui.pc_input, None);
        assert_eq!(tui.current_step, 0);
        assert_eq!(tui.status, None);
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
        assert_eq!(tui.buffer_offset_input.as_deref(), Some(""));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.buffer_offset_input.as_deref(), Some(""));
        assert_eq!(tui.draw_memory.current_buf_startline, 0);

        for c in "40".chars() {
            let _ = tui.handle_key_event(key(KeyCode::Char(c)));
        }
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.buffer_offset_input, None);
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

        assert_eq!(tui.buffer_offset_input, None);
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
    fn opcode_search_wraps_and_is_case_insensitive() {
        let opcodes =
            vec!["STOP".to_string(), "PUSH4(0x95d89b41)".to_string(), "MSTORE".to_string()];

        assert_eq!(find_opcode_match(&opcodes, 0, "push4", SearchDirection::Forward), Some(1));
        assert_eq!(find_opcode_match(&opcodes, 0, "95D89B41", SearchDirection::Forward), Some(1));
        assert_eq!(find_opcode_match(&opcodes, 0, "mstore", SearchDirection::Backward), Some(2));
        assert_eq!(find_opcode_match(&opcodes, 0, "sload", SearchDirection::Forward), None);
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
        assert_eq!(tui.opcode_search_input.as_deref(), Some(""));

        let _ = tui.handle_key_event(key(KeyCode::Char('q')));
        assert_eq!(tui.opcode_search_input.as_deref(), Some("q"));
        assert_eq!(tui.current_step, 0);

        let _ = tui.handle_key_event(key(KeyCode::Backspace));
        let _ = tui.handle_key_event(key(KeyCode::Char('9')));
        let _ = tui.handle_key_event(key(KeyCode::Char('5')));
        let _ = tui.handle_key_event(key(KeyCode::Enter));

        assert_eq!(tui.opcode_search_input, None);
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
    fn opcode_search_escape_cancels_without_moving() {
        let address = Address::repeat_byte(1);
        let mut context = context_with_arena(vec![node(address, CallKind::Call, &[1, 42])]);
        let mut tui = TUIContext::new(&mut context);
        tui.init();

        let _ = tui.handle_key_event(key(KeyCode::Char('/')));
        let _ = tui.handle_key_event(key(KeyCode::Char('s')));
        let _ = tui.handle_key_event(key(KeyCode::Esc));

        assert_eq!(tui.opcode_search_input, None);
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
            Bytes::new(),
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
}
