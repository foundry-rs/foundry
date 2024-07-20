//! TUI draw implementation.

use super::context::{BufferKind, DebuggerContext};
use crate::op::OpcodeParam;
use alloy_primitives::U256;
use foundry_compilers::artifacts::sourcemap::SourceElement;
use foundry_evm_traces::debug::SourceData;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use revm::interpreter::opcode;
use revm_inspectors::tracing::types::CallKind;
use std::{collections::VecDeque, fmt::Write, io};

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
    /// |             buf             |
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
        let [app, footer] = Layout::new(
            Direction::Vertical,
            [Constraint::Ratio(100 - h_height, 100), Constraint::Ratio(h_height, 100)],
        )
        .split(area)[..] else {
            unreachable!()
        };

        // Split the app in 4 vertically to construct all the panes.
        let [op_pane, stack_pane, memory_pane, src_pane] = Layout::new(
            Direction::Vertical,
            [
                Constraint::Ratio(1, 6),
                Constraint::Ratio(1, 6),
                Constraint::Ratio(1, 6),
                Constraint::Ratio(3, 6),
            ],
        )
        .split(app)[..] else {
            unreachable!()
        };

        if self.show_shortcuts {
            self.draw_footer(f, footer);
        }
        self.draw_src(f, src_pane);
        self.draw_op_list(f, op_pane);
        self.draw_stack(f, stack_pane);
        self.draw_buffer(f, memory_pane);
    }

    /// Draws the layout in horizontal mode.
    ///
    /// ```text
    /// |-----------------|-----------|
    /// |        op       |   stack   |
    /// |-----------------|-----------|
    /// |                 |           |
    /// |       src       |    buf    |
    /// |                 |           |
    /// |-----------------|-----------|
    /// ```
    fn horizontal_layout(&self, f: &mut Frame<'_>) {
        let area = f.size();
        let h_height = if self.show_shortcuts { 4 } else { 0 };

        // Split off footer.
        let [app, footer] = Layout::new(
            Direction::Vertical,
            [Constraint::Ratio(100 - h_height, 100), Constraint::Ratio(h_height, 100)],
        )
        .split(area)[..] else {
            unreachable!()
        };

        // Split app in 2 horizontally.
        let [app_left, app_right] =
            Layout::new(Direction::Horizontal, [Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                .split(app)[..]
        else {
            unreachable!()
        };

        // Split left pane in 2 vertically to opcode list and source.
        let [op_pane, src_pane] =
            Layout::new(Direction::Vertical, [Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
                .split(app_left)[..]
        else {
            unreachable!()
        };

        // Split right pane horizontally to construct stack and memory.
        let [stack_pane, memory_pane] =
            Layout::new(Direction::Vertical, [Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
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
        self.draw_buffer(f, memory_pane);
    }

    fn draw_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let l1 = "[q]: quit | [k/j]: prev/next op | [a/s]: prev/next jump | [c/C]: prev/next call | [g/G]: start/end | [b]: cycle memory/calldata/returndata buffers";
        let l2 = "[t]: stack labels | [m]: buffer decoding | [shift + j/k]: scroll stack | [ctrl + j/k]: scroll buffer | ['<char>]: goto breakpoint | [h] toggle help";
        let dimmed = Style::new().add_modifier(Modifier::DIM);
        let lines =
            vec![Line::from(Span::styled(l1, dimmed)), Line::from(Span::styled(l2, dimmed))];
        let paragraph =
            Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    fn draw_src(&self, f: &mut Frame<'_>, area: Rect) {
        let (text_output, source_name) = self.src_text(area);
        let call_kind_text = match self.call_kind() {
            CallKind::Create | CallKind::Create2 => "Contract creation",
            CallKind::Call => "Contract call",
            CallKind::StaticCall => "Contract staticcall",
            CallKind::CallCode => "Contract callcode",
            CallKind::DelegateCall => "Contract delegatecall",
            CallKind::AuthCall => "Contract authcall",
            CallKind::EOFCreate => "EOF contract creation",
        };
        let title = format!(
            "{} {} ",
            call_kind_text,
            source_name.map(|s| format!("| {s}")).unwrap_or_default()
        );
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text_output).block(block).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    fn src_text(&self, area: Rect) -> (Text<'_>, Option<&str>) {
        let (source_element, source) = match self.src_map() {
            Ok(r) => r,
            Err(e) => return (Text::from(e), None),
        };

        // We are handed a vector of SourceElements that give us a span of sourcecode that is
        // currently being executed. This includes an offset and length.
        // This vector is in instruction pointer order, meaning the location of the instruction
        // minus `sum(push_bytes[..pc])`.
        let offset = source_element.offset() as usize;
        let len = source_element.length() as usize;
        let max = source.source.len();

        // Split source into before, relevant, and after chunks, split by line, for formatting.
        let actual_start = offset.min(max);
        let actual_end = (offset + len).min(max);

        let mut before: Vec<_> = source.source[..actual_start].split_inclusive('\n').collect();
        let actual: Vec<_> =
            source.source[actual_start..actual_end].split_inclusive('\n').collect();
        let mut after: VecDeque<_> = source.source[actual_end..].split_inclusive('\n').collect();

        let num_lines = before.len() + actual.len() + after.len();
        let height = area.height as usize;
        let needed_highlight = actual.len();
        let mid_len = before.len() + actual.len();

        // adjust what text we show of the source code
        let (start_line, end_line) = if needed_highlight > height {
            // highlighted section is more lines than we have available
            let start_line = before.len().saturating_sub(1);
            (start_line, before.len() + needed_highlight)
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

            // since above is subtracted from before.len(), and the resulting
            // start_line is used to index into before, above must be at least
            // 1 to avoid out-of-range accesses.
            if above == 0 {
                above = 1;
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

        let mut lines = SourceLines::new(start_line, end_line);

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

        // pad with empty to each line to ensure the previous text is cleared
        for line in &mut lines.lines {
            // note that the \n is not included in the line length
            if area.width as usize > line.width() + 1 {
                line.push_span(Span::raw(" ".repeat(area.width as usize - line.width() - 1)));
            }
        }

        (Text::from(lines.lines), source.path.to_str())
    }

    /// Returns source map, source code and source name of the current line.
    fn src_map(&self) -> Result<(SourceElement, &SourceData), String> {
        let address = self.address();
        let Some(contract_name) = self.debugger.identified_contracts.get(address) else {
            return Err(format!("Unknown contract at address {address}"));
        };

        self.debugger
            .contracts_sources
            .find_source_mapping(
                contract_name,
                self.current_step().pc,
                self.debug_call().kind.is_any_create(),
            )
            .ok_or_else(|| format!("No source map for contract {contract_name}"))
    }

    fn draw_op_list(&self, f: &mut Frame<'_>, area: Rect) {
        let debug_steps = self.debug_steps();
        let max_pc = debug_steps.iter().map(|step| step.pc).max().unwrap_or(0);
        let max_pc_len = hex_digits(max_pc);

        let items = debug_steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let mut content = String::with_capacity(64);
                write!(content, "{:0>max_pc_len$x}|", step.pc).unwrap();
                if let Some(op) = self.opcode_list.get(i) {
                    content.push_str(op);
                }
                ListItem::new(Span::styled(content, Style::new().fg(Color::White)))
            })
            .collect::<Vec<_>>();

        let title = format!(
            "Address: {} | PC: {} | Gas used in call: {} | Code section: {}",
            self.address(),
            self.current_step().pc,
            self.current_step().gas_used,
            self.current_step().code_section_idx,
        );
        let block = Block::default().title(title).borders(Borders::ALL);
        let list = List::new(items)
            .block(block)
            .highlight_symbol("▶")
            .highlight_style(Style::new().fg(Color::White).bg(Color::DarkGray))
            .scroll_padding(1);
        let mut state = ListState::default().with_selected(Some(self.current_step));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn draw_stack(&self, f: &mut Frame<'_>, area: Rect) {
        let step = self.current_step();
        let stack = step.stack.as_ref();
        let stack_len = stack.map_or(0, |s| s.len());

        let min_len = decimal_digits(stack_len).max(2);

        let params = OpcodeParam::of(step.op.get(), step.immediate_bytes.as_ref());

        let text: Vec<Line<'_>> = stack
            .map(|stack| {
                stack
                    .iter()
                    .rev()
                    .enumerate()
                    .skip(self.draw_memory.current_stack_startline)
                    .map(|(i, stack_item)| {
                        let param = params
                            .as_ref()
                            .and_then(|params| params.iter().find(|param| param.index == i));

                        let mut spans = Vec::with_capacity(1 + 32 * 2 + 3);

                        // Stack index.
                        spans.push(Span::styled(
                            format!("{i:0min_len$}| "),
                            Style::new().fg(Color::White),
                        ));

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
                    .collect()
            })
            .unwrap_or_default();

        let title = format!("Stack: {stack_len}");
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    fn draw_buffer(&self, f: &mut Frame<'_>, area: Rect) {
        let call = self.debug_call();
        let step = self.current_step();
        let buf = match self.active_buffer {
            BufferKind::Memory => step.memory.as_ref().unwrap().as_ref(),
            BufferKind::Calldata => call.calldata.as_ref(),
            BufferKind::Returndata => step.returndata.as_ref(),
        };

        let min_len = hex_digits(buf.len());

        // Color memory region based on read/write.
        let mut offset = None;
        let mut size = None;
        let mut write_offset = None;
        let mut write_size = None;
        let mut color = None;
        let stack_len = step.stack.as_ref().map_or(0, |s| s.len());
        if stack_len > 0 {
            if let Some(stack) = step.stack.as_ref() {
                if let Some(accesses) = get_buffer_accesses(step.op.get(), stack) {
                    if let Some(read_access) = accesses.read {
                        offset = Some(read_access.1.offset);
                        size = Some(read_access.1.size);
                        color = Some(Color::Cyan);
                    }
                    if let Some(write_access) = accesses.write {
                        if self.active_buffer == BufferKind::Memory {
                            write_offset = Some(write_access.offset);
                            write_size = Some(write_access.size);
                        }
                    }
                }
            }
        }

        // color word on previous write op
        // TODO: technically it's possible for this to conflict with the current op, ie, with
        // subsequent MCOPYs, but solc can't seem to generate that code even with high optimizer
        // settings
        if self.current_step > 0 {
            let prev_step = self.current_step - 1;
            let prev_step = &self.debug_steps()[prev_step];
            if let Some(stack) = prev_step.stack.as_ref() {
                if let Some(write_access) =
                    get_buffer_accesses(prev_step.op.get(), stack).and_then(|a| a.write)
                {
                    if self.active_buffer == BufferKind::Memory {
                        offset = Some(write_access.offset);
                        size = Some(write_access.size);
                        color = Some(Color::Green);
                    }
                }
            }
        }

        let height = area.height as usize;
        let end_line = self.draw_memory.current_buf_startline + height;

        let text: Vec<Line<'_>> = buf
            .chunks(32)
            .enumerate()
            .skip(self.draw_memory.current_buf_startline)
            .take_while(|(i, _)| *i < end_line)
            .map(|(i, buf_word)| {
                let mut spans = Vec::with_capacity(1 + 32 * 2 + 1 + 32 / 4 + 1);

                // Buffer index.
                spans.push(Span::styled(
                    format!("{:0min_len$x}| ", i * 32),
                    Style::new().fg(Color::White),
                ));

                // Word hex bytes.
                hex_bytes_spans(buf_word, &mut spans, |j, _| {
                    let mut byte_color = Color::White;
                    let mut end = None;
                    let idx = i * 32 + j;
                    if let (Some(offset), Some(size), Some(color)) = (offset, size, color) {
                        end = Some(offset + size);
                        if (offset..offset + size).contains(&idx) {
                            // [offset, offset + size] is the memory region to be colored.
                            // If a byte at row i and column j in the memory panel
                            // falls in this region, set the color.
                            byte_color = color;
                        }
                    }
                    if let (Some(write_offset), Some(write_size)) = (write_offset, write_size) {
                        // check for overlap with read region
                        let write_end = write_offset + write_size;
                        if let Some(read_end) = end {
                            let read_start = offset.unwrap();
                            if (write_offset..write_end).contains(&read_end) {
                                // if it contains end, start from write_start up to read_end
                                if (write_offset..read_end).contains(&idx) {
                                    return Style::new().fg(Color::Yellow);
                                }
                            } else if (write_offset..write_end).contains(&read_start) {
                                // otherwise if it contains read start, start from read_start up to
                                // write_end
                                if (read_start..write_end).contains(&idx) {
                                    return Style::new().fg(Color::Yellow);
                                }
                            }
                        }
                        if (write_offset..write_end).contains(&idx) {
                            byte_color = Color::Red;
                        }
                    }

                    Style::new().fg(byte_color)
                });

                if self.buf_utf {
                    spans.push(Span::raw("|"));
                    for utf in buf_word.chunks(4) {
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

        let title = self.active_buffer.title(buf.len());
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }
}

/// Wrapper around a list of [`Line`]s that prepends the line number on each new line.
struct SourceLines<'a> {
    lines: Vec<Line<'a>>,
    start_line: usize,
    max_line_num: usize,
}

impl<'a> SourceLines<'a> {
    fn new(start_line: usize, end_line: usize) -> Self {
        Self { lines: Vec::new(), start_line, max_line_num: decimal_digits(end_line) }
    }

    fn push(&mut self, line_number_style: Style, line: &'a str, line_style: Style) {
        self.push_raw(line_number_style, &[Span::styled(line, line_style)]);
    }

    fn push_raw(&mut self, line_number_style: Style, spans: &[Span<'a>]) {
        let mut line_spans = Vec::with_capacity(4);

        let line_number = format!(
            "{number: >width$} ",
            number = self.start_line + self.lines.len() + 1,
            width = self.max_line_num
        );
        line_spans.push(Span::styled(line_number, line_number_style));

        // Space between line number and line text.
        line_spans.push(Span::raw("  "));

        line_spans.extend_from_slice(spans);

        self.lines.push(Line::from(line_spans));
    }
}

/// Container for buffer access information.
struct BufferAccess {
    offset: usize,
    size: usize,
}

/// Container for read and write buffer access information.
struct BufferAccesses {
    /// The read buffer kind and access information.
    read: Option<(BufferKind, BufferAccess)>,
    /// The only mutable buffer is the memory buffer, so don't store the buffer kind.
    write: Option<BufferAccess>,
}

/// The memory_access variable stores the index on the stack that indicates the buffer
/// offset/size accessed by the given opcode:
///    (read buffer, buffer read offset, buffer read size, write memory offset, write memory size)
///    \>= 1: the stack index
///    0: no memory access
///    -1: a fixed size of 32 bytes
///    -2: a fixed size of 1 byte
///
/// The return value is a tuple about accessed buffer region by the given opcode:
///    (read buffer, buffer read offset, buffer read size, write memory offset, write memory size)
fn get_buffer_accesses(op: u8, stack: &[U256]) -> Option<BufferAccesses> {
    let buffer_access = match op {
        opcode::KECCAK256 | opcode::RETURN | opcode::REVERT => {
            (Some((BufferKind::Memory, 1, 2)), None)
        }
        opcode::CALLDATACOPY => (Some((BufferKind::Calldata, 2, 3)), Some((1, 3))),
        opcode::RETURNDATACOPY => (Some((BufferKind::Returndata, 2, 3)), Some((1, 3))),
        opcode::CALLDATALOAD => (Some((BufferKind::Calldata, 1, -1)), None),
        opcode::CODECOPY => (None, Some((1, 3))),
        opcode::EXTCODECOPY => (None, Some((2, 4))),
        opcode::MLOAD => (Some((BufferKind::Memory, 1, -1)), None),
        opcode::MSTORE => (None, Some((1, -1))),
        opcode::MSTORE8 => (None, Some((1, -2))),
        opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
            (Some((BufferKind::Memory, 1, 2)), None)
        }
        opcode::CREATE | opcode::CREATE2 => (Some((BufferKind::Memory, 2, 3)), None),
        opcode::CALL | opcode::CALLCODE => (Some((BufferKind::Memory, 4, 5)), None),
        opcode::DELEGATECALL | opcode::STATICCALL => (Some((BufferKind::Memory, 3, 4)), None),
        opcode::MCOPY => (Some((BufferKind::Memory, 2, 3)), Some((1, 3))),
        opcode::RETURNDATALOAD => (Some((BufferKind::Returndata, 1, -1)), None),
        opcode::EOFCREATE => (Some((BufferKind::Memory, 3, 4)), None),
        opcode::RETURNCONTRACT => (Some((BufferKind::Memory, 1, 2)), None),
        opcode::DATACOPY => (None, Some((1, 3))),
        opcode::EXTCALL | opcode::EXTSTATICCALL | opcode::EXTDELEGATECALL => {
            (Some((BufferKind::Memory, 2, 3)), None)
        }
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

    if buffer_access.0.is_some() || buffer_access.1.is_some() {
        let (read, write) = buffer_access;
        let read_access = read.and_then(|b| {
            let (buffer, offset, size) = b;
            Some((buffer, BufferAccess { offset: get_size(offset)?, size: get_size(size)? }))
        });
        let write_access = write.and_then(|b| {
            let (offset, size) = b;
            Some(BufferAccess { offset: get_size(offset)?, size: get_size(size)? })
        });
        Some(BufferAccesses { read: read_access, write: write_access })
    } else {
        None
    }
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
