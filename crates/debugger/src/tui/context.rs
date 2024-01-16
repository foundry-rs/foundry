//! Debugger context and event handler implementation.

use crate::{Debugger, ExitReason};
use alloy_primitives::Address;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use foundry_evm_core::debug::{DebugNodeFlat, DebugStep};
use revm_inspectors::tracing::types::CallKind;
use std::{cell::RefCell, ops::ControlFlow};

/// This is currently used to remember last scroll position so screen doesn't wiggle as much.
#[derive(Default)]
pub(crate) struct DrawMemory {
    // TODO
    pub(crate) current_startline: RefCell<usize>,
    pub(crate) inner_call_index: usize,
    pub(crate) current_mem_startline: usize,
    pub(crate) current_stack_startline: usize,
}

pub(crate) struct DebuggerContext<'a> {
    pub(crate) debugger: &'a mut Debugger,

    /// Buffer for keys prior to execution, i.e. '10' + 'k' => move up 10 operations.
    pub(crate) key_buffer: String,
    /// Current step in the debug steps.
    pub(crate) current_step: usize,
    pub(crate) draw_memory: DrawMemory,
    pub(crate) opcode_list: Vec<String>,
    pub(crate) last_index: usize,

    pub(crate) stack_labels: bool,
    pub(crate) mem_utf: bool,
    pub(crate) show_shortcuts: bool,
}

impl<'a> DebuggerContext<'a> {
    pub(crate) fn new(debugger: &'a mut Debugger) -> Self {
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

    pub(crate) fn init(&mut self) {
        self.gen_opcode_list();
    }

    pub(crate) fn debug_arena(&self) -> &[DebugNodeFlat] {
        &self.debugger.debug_arena
    }

    pub(crate) fn debug_call(&self) -> &DebugNodeFlat {
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
    pub(crate) fn debug_steps(&self) -> &[DebugStep] {
        &self.debug_call().steps
    }

    /// Returns the current debug step.
    pub(crate) fn current_step(&self) -> &DebugStep {
        &self.debug_steps()[self.current_step]
    }

    fn gen_opcode_list(&mut self) {
        self.opcode_list = self.opcode_list();
    }

    fn opcode_list(&self) -> Vec<String> {
        self.debug_steps().iter().map(DebugStep::pretty_opcode).collect()
    }
}

impl DebuggerContext<'_> {
    pub(crate) fn handle_event(&mut self, event: Event) -> ControlFlow<ExitReason> {
        if self.last_index != self.draw_memory.inner_call_index {
            self.gen_opcode_list();
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
            if c.is_alphabetic() && self.key_buffer.starts_with('\'') {
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
                for _ in 0..buffer_as_number(&self.key_buffer, 1) {
                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                        let max_mem = (self.current_step().memory.len() / 32).saturating_sub(1);
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
                for _ in 0..buffer_as_number(&self.key_buffer, 1) {
                    let max_stack = self.current_step().stack.len().saturating_sub(1);
                    if self.draw_memory.current_stack_startline < max_stack {
                        self.draw_memory.current_stack_startline += 1;
                    }
                }
                self.key_buffer.clear();
            }
            // Move up
            KeyCode::Char('k') | KeyCode::Up => {
                for _ in 0..buffer_as_number(&self.key_buffer, 1) {
                    if event.modifiers.contains(KeyModifiers::CONTROL) {
                        self.draw_memory.current_mem_startline =
                            self.draw_memory.current_mem_startline.saturating_sub(1);
                    } else if self.current_step > 0 {
                        self.current_step -= 1;
                    } else if self.draw_memory.inner_call_index > 0 {
                        self.draw_memory.inner_call_index -= 1;
                        self.current_step = self.debug_steps().len() - 1;
                    }
                }
                self.key_buffer.clear();
            }
            KeyCode::Char('K') => {
                for _ in 0..buffer_as_number(&self.key_buffer, 1) {
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
                self.current_step = self.debug_steps().len() - 1;
                self.key_buffer.clear();
            }
            // Go to previous call
            KeyCode::Char('c') => {
                self.draw_memory.inner_call_index =
                    self.draw_memory.inner_call_index.saturating_sub(1);
                self.current_step = self.debug_steps().len() - 1;
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
                for _ in 0..buffer_as_number(&self.key_buffer, 1) {
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
                for _ in 0..buffer_as_number(&self.key_buffer, 1) {
                    let prev_ops = self.opcode_list[..self.current_step].to_vec();
                    self.current_step = prev_ops
                        .iter()
                        .enumerate()
                        .rev()
                        .find_map(|(i, op)| {
                            if i > 0 {
                                match (
                                    prev_ops[i - 1].contains("JUMP")
                                        && prev_ops[i - 1] != "JUMPDEST",
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
            KeyCode::Char(
                other @ ('0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '\''),
            ) => self.key_buffer.push(other),
            _ => self.key_buffer.clear(),
        };

        ControlFlow::Continue(())
    }

    fn handle_breakpoint(&mut self, c: char) {
        // Find the location of the called breakpoint in the whole debug arena (at this address with
        // this pc)
        if let Some((caller, pc)) = self.debugger.breakpoints.get(&c) {
            for (i, node) in self.debug_arena().iter().enumerate() {
                if node.address == *caller {
                    if let Some(step) = node.steps.iter().position(|step| step.pc == *pc) {
                        self.draw_memory.inner_call_index = i;
                        self.current_step = step;
                        break;
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
                    self.current_step = self.debug_steps().len() - 1;
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
}

/// Grab number from buffer. Used for something like '10k' to move up 10 operations
fn buffer_as_number(s: &str, default_value: usize) -> usize {
    match s.parse() {
        Ok(num) if num >= 1 => num,
        _ => default_value,
    }
}
