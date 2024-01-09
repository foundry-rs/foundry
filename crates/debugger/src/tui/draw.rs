//! TUI draw implementation.

use super::context::DebuggerContext;
use crate::op::OpcodeParam;
use alloy_primitives::U256;
use foundry_compilers::sourcemap::SourceElement;
use foundry_evm_core::{debug::Instruction, utils::CallKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use revm::interpreter::opcode;
use std::{cmp, collections::VecDeque, fmt::Write, io};

impl DebuggerContext<'_> {
    /// Draws the TUI layout and subcomponents to the given terminal.
    pub(crate) fn draw(&self, terminal: &mut super::DebuggerTerminal) -> io::Result<()> {
        terminal.draw(|f| self.draw_layout(f)).map(drop)
    }

    #[inline]
    fn draw_layout(&self, f: &mut Frame<'_>) {
        // We need 100 columns to display a 32 byte word in the memory and stack panes.
        let size = f.size();
        let min_width = 100;
        let min_height = 16;
        if size.width < min_width || size.height < min_height {
            self.size_too_small(f, min_width, min_height);
            return;
        }

        // The horizontal layout draws these panes at 50% width.
        let min_column_width_for_horizontal = 200;
        if size.width >= min_column_width_for_horizontal {
            self.horizontal_layout(f);
        } else {
            self.vertical_layout(f);
        }
    }

    fn size_too_small(&self, f: &mut Frame<'_>, min_width: u16, min_height: u16) {
        let mut lines = Vec::with_capacity(4);

        let l1 = "Terminal size too small:";
        lines.push(Line::from(l1));

        let size = f.size();
        let width_color = if size.width >= min_width { Color::Green } else { Color::Red };
        let height_color = if size.height >= min_height { Color::Green } else { Color::Red };
        let l2 = vec![
            Span::raw("Width = "),
            Span::styled(size.width.to_string(), Style::new().fg(width_color)),
            Span::raw(" Height = "),
            Span::styled(size.height.to_string(), Style::new().fg(height_color)),
        ];
        lines.push(Line::from(l2));

        let l3 = "Needed for current config:";
        lines.push(Line::from(l3));
        let l4 = format!("Width = {min_width} Height = {min_height}");
        lines.push(Line::from(l4));

        let paragraph =
            Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true });
        f.render_widget(paragraph, size)
    }

    /// Draws the layout in vertical mode.
    ///
    /// ```text
    /// |-----------------------------|
    /// |             op              |
    /// |-----------------------------|
    /// |            stack            |
    /// |-----------------------------|
    /// |             mem             |
    /// |-----------------------------|
    /// |                             |
    /// |             src             |
    /// |                             |
    /// |-----------------------------|
    /// ```
    fn vertical_layout(&self, f: &mut Frame<'_>) {
        let area = f.size();
        let h_height = if self.show_shortcuts { 4 } else { 0 };

        // NOTE: `Layout::split` always returns a slice of the same length as the number of
        // constraints, so the `else` branch is unreachable.

        // Split off footer.
        let [app, footer] = Layout::new()
            .constraints([Constraint::Ratio(100 - h_height, 100), Constraint::Ratio(h_height, 100)])
            .direction(Direction::Vertical)
            .split(area)[..]
        else {
            unreachable!()
        };

        // Split the app in 4 vertically to construct all the panes.
        let [op_pane, stack_pane, memory_pane, src_pane] = Layout::new()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Ratio(1, 6),
                Constraint::Ratio(1, 6),
                Constraint::Ratio(1, 6),
                Constraint::Ratio(3, 6),
            ])
            .split(app)[..]
        else {
            unreachable!()
        };

        if self.show_shortcuts {
            self.draw_footer(f, footer);
        }
        self.draw_src(f, src_pane);
        self.draw_op_list(f, op_pane);
        self.draw_stack(f, stack_pane);
        self.draw_memory(f, memory_pane);
    }

    /// Draws the layout in horizontal mode.
    ///
    /// ```text
    /// |-----------------|-----------|
    /// |        op       |   stack   |
    /// |-----------------|-----------|
    /// |                 |           |
    /// |       src       |    mem    |
    /// |                 |           |
    /// |-----------------|-----------|
    /// ```
    fn horizontal_layout(&self, f: &mut Frame<'_>) {
        let area = f.size();
        let h_height = if self.show_shortcuts { 4 } else { 0 };

        // Split off footer.
        let [app, footer] = Layout::new()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(100 - h_height, 100), Constraint::Ratio(h_height, 100)])
            .split(area)[..]
        else {
            unreachable!()
        };

        // Split app in 2 horizontally.
        let [app_left, app_right] = Layout::new()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(app)[..]
        else {
            unreachable!()
        };

        // Split left pane in 2 vertically to opcode list and source.
        let [op_pane, src_pane] = Layout::new()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
            .split(app_left)[..]
        else {
            unreachable!()
        };

        // Split right pane horizontally to construct stack and memory.
        let [stack_pane, memory_pane] = Layout::new()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
            .split(app_right)[..]
        else {
            unreachable!()
        };

        if self.show_shortcuts {
            self.draw_footer(f, footer);
        }
        self.draw_src(f, src_pane);
        self.draw_op_list(f, op_pane);
        self.draw_stack(f, stack_pane);
        self.draw_memory(f, memory_pane);
    }

    fn draw_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let l1 = "[q]: quit | [k/j]: prev/next op | [a/s]: prev/next jump | [c/C]: prev/next call | [g/G]: start/end";
        let l2 = "[t]: stack labels | [m]: memory decoding | [shift + j/k]: scroll stack | [ctrl + j/k]: scroll memory | ['<char>]: goto breakpoint | [h] toggle help";
        let dimmed = Style::new().add_modifier(Modifier::DIM);
        let lines =
            vec![Line::from(Span::styled(l1, dimmed)), Line::from(Span::styled(l2, dimmed))];
        let paragraph =
            Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    fn draw_src(&self, f: &mut Frame<'_>, area: Rect) {
        let text_output = self.src_text(area);
        let title = match self.call_kind() {
            CallKind::Create | CallKind::Create2 => "Contract creation",
            CallKind::Call => "Contract call",
            CallKind::StaticCall => "Contract staticcall",
            CallKind::CallCode => "Contract callcode",
            CallKind::DelegateCall => "Contract delegatecall",
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text_output).block(block).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    fn src_text(&self, area: Rect) -> Text<'_> {
        let (source_element, source_code) = match self.src_map() {
            Ok(r) => r,
            Err(e) => return Text::from(e),
        };

        // We are handed a vector of SourceElements that give us a span of sourcecode that is
        // currently being executed. This includes an offset and length.
        // This vector is in instruction pointer order, meaning the location of the instruction
        // minus `sum(push_bytes[..pc])`.
        let offset = source_element.offset;
        let len = source_element.length;
        let max = source_code.len();

        // Split source into before, relevant, and after chunks, split by line, for formatting.
        let actual_start = offset.min(max);
        let actual_end = (offset + len).min(max);

        let mut before: Vec<_> = source_code[..actual_start].split_inclusive('\n').collect();
        let actual: Vec<_> = source_code[actual_start..actual_end].split_inclusive('\n').collect();
        let mut after: VecDeque<_> = source_code[actual_end..].split_inclusive('\n').collect();

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

        // Unhighlighted line number: gray.
        let u_num = Style::new().fg(Color::Gray);
        // Unhighlighted text: default, dimmed.
        let u_text = Style::new().add_modifier(Modifier::DIM);
        // Highlighted line number: cyan.
        let h_num = Style::new().fg(Color::Cyan);
        // Highlighted text: cyan, bold.
        let h_text = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

        let mut lines = SourceLines::new(decimal_digits(num_lines));

        // We check if there is other text on the same line before the highlight starts.
        if let Some(last) = before.pop() {
            let last_has_nl = last.ends_with('\n');

            if last_has_nl {
                before.push(last);
            }
            for line in &before[start_line..] {
                lines.push(u_num, line, u_text);
            }

            let first = if !last_has_nl {
                lines.push_raw(h_num, &[Span::raw(last), Span::styled(actual[0], h_text)]);
                1
            } else {
                0
            };

            // Skip the first line if it has already been handled above.
            for line in &actual[first..] {
                lines.push(h_num, line, h_text);
            }
        } else {
            // No text before the current line.
            for line in &actual {
                lines.push(h_num, line, h_text);
            }
        }

        // Fill in the rest of the line as unhighlighted.
        if let Some(last) = actual.last() {
            if !last.ends_with('\n') {
                if let Some(post) = after.pop_front() {
                    if let Some(last) = lines.lines.last_mut() {
                        last.spans.push(Span::raw(post));
                    }
                }
            }
        }

        // Add after highlighted text.
        while mid_len + after.len() > end_line {
            after.pop_back();
        }
        for line in after {
            lines.push(u_num, line, u_text);
        }

        Text::from(lines.lines)
    }

    fn src_map(&self) -> Result<(SourceElement, &str), String> {
        let address = self.address();
        let Some(contract_name) = self.debugger.identified_contracts.get(address) else {
            return Err(format!("Unknown contract at address {address}"));
        };

        let Some(files_source_code) = self.debugger.contracts_sources.0.get(contract_name) else {
            return Err(format!("No source map index for contract {contract_name}"));
        };

        let Some((create_map, rt_map)) = self.debugger.pc_ic_maps.get(contract_name) else {
            return Err(format!("No PC-IC maps for contract {contract_name}"));
        };

        let is_create = matches!(self.call_kind(), CallKind::Create | CallKind::Create2);
        let pc = self.current_step().pc;
        let Some((source_element, source_code)) =
            files_source_code.iter().find_map(|(file_id, (source_code, contract_source))| {
                let bytecode = if is_create {
                    &contract_source.bytecode
                } else {
                    contract_source.deployed_bytecode.bytecode.as_ref()?
                };
                let mut source_map = bytecode.source_map()?.ok()?;

                let pc_ic_map = if is_create { create_map } else { rt_map };
                let ic = pc_ic_map.get(pc)?;
                let source_element = source_map.swap_remove(ic);
                (*file_id == source_element.index?).then_some((source_element, source_code))
            })
        else {
            return Err(format!("No source map for contract {contract_name}"));
        };

        Ok((source_element, source_code))
    }

    fn draw_op_list(&self, f: &mut Frame<'_>, area: Rect) {
        let height = area.height as i32;
        let extra_top_lines = height / 2;
        // Absolute minimum start line
        let abs_min_start = 0;
        // Adjust for weird scrolling for max top line
        let abs_max_start = (self.opcode_list.len() as i32 - 1) - (height / 2);
        // actual minimum start line
        let mut min_start =
            cmp::max(self.current_step as i32 - height + extra_top_lines, abs_min_start) as usize;

        // actual max start line
        let mut max_start = cmp::max(
            cmp::min(self.current_step as i32 - extra_top_lines, abs_max_start),
            abs_min_start,
        ) as usize;

        // Sometimes, towards end of file, maximum and minim lines have swapped values. Swap if the
        // case
        if min_start > max_start {
            std::mem::swap(&mut min_start, &mut max_start);
        }

        let prev_start = *self.draw_memory.current_startline.borrow();
        let display_start = prev_start.clamp(min_start, max_start);
        *self.draw_memory.current_startline.borrow_mut() = display_start;

        let max_pc = self.debug_steps().iter().map(|step| step.pc).max().unwrap_or(0);
        let max_pc_len = hex_digits(max_pc);

        let debug_steps = self.debug_steps();
        let mut lines = Vec::new();
        let mut add_new_line = |line_number: usize| {
            let mut line = String::with_capacity(64);

            let is_current_step = line_number == self.current_step;
            if line_number < self.debug_steps().len() {
                let step = &debug_steps[line_number];
                write!(line, "{:0>max_pc_len$x}|", step.pc).unwrap();
                line.push_str(if is_current_step { "▶" } else { " " });
                if let Some(op) = self.opcode_list.get(line_number) {
                    line.push_str(op);
                }
            } else {
                line.push_str("END CALL");
            }

            let bg_color = if is_current_step { Color::DarkGray } else { Color::Reset };
            let style = Style::new().fg(Color::White).bg(bg_color);
            lines.push(Line::from(Span::styled(line, style)));
        };

        for number in display_start..self.opcode_list.len() {
            add_new_line(number);
        }

        // Add one more "phantom" line so we see line where current segment execution ends
        add_new_line(self.opcode_list.len());

        let title = format!(
            "Address: {} | PC: {} | Gas used in call: {}",
            self.address(),
            self.current_step().pc,
            self.current_step().total_gas_used,
        );
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    fn draw_stack(&self, f: &mut Frame<'_>, area: Rect) {
        let step = self.current_step();
        let stack = &step.stack;

        let min_len = decimal_digits(stack.len()).max(2);

        let params =
            if let Instruction::OpCode(op) = step.instruction { OpcodeParam::of(op) } else { &[] };

        let text: Vec<Line> = stack
            .iter()
            .rev()
            .enumerate()
            .skip(self.draw_memory.current_stack_startline)
            .map(|(i, stack_item)| {
                let param = params.iter().find(|param| param.index == i);

                let mut spans = Vec::with_capacity(1 + 32 * 2 + 3);

                // Stack index.
                spans.push(Span::styled(format!("{i:0min_len$}| "), Style::new().fg(Color::White)));

                // Item hex bytes.
                hex_bytes_spans(&stack_item.to_be_bytes::<32>(), &mut spans, |_, _| {
                    if param.is_some() {
                        Style::new().fg(Color::Cyan)
                    } else {
                        Style::new().fg(Color::White)
                    }
                });

                if self.stack_labels {
                    if let Some(param) = param {
                        spans.push(Span::raw("| "));
                        spans.push(Span::raw(param.name));
                    }
                }

                spans.push(Span::raw("\n"));

                Line::from(spans)
            })
            .collect();

        let title = format!("Stack: {}", stack.len());
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    fn draw_memory(&self, f: &mut Frame<'_>, area: Rect) {
        let step = self.current_step();
        let memory = &step.memory;

        let min_len = hex_digits(memory.len());

        // Color memory region based on read/write.
        let mut offset = None;
        let mut size = None;
        let mut color = None;
        if let Instruction::OpCode(op) = step.instruction {
            let stack_len = step.stack.len();
            if stack_len > 0 {
                let (read_offset, read_size, write_offset, write_size) =
                    get_memory_access(op, &step.stack);
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
        if self.current_step > 0 {
            let prev_step = self.current_step - 1;
            let prev_step = &self.debug_steps()[prev_step];
            if let Instruction::OpCode(op) = prev_step.instruction {
                let (_, _, write_offset, write_size) = get_memory_access(op, &prev_step.stack);
                if write_offset.is_some() {
                    offset = write_offset;
                    size = write_size;
                    color = Some(Color::Green);
                }
            }
        }

        let height = area.height as usize;
        let end_line = self.draw_memory.current_mem_startline + height;

        let text: Vec<Line> = memory
            .chunks(32)
            .enumerate()
            .skip(self.draw_memory.current_mem_startline)
            .take_while(|(i, _)| *i < end_line)
            .map(|(i, mem_word)| {
                let mut spans = Vec::with_capacity(1 + 32 * 2 + 1 + 32 / 4 + 1);

                // Memory index.
                spans.push(Span::styled(
                    format!("{:0min_len$x}| ", i * 32),
                    Style::new().fg(Color::White),
                ));

                // Word hex bytes.
                hex_bytes_spans(mem_word, &mut spans, |j, _| {
                    let mut byte_color = Color::White;
                    if let (Some(offset), Some(size), Some(color)) = (offset, size, color) {
                        let idx = i * 32 + j;
                        if (offset..offset + size).contains(&idx) {
                            // [offset, offset + size] is the memory region to be colored.
                            // If a byte at row i and column j in the memory panel
                            // falls in this region, set the color.
                            byte_color = color;
                        }
                    }
                    Style::new().fg(byte_color)
                });

                if self.mem_utf {
                    spans.push(Span::raw("|"));
                    for utf in mem_word.chunks(4) {
                        if let Ok(utf_str) = std::str::from_utf8(utf) {
                            spans.push(Span::raw(utf_str.replace('\0', ".")));
                        } else {
                            spans.push(Span::raw("."));
                        }
                    }
                }

                spans.push(Span::raw("\n"));

                Line::from(spans)
            })
            .collect();

        let title = format!("Memory (max expansion: {} bytes)", memory.len());
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }
}

/// Wrapper around a list of [`Line`]s that prepends the line number on each new line.
struct SourceLines<'a> {
    lines: Vec<Line<'a>>,
    max_line_num: usize,
}

impl<'a> SourceLines<'a> {
    fn new(max_line_num: usize) -> Self {
        Self { lines: Vec::new(), max_line_num }
    }

    fn push(&mut self, line_number_style: Style, line: &'a str, line_style: Style) {
        self.push_raw(line_number_style, &[Span::styled(line, line_style)]);
    }

    fn push_raw(&mut self, line_number_style: Style, spans: &[Span<'a>]) {
        let mut line_spans = Vec::with_capacity(4);

        let line_number =
            format!("{number: >width$} ", number = self.lines.len() + 1, width = self.max_line_num);
        line_spans.push(Span::styled(line_number, line_number_style));

        // Space between line number and line text.
        line_spans.push(Span::raw("  "));

        line_spans.extend_from_slice(spans);

        self.lines.push(Line::from(line_spans));
    }
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
    stack: &[U256],
) -> (Option<usize>, Option<usize>, Option<usize>, Option<usize>) {
    let memory_access = match op {
        opcode::KECCAK256 | opcode::RETURN | opcode::REVERT => (1, 2, 0, 0),
        opcode::CALLDATACOPY | opcode::CODECOPY | opcode::RETURNDATACOPY => (0, 0, 1, 3),
        opcode::EXTCODECOPY => (0, 0, 2, 4),
        opcode::MLOAD => (1, -1, 0, 0),
        opcode::MSTORE => (0, 0, 1, -1),
        opcode::MSTORE8 => (0, 0, 1, -2),
        opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => (1, 2, 0, 0),
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
        1.. => {
            if (stack_index as usize) <= stack_len {
                Some(stack[stack_len - stack_index as usize].saturating_to())
            } else {
                None
            }
        }
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

fn hex_bytes_spans(bytes: &[u8], spans: &mut Vec<Span<'_>>, f: impl Fn(usize, u8) -> Style) {
    for (i, &byte) in bytes.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(alloy_primitives::hex::encode([byte]), f(i, byte)));
    }
}

/// Returns the number of decimal digits in the given number.
///
/// This is the same as `n.to_string().len()`.
fn decimal_digits(n: usize) -> usize {
    n.checked_ilog10().unwrap_or(0) as usize + 1
}

/// Returns the number of hexadecimal digits in the given number.
///
/// This is the same as `format!("{n:x}").len()`.
fn hex_digits(n: usize) -> usize {
    n.checked_ilog(16).unwrap_or(0) as usize + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn decimal_digits() {
        assert_eq!(super::decimal_digits(0), 1);
        assert_eq!(super::decimal_digits(1), 1);
        assert_eq!(super::decimal_digits(2), 1);
        assert_eq!(super::decimal_digits(9), 1);
        assert_eq!(super::decimal_digits(10), 2);
        assert_eq!(super::decimal_digits(11), 2);
        assert_eq!(super::decimal_digits(50), 2);
        assert_eq!(super::decimal_digits(99), 2);
        assert_eq!(super::decimal_digits(100), 3);
        assert_eq!(super::decimal_digits(101), 3);
        assert_eq!(super::decimal_digits(201), 3);
        assert_eq!(super::decimal_digits(999), 3);
        assert_eq!(super::decimal_digits(1000), 4);
        assert_eq!(super::decimal_digits(1001), 4);
    }

    #[test]
    fn hex_digits() {
        assert_eq!(super::hex_digits(0), 1);
        assert_eq!(super::hex_digits(1), 1);
        assert_eq!(super::hex_digits(2), 1);
        assert_eq!(super::hex_digits(9), 1);
        assert_eq!(super::hex_digits(10), 1);
        assert_eq!(super::hex_digits(11), 1);
        assert_eq!(super::hex_digits(15), 1);
        assert_eq!(super::hex_digits(16), 2);
        assert_eq!(super::hex_digits(17), 2);
        assert_eq!(super::hex_digits(0xff), 2);
        assert_eq!(super::hex_digits(0x100), 3);
        assert_eq!(super::hex_digits(0x101), 3);
    }
}
