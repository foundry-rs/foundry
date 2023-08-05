use crate::{Breakpoints, ContractBytecodeSome, TUIExitReason};
use cast::{
    decode,
    executor::inspector::{
        cheatcodes::{util::BroadcastableTransactions, BroadcastableTransaction},
        DEFAULT_CREATE2_DEPLOYER,
    },
    trace::CallTraceDecoder,
};
use crossterm::{
    event::{self, DisableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, LeaveAlternateScreen},
};
use ethers::{
    signers::LocalWallet,
    types::{Address, Log},
};
use ethers_solc::{
    artifacts::{BytecodeObject, CompactBytecode, CompactContractBytecode, Libraries},
    contracts::ArtifactContracts,
    ArtifactId, Project,
};
use forge::{debug::DebugArena, trace::Traces};
use std::{
    collections::BTreeMap,
    convert::From,
    io,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tracing::log::trace;

pub struct ExecutionResult {}

/// Standardized way of firing up the debugger
pub struct DebuggerArgs<'a> {
    pub success: bool,
    pub debug: Option<Vec<DebugArena>>,
    pub decoder: &'a CallTraceDecoder,
    pub sources: BTreeMap<ArtifactId, String>,
    pub project: Project,
    pub highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    pub breakpoints: Breakpoints,
}

impl DebuggerArgs<'_> {
    pub fn run(&self) -> eyre::Result<()> {
        // trace!(target: "debugger", "running debugger");

        // let (sources, artifacts) = filter_sources_and_artifacts(
        //     &self.path,
        //     sources,
        //     highlevel_known_contracts.clone(),
        //     project,
        // )?;
        // let flattened = result
        //     .debug
        //     .and_then(|arena| arena.last().map(|arena| arena.flatten(0)))
        //     .expect("We should have collected debug information");
        // let identified_contracts = decoder
        //     .contracts
        //     .iter()
        //     .map(|(addr, identifier)| (*addr, get_contract_name(identifier).to_string()))
        //     .collect();
        // let tui = Tui::new(
        //     flattened,
        //     0,
        //     identified_contracts,
        //     artifacts,
        //     highlevel_known_contracts
        //         .into_iter()
        //         .map(|(id, _)| (id.name, sources.clone()))
        //         .collect(),
        //     breakpoints,
        // )?;

        // // If something panics inside here, we should do everything we can to
        // // not corrupt the user's terminal.
        // std::panic::set_hook(Box::new(|e| {
        //     disable_raw_mode().expect("Unable to disable raw mode");
        //     execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
        //         .expect("unable to execute disable mouse capture");
        //     println!("{e}");
        // }));
        // // This is the recommend tick rate from tui-rs, based on their examples
        // let tick_rate = Duration::from_millis(200);

        // // Setup a channel to send interrupts
        // let (tx, rx) = mpsc::channel();
        // thread::spawn(move || {
        //     let mut last_tick = Instant::now();
        //     loop {
        //         // Poll events since last tick - if last tick is greater than tick_rate, we
        // demand         // immediate availability of the event. This may affect
        //         // interactivity, but I'm not sure as it is hard to test.
        //         if event::poll(tick_rate.saturating_sub(last_tick.elapsed())).unwrap() {
        //             let event = event::read().unwrap();
        //             if let Event::Key(key) = event {
        //                 if tx.send(Interrupt::KeyPressed(key)).is_err() {
        //                     return;
        //                 }
        //             } else if let Event::Mouse(mouse) = event {
        //                 if tx.send(Interrupt::MouseEvent(mouse)).is_err() {
        //                     return;
        //                 }
        //             }
        //         }
        //         // Force update if time has passed
        //         if last_tick.elapsed() > tick_rate {
        //             if tx.send(Interrupt::IntervalElapsed).is_err() {
        //                 return;
        //             }
        //             last_tick = Instant::now();
        //         }
        //     }
        // });

        // self.terminal.clear()?;
        // let mut draw_memory: DrawMemory = DrawMemory::default();

        // let debug_call: Vec<(Address, Vec<DebugStep>, CallKind)> = self.debug_arena.clone();
        // let mut opcode_list: Vec<String> =
        //     debug_call[0].1.iter().map(|step| step.pretty_opcode()).collect();
        // let mut last_index = 0;

        // let mut stack_labels = false;
        // let mut mem_utf = false;
        // let mut show_shortcuts = true;
        // // UI thread that manages drawing
        // loop {
        //     if last_index != draw_memory.inner_call_index {
        //         opcode_list = debug_call[draw_memory.inner_call_index]
        //             .1
        //             .iter()
        //             .map(|step| step.pretty_opcode())
        //             .collect();
        //         last_index = draw_memory.inner_call_index;
        //     }
        //     // Grab interrupt

        //     let receiver = rx.recv()?;

        //     if let Some(c) = receiver.char_press() {
        //         if self.key_buffer.ends_with('\'') {
        //             // Find the location of the called breakpoint in the whole debug arena (at
        // this             // address with this pc)
        //             if let Some((caller, pc)) = self.breakpoints.get(&c) {
        //                 for (i, (_caller, debug_steps, _)) in debug_call.iter().enumerate() {
        //                     if _caller == caller {
        //                         if let Some(step) =
        //                             debug_steps.iter().position(|step| step.pc == *pc)
        //                         {
        //                             draw_memory.inner_call_index = i;
        //                             self.current_step = step;
        //                             break;
        //                         }
        //                     }
        //                 }
        //             }
        //             self.key_buffer.clear();
        //         } else if let Interrupt::KeyPressed(event) = receiver {
        //             match event.code {
        //                 // Exit
        //                 KeyCode::Char('q') => {
        //                     disable_raw_mode()?;
        //                     execute!(
        //                         self.terminal.backend_mut(),
        //                         LeaveAlternateScreen,
        //                         DisableMouseCapture
        //                     )?;
        //                     return Ok(TUIExitReason::CharExit);
        //                 }
        //                 // Move down
        //                 KeyCode::Char('j') | KeyCode::Down => {
        //                     // Grab number of times to do it
        //                     for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
        //                         if event.modifiers.contains(KeyModifiers::CONTROL) {
        //                             let max_mem = (debug_call[draw_memory.inner_call_index].1
        //                                 [self.current_step]
        //                                 .memory
        //                                 .len()
        //                                 / 32)
        //                                 .saturating_sub(1);
        //                             if draw_memory.current_mem_startline < max_mem {
        //                                 draw_memory.current_mem_startline += 1;
        //                             }
        //                         } else if self.current_step < opcode_list.len() - 1 {
        //                             self.current_step += 1;
        //                         } else if draw_memory.inner_call_index < debug_call.len() - 1 {
        //                             draw_memory.inner_call_index += 1;
        //                             self.current_step = 0;
        //                         }
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 KeyCode::Char('J') => {
        //                     for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
        //                         let max_stack = debug_call[draw_memory.inner_call_index].1
        //                             [self.current_step]
        //                             .stack
        //                             .len()
        //                             .saturating_sub(1);
        //                         if draw_memory.current_stack_startline < max_stack {
        //                             draw_memory.current_stack_startline += 1;
        //                         }
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 // Move up
        //                 KeyCode::Char('k') | KeyCode::Up => {
        //                     for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
        //                         if event.modifiers.contains(KeyModifiers::CONTROL) {
        //                             draw_memory.current_mem_startline =
        //                                 draw_memory.current_mem_startline.saturating_sub(1);
        //                         } else if self.current_step > 0 {
        //                             self.current_step -= 1;
        //                         } else if draw_memory.inner_call_index > 0 {
        //                             draw_memory.inner_call_index -= 1;
        //                             self.current_step =
        //                                 debug_call[draw_memory.inner_call_index].1.len() - 1;
        //                         }
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 KeyCode::Char('K') => {
        //                     for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
        //                         draw_memory.current_stack_startline =
        //                             draw_memory.current_stack_startline.saturating_sub(1);
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 // Go to top of file
        //                 KeyCode::Char('g') => {
        //                     draw_memory.inner_call_index = 0;
        //                     self.current_step = 0;
        //                     self.key_buffer.clear();
        //                 }
        //                 // Go to bottom of file
        //                 KeyCode::Char('G') => {
        //                     draw_memory.inner_call_index = debug_call.len() - 1;
        //                     self.current_step =
        //                         debug_call[draw_memory.inner_call_index].1.len() - 1;
        //                     self.key_buffer.clear();
        //                 }
        //                 // Go to previous call
        //                 KeyCode::Char('c') => {
        //                     draw_memory.inner_call_index =
        //                         draw_memory.inner_call_index.saturating_sub(1);
        //                     self.current_step =
        //                         debug_call[draw_memory.inner_call_index].1.len() - 1;
        //                     self.key_buffer.clear();
        //                 }
        //                 // Go to next call
        //                 KeyCode::Char('C') => {
        //                     if debug_call.len() > draw_memory.inner_call_index + 1 {
        //                         draw_memory.inner_call_index += 1;
        //                         self.current_step = 0;
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 // Step forward
        //                 KeyCode::Char('s') => {
        //                     for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
        //                         let remaining_ops =
        //                             opcode_list[self.current_step..].to_vec().clone();
        //                         self.current_step += remaining_ops
        //                             .iter()
        //                             .enumerate()
        //                             .find_map(|(i, op)| {
        //                                 if i < remaining_ops.len() - 1 {
        //                                     match (
        //                                         op.contains("JUMP") && op != "JUMPDEST",
        //                                         &*remaining_ops[i + 1],
        //                                     ) { (true, "JUMPDEST") => Some(i + 1), _ => None,
        //                                     }
        //                                 } else {
        //                                     None
        //                                 }
        //                             })
        //                             .unwrap_or(opcode_list.len() - 1);
        //                         if self.current_step > opcode_list.len() {
        //                             self.current_step = opcode_list.len() - 1
        //                         };
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 // Step backwards
        //                 KeyCode::Char('a') => {
        //                     for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
        //                         let prev_ops = opcode_list[..self.current_step].to_vec().clone();
        //                         self.current_step = prev_ops
        //                             .iter()
        //                             .enumerate()
        //                             .rev()
        //                             .find_map(|(i, op)| {
        //                                 if i > 0 {
        //                                     match (
        //                                         prev_ops[i - 1].contains("JUMP")
        //                                             && prev_ops[i - 1] != "JUMPDEST",
        //                                         &**op,
        //                                     ) { (true, "JUMPDEST") => Some(i - 1), _ => None,
        //                                     }
        //                                 } else {
        //                                     None
        //                                 }
        //                             })
        //                             .unwrap_or_default();
        //                     }
        //                     self.key_buffer.clear();
        //                 }
        //                 // toggle stack labels
        //                 KeyCode::Char('t') => {
        //                     stack_labels = !stack_labels;
        //                 }
        //                 // toggle memory utf8 decoding
        //                 KeyCode::Char('m') => {
        //                     mem_utf = !mem_utf;
        //                 }
        //                 // toggle help notice
        //                 KeyCode::Char('h') => {
        //                     show_shortcuts = !show_shortcuts;
        //                 }
        //                 KeyCode::Char(other) => match other {
        //                     '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '\'' => {
        //                         self.key_buffer.push(other);
        //                     }
        //                     _ => {
        //                         // Invalid key, clear buffer
        //                         self.key_buffer.clear();
        //                     }
        //                 },
        //                 _ => {
        //                     self.key_buffer.clear();
        //                 }
        //             }
        //         }
        //     } else {
        //         match receiver {
        //             Interrupt::MouseEvent(event) => match event.kind {
        //                 MouseEventKind::ScrollUp => {
        //                     if self.current_step > 0 {
        //                         self.current_step -= 1;
        //                     } else if draw_memory.inner_call_index > 0 {
        //                         draw_memory.inner_call_index -= 1;
        //                         draw_memory.current_mem_startline = 0;
        //                         draw_memory.current_stack_startline = 0;
        //                         self.current_step =
        //                             debug_call[draw_memory.inner_call_index].1.len() - 1;
        //                     }
        //                 }
        //                 MouseEventKind::ScrollDown => {
        //                     if self.current_step < opcode_list.len() - 1 {
        //                         self.current_step += 1;
        //                     } else if draw_memory.inner_call_index < debug_call.len() - 1 {
        //                         draw_memory.inner_call_index += 1;
        //                         draw_memory.current_mem_startline = 0;
        //                         draw_memory.current_stack_startline = 0;
        //                         self.current_step = 0;
        //                     }
        //                 }
        //                 _ => {}
        //             },
        //             Interrupt::IntervalElapsed => {}
        //             _ => (),
        //         }
        //     }

        //     // Draw
        //     let current_step = self.current_step;
        //     self.terminal.draw(|f| {
        //         Tui::draw_layout(
        //             f,
        //             debug_call[draw_memory.inner_call_index].0,
        //             &self.identified_contracts,
        //             &self.known_contracts,
        //             &self.pc_ic_maps,
        //             &self.known_contracts_sources,
        //             &debug_call[draw_memory.inner_call_index].1[..],
        //             &opcode_list,
        //             current_step,
        //             debug_call[draw_memory.inner_call_index].2,
        //             &mut draw_memory,
        //             stack_labels,
        //             mem_utf,
        //             show_shortcuts,
        //         )
        //     })?;
        // }
        Ok(())
    }
}
