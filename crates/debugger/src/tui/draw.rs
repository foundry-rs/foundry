//! TUI draw implementation.

use super::context::{ActiveInternalCallCache, ActiveInternalCallLocation, StatusKind, TUIContext};
use crate::{DebuggerLayout, debugger::DebuggerStats, op::OpcodeParam};
use alloy_dyn_abi::{DynSolType, Specifier, parser::Parameters};
use alloy_primitives::{Address, U256, keccak256};
use foundry_common::fmt::format_token;
use foundry_compilers::artifacts::sourcemap::SourceElement;
use foundry_evm_core::buffer::{BufferKind, get_buffer_accesses};
use foundry_evm_traces::debug::{
    DebugSourceScope, DebugVariable, SourceData, decode_step_parameters,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use revm_inspectors::tracing::types::{
    CallKind, CallTraceStep, DecodedInternalCall, DecodedTraceStep,
};
use std::{collections::VecDeque, fmt::Write};

impl TUIContext<'_> {
    pub(crate) fn draw_layout(&mut self, f: &mut Frame<'_>) {
        // We need 100 columns to display a 32 byte word in the memory and stack panes.
        let area = f.area();
        let min_width = 100;
        let min_height = 16;
        if area.width < min_width || area.height < min_height {
            self.size_too_small(f, min_width, min_height);
            return;
        }

        match self.layout() {
            DebuggerLayout::Horizontal => self.horizontal_layout(f),
            DebuggerLayout::Vertical => self.vertical_layout(f),
            DebuggerLayout::Auto => {
                // The horizontal layout draws these panes at 50% width.
                let min_column_width_for_horizontal = 200;
                if area.width >= min_column_width_for_horizontal {
                    self.horizontal_layout(f);
                } else {
                    self.vertical_layout(f);
                }
            }
        }
    }

    fn size_too_small(&self, f: &mut Frame<'_>, min_width: u16, min_height: u16) {
        let mut lines = Vec::with_capacity(4);

        let l1 = "Terminal size too small:";
        lines.push(Line::from(l1));

        let area = f.area();
        let width_color = if area.width >= min_width { Color::Green } else { Color::Red };
        let height_color = if area.height >= min_height { Color::Green } else { Color::Red };
        let l2 = vec![
            Span::raw("Width = "),
            Span::styled(area.width.to_string(), Style::new().fg(width_color)),
            Span::raw(" Height = "),
            Span::styled(area.height.to_string(), Style::new().fg(height_color)),
        ];
        lines.push(Line::from(l2));

        let l3 = "Needed for current config:";
        lines.push(Line::from(l3));
        let l4 = format!("Width = {min_width} Height = {min_height}");
        lines.push(Line::from(l4));

        let paragraph =
            Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area)
    }

    /// Draws the layout in vertical mode.
    ///
    /// ```text
    /// |-----------------------------|
    /// |             op              |
    /// |-----------------------------|
    /// |          variables          |
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
    fn vertical_layout(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        let footer_height = self.footer_height();

        // NOTE: `Layout::split` always returns a slice of the same length as the number of
        // constraints, so the `else` branch is unreachable.

        // Split off footer.
        let [app, footer] = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(footer_height)],
        )
        .split(area)[..] else {
            unreachable!()
        };

        // Split the app vertically to construct all the panes.
        let [op_pane, variables_pane, stack_pane, memory_pane, src_pane] = Layout::new(
            Direction::Vertical,
            [
                Constraint::Ratio(1, 7),
                Constraint::Ratio(1, 7),
                Constraint::Ratio(1, 7),
                Constraint::Ratio(1, 7),
                Constraint::Ratio(3, 7),
            ],
        )
        .split(app)[..] else {
            unreachable!()
        };

        if footer_height > 0 {
            self.draw_footer(f, footer);
        }
        self.draw_src(f, src_pane);
        self.draw_op_list(f, op_pane);
        self.draw_variables(f, variables_pane);
        self.draw_stack(f, stack_pane);
        self.draw_buffer(f, memory_pane);
    }

    /// Draws the layout in horizontal mode.
    ///
    /// ```text
    /// |-----------------|-----------|
    /// |        op       | variables |
    /// |-----------------|-----------|
    /// |                 |   stack   |
    /// |       src       |-----------|
    /// |                 |           |
    /// |                 |    buf    |
    /// |                 |           |
    /// |-----------------|-----------|
    /// ```
    fn horizontal_layout(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        let footer_height = self.footer_height();

        // Split off footer.
        let [app, footer] = Layout::new(
            Direction::Vertical,
            [Constraint::Min(0), Constraint::Length(footer_height)],
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

        // Split right pane vertically to construct variables, stack and memory.
        let [variables_pane, stack_pane, memory_pane] = Layout::new(
            Direction::Vertical,
            [Constraint::Ratio(1, 4), Constraint::Ratio(1, 4), Constraint::Ratio(2, 4)],
        )
        .split(app_right)[..] else {
            unreachable!()
        };

        if footer_height > 0 {
            self.draw_footer(f, footer);
        }
        self.draw_src(f, src_pane);
        self.draw_op_list(f, op_pane);
        self.draw_variables(f, variables_pane);
        self.draw_stack(f, stack_pane);
        self.draw_buffer(f, memory_pane);
    }

    fn footer_height(&self) -> u16 {
        let status_or_input = u16::from(self.pc_input.is_some() || self.status.is_some());
        let shortcuts = if self.show_shortcuts { 2 } else { 0 };
        status_or_input + shortcuts
    }

    fn draw_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let mut lines = Vec::with_capacity(self.footer_height() as usize);

        if let Some(input) = &self.pc_input {
            lines.push(Line::from(vec![
                Span::styled(
                    "Goto PC: ",
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::raw(input.as_str()),
                Span::styled("█", Style::new().fg(Color::Cyan)),
                Span::styled(
                    "  Enter: jump | Esc: cancel | hex: 0x2a/2a | decimal: d:42",
                    Style::new().add_modifier(Modifier::DIM),
                ),
            ]));
        } else if let Some(status) = &self.status {
            let style = match status.kind {
                StatusKind::Info => Style::new().fg(Color::Green),
                StatusKind::Error => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            };
            lines.push(Line::from(Span::styled(status.text.as_str(), style)));
        }

        let l1 = "[q]: quit | [k/j]: prev/next op | [a/s]: prev/next jump | [c/C]: prev/next call | [g/G]: start/end | [p]: goto PC | [b]: cycle memory/calldata/returndata buffers";
        let l2 = "[l]: layout | [t]: stack labels | [m]: buffer decoding | [shift + j/k]: scroll stack | [ctrl + j/k]: scroll buffer | ['<char>]: goto breakpoint | [h] toggle help";
        let dimmed = Style::new().add_modifier(Modifier::DIM);
        if self.show_shortcuts {
            lines.push(Line::from(Span::styled(l1, dimmed)));
            lines.push(Line::from(Span::styled(l2, dimmed)));
        }

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

            let first = if last_has_nl {
                0
            } else {
                lines.push_raw(h_num, &[Span::raw(last), Span::styled(actual[0], h_text)]);
                1
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
        if let Some(last) = actual.last()
            && !last.ends_with('\n')
            && let Some(post) = after.pop_front()
            && let Some(last) = lines.lines.last_mut()
        {
            last.spans.push(Span::raw(post));
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

        let step = self.current_step();
        let call_gas_used = self.debug_call().gas_limit.saturating_sub(step.gas_remaining);
        let title = op_list_title(
            self.address(),
            step.pc,
            step.gas_remaining,
            call_gas_used,
            step.gas_refund_counter,
            self.debugger_context.stats,
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

        let params = OpcodeParam::of(step.op.get());

        let text: Vec<Line<'_>> = stack
            .map(|stack| {
                stack
                    .iter()
                    .rev()
                    .enumerate()
                    .skip(self.draw_memory.current_stack_startline)
                    .map(|(i, stack_item)| {
                        let param = params.iter().find(|param| param.index == i);
                        stack_item_line(i, min_len, stack_item, param, self.stack_labels)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let title = format!("Stack: {stack_len}");
        let block = Block::default().title(title).borders(Borders::ALL);
        let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    fn draw_variables(&mut self, f: &mut Frame<'_>, area: Rect) {
        let variables = self.scope_variables();
        let known = variables.iter().filter(|variable| variable.value.is_some()).count();
        let title = if variables.is_empty() {
            "Variables".to_string()
        } else {
            format!("Variables: {known}/{}", variables.len())
        };

        let text = if variables.is_empty() {
            vec![Line::from(Span::styled(
                "No variables in current scope",
                Style::new().add_modifier(Modifier::DIM),
            ))]
        } else {
            variables.into_iter().map(scope_variable_line).collect()
        };

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
        let mut len = None;
        let mut write_offset = None;
        let mut write_size = None;
        let mut color = None;
        let stack_len = step.stack.as_ref().map_or(0, |s| s.len());
        if stack_len > 0
            && let Some(stack) = step.stack.as_ref()
            && let Some(accesses) = get_buffer_accesses(step.op.get(), stack)
        {
            if let Some(read_access) = accesses.read {
                offset = Some(read_access.1.offset);
                len = Some(read_access.1.len);
                color = Some(Color::Cyan);
            }
            if let Some(write_access) = accesses.write
                && self.active_buffer == BufferKind::Memory
            {
                write_offset = Some(write_access.offset);
                write_size = Some(write_access.len);
            }
        }

        // color word on previous write op
        // TODO: technically it's possible for this to conflict with the current op, ie, with
        // subsequent MCOPYs, but solc can't seem to generate that code even with high optimizer
        // settings
        if self.current_step > 0 {
            let prev_step = self.current_step - 1;
            let prev_step = &self.debug_steps()[prev_step];
            if let Some(stack) = prev_step.stack.as_ref()
                && let Some(write_access) =
                    get_buffer_accesses(prev_step.op.get(), stack).and_then(|a| a.write)
                && self.active_buffer == BufferKind::Memory
            {
                offset = Some(write_access.offset);
                len = Some(write_access.len);
                color = Some(Color::Green);
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
                    if let (Some(offset), Some(len), Some(color)) = (offset, len, color) {
                        end = Some(offset + len);
                        if (offset..offset + len).contains(&idx) {
                            // [offset, offset + len] is the memory region to be colored.
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ScopeVariable {
    kind: ScopeVariableKind,
    name: String,
    value: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeVariableKind {
    Parameter,
    Return,
    Local,
}

struct ActiveInternalCall<'a> {
    trace_node_idx: usize,
    entry_step: usize,
    end_step: usize,
    decoded: &'a DecodedInternalCall,
}

impl ScopeVariableKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Parameter => "param",
            Self::Return => "return",
            Self::Local => "local",
        }
    }

    const fn color(self) -> Color {
        match self {
            Self::Parameter => Color::Cyan,
            Self::Return => Color::Green,
            Self::Local => Color::White,
        }
    }
}

impl TUIContext<'_> {
    fn scope_variables(&mut self) -> Vec<ScopeVariable> {
        let (scope, start) = {
            let Ok((source_element, source)) = self.src_map() else {
                return Vec::new();
            };
            let start = source_element.offset() as usize;
            let end = start.saturating_add(source_element.length() as usize);
            let Some(scope) = source.find_debug_scope(start, end) else {
                return Vec::new();
            };
            (scope.clone(), start)
        };

        let parameter_values = self.decode_parameter_values(&scope);
        let return_values = self.decode_return_values(&scope);
        let mut variables = Vec::new();

        variables.extend(scope.parameters.iter().enumerate().map(|(i, variable)| ScopeVariable {
            kind: ScopeVariableKind::Parameter,
            name: variable_name(variable, i, "arg"),
            value: parameter_values.as_ref().and_then(|values| values.get(i).cloned()),
        }));

        variables.extend(scope.returns.iter().enumerate().map(|(i, variable)| ScopeVariable {
            kind: ScopeVariableKind::Return,
            name: variable_name(variable, i, "ret"),
            value: return_values.as_ref().and_then(|values| values.get(i).cloned()),
        }));

        variables.extend(scope.visible_locals(start).enumerate().map(|(i, variable)| {
            ScopeVariable {
                kind: ScopeVariableKind::Local,
                name: variable_name(variable, i, "local"),
                value: None,
            }
        }));

        variables
    }

    fn decode_parameter_values(&mut self, scope: &DebugSourceScope) -> Option<Vec<String>> {
        let scope_signature = scope_function_signature(scope);
        self.decode_internal_parameter_values(scope)
            .or_else(|| decode_external_parameter_values(scope, &self.debug_call().calldata))
            .or_else(|| {
                self.debug_call().decoded.as_ref().and_then(|decoded| {
                    let call_data = decoded.call_data.as_ref()?;
                    scope_signature
                        .as_deref()
                        .is_some_and(|signature| signature == call_data.signature)
                        .then(|| call_data.args.clone())
                })
            })
    }

    fn decode_return_values(&mut self, scope: &DebugSourceScope) -> Option<Vec<String>> {
        let current_step = self.absolute_current_step();
        self.active_internal_call().and_then(|active| {
            (current_step >= active.end_step
                && decoded_internal_name_matches(&active.decoded.func_name, scope))
            .then(|| active.decoded.return_data.clone())
            .flatten()
        })
    }

    fn decode_internal_parameter_values(
        &mut self,
        scope: &DebugSourceScope,
    ) -> Option<Vec<String>> {
        let (args, trace_node_idx, entry_step) = {
            let active = self.active_internal_call()?;
            if !decoded_internal_name_matches(&active.decoded.func_name, scope) {
                return None;
            }

            (active.decoded.args.clone(), active.trace_node_idx, active.entry_step)
        };

        if let Some(args) = args {
            return Some(args);
        }

        let parameters = Parameters::parse(&scope.parameters_src).ok()?;
        let step = self.step_by_absolute_index(trace_node_idx, entry_step)?;
        decode_step_parameters(&parameters, step)
    }

    fn active_internal_call(&mut self) -> Option<ActiveInternalCall<'_>> {
        let current_node_idx = self.draw_memory.inner_call_index;
        let trace_node_idx = self.debug_call().trace_node_idx;
        let current_step = self.absolute_current_step();
        let location = if let Some(cache) = self.draw_memory.active_internal_call
            && cache.matches(current_node_idx, trace_node_idx, current_step)
        {
            cache.location
        } else {
            let location =
                self.find_active_internal_call(current_node_idx, trace_node_idx, current_step);
            self.draw_memory.active_internal_call = Some(ActiveInternalCallCache {
                current_node_idx,
                trace_node_idx,
                absolute_step: current_step,
                location,
            });
            location
        }?;

        self.active_internal_call_at(location)
    }

    fn find_active_internal_call(
        &self,
        current_node_idx: usize,
        trace_node_idx: usize,
        current_step: usize,
    ) -> Option<ActiveInternalCallLocation> {
        let mut active = None;

        for (node_idx, node) in
            self.debug_arena().iter().enumerate().take(current_node_idx.saturating_add(1))
        {
            if node.trace_node_idx != trace_node_idx {
                continue;
            }

            for (step_idx, step) in node.steps.iter().enumerate() {
                let marker_step = node.step_offset.saturating_add(step_idx);
                if marker_step > current_step {
                    break;
                }

                let Some(decoded) = step.decoded.as_deref() else { continue };
                let DecodedTraceStep::InternalCall(_, end_step) = decoded else { continue };
                if current_step <= *end_step {
                    active = Some(ActiveInternalCallLocation {
                        trace_node_idx,
                        marker_node_idx: node_idx,
                        marker_step_idx: step_idx,
                        entry_step: marker_step.saturating_add(1),
                        end_step: *end_step,
                    });
                }
            }
        }

        active
    }

    fn active_internal_call_at(
        &self,
        location: ActiveInternalCallLocation,
    ) -> Option<ActiveInternalCall<'_>> {
        let step = self
            .debug_arena()
            .get(location.marker_node_idx)?
            .steps
            .get(location.marker_step_idx)?;
        let DecodedTraceStep::InternalCall(decoded, end_step) = step.decoded.as_deref()? else {
            return None;
        };
        (*end_step == location.end_step).then_some(ActiveInternalCall {
            trace_node_idx: location.trace_node_idx,
            entry_step: location.entry_step,
            end_step: location.end_step,
            decoded,
        })
    }

    fn step_by_absolute_index(
        &self,
        trace_node_idx: usize,
        absolute_step: usize,
    ) -> Option<&CallTraceStep> {
        self.debug_arena()
            .iter()
            .filter(|node| node.trace_node_idx == trace_node_idx)
            .find_map(|node| node.steps.get(absolute_step.checked_sub(node.step_offset)?))
    }

    fn absolute_current_step(&self) -> usize {
        self.debug_call().step_offset.saturating_add(self.current_step)
    }
}

fn scope_variable_line(variable: ScopeVariable) -> Line<'static> {
    let color = variable.kind.color();
    let mut spans = Vec::with_capacity(6);
    spans.push(Span::styled(variable.kind.label(), Style::new().fg(Color::Gray)));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(variable.name, Style::new().fg(color).add_modifier(Modifier::BOLD)));
    spans.push(Span::raw(" = "));
    if let Some(value) = variable.value {
        spans.push(Span::styled(value, Style::new().fg(color)));
    } else {
        spans.push(Span::styled("<unavailable>", Style::new().fg(Color::Gray)));
    }
    Line::from(spans)
}

fn variable_name(variable: &DebugVariable, index: usize, fallback_prefix: &str) -> String {
    variable
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{fallback_prefix}{index}"))
}

fn decoded_internal_name_matches(decoded_name: &str, scope: &DebugSourceScope) -> bool {
    if let Some((contract_name, function_name)) = decoded_name.rsplit_once("::") {
        return contract_name == scope.contract_name && function_name == scope.function_name;
    }
    decoded_name == scope.function_name
}

fn decode_external_parameter_values(
    scope: &DebugSourceScope,
    calldata: &[u8],
) -> Option<Vec<String>> {
    if calldata.len() < 4 {
        return None;
    }

    let parameters = Parameters::parse(&scope.parameters_src).ok()?;
    let types = resolved_types(&parameters)?;
    let selector = function_selector(&scope.function_name, &types);
    if calldata.get(..4)? != selector.as_slice() {
        return None;
    }

    decode_abi_sequence(&types, &calldata[4..])
}

fn resolved_types(parameters: &Parameters<'_>) -> Option<Vec<DynSolType>> {
    parameters.params.iter().map(|param| param.resolve().ok()).collect()
}

fn scope_function_signature(scope: &DebugSourceScope) -> Option<String> {
    let parameters = Parameters::parse(&scope.parameters_src).ok()?;
    let types = resolved_types(&parameters)?;
    Some(function_signature(&scope.function_name, &types))
}

fn function_selector(function_name: &str, types: &[DynSolType]) -> [u8; 4] {
    let signature = function_signature(function_name, types);
    keccak256(signature.as_bytes())[..4].try_into().unwrap()
}

fn function_signature(function_name: &str, types: &[DynSolType]) -> String {
    let mut signature = String::new();
    signature.push_str(function_name);
    signature.push('(');
    for (i, ty) in types.iter().enumerate() {
        if i > 0 {
            signature.push(',');
        }
        signature.push_str(&ty.sol_type_name());
    }
    signature.push(')');
    signature
}

fn decode_abi_sequence(types: &[DynSolType], data: &[u8]) -> Option<Vec<String>> {
    if types.is_empty() {
        return Some(Vec::new());
    }

    let value = DynSolType::Tuple(types.to_vec()).abi_decode_sequence(data).ok()?;
    let values = value.as_fixed_seq()?;
    Some(values.iter().map(format_token).collect())
}

fn op_list_title(
    address: &Address,
    pc: usize,
    gas_remaining: u64,
    call_gas_used: u64,
    gas_refund_counter: u64,
    stats: Option<DebuggerStats>,
) -> String {
    let address = full_checksum_address(address);
    let mut title = format!(
        "address: {address} | pc: 0x{pc:x} ({pc}) | gasLeft: {gas_remaining} | \
         callGasUsed: {call_gas_used} | gasRefund: {gas_refund_counter}"
    );

    if let Some(stats) = stats {
        write!(
            title,
            " | sessionTraceGasUsed: {} | sessionSubcalls: {}",
            stats.session_trace_gas_used, stats.session_subcalls
        )
        .unwrap();
    }

    title
}

fn full_checksum_address(address: &Address) -> String {
    address.to_string()
}

fn stack_item_line(
    i: usize,
    min_len: usize,
    stack_item: &U256,
    param: Option<&OpcodeParam>,
    stack_labels: bool,
) -> Line<'static> {
    let value_style =
        if param.is_some() { Style::new().fg(Color::Cyan) } else { Style::new().fg(Color::White) };
    let mut spans = Vec::with_capacity(1 + 32 * 2 + 5);

    // Stack index.
    spans.push(Span::styled(format!("{i:0min_len$}| "), Style::new().fg(Color::White)));

    // Item hex bytes.
    hex_bytes_spans(&stack_item.to_be_bytes::<32>(), &mut spans, |_, _| value_style);

    spans.push(Span::raw(" | "));
    spans.push(Span::styled(stack_item.to_string(), value_style));

    if stack_labels && let Some(param) = param {
        spans.push(Span::raw(" | "));
        spans.push(Span::raw(param.name));
    }

    spans.push(Span::raw("\n"));

    Line::from(spans)
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
    use super::TUIContext;
    use crate::{
        DebugNode,
        debugger::{DebuggerContext, DebuggerStats},
        op::OpcodeParam,
    };
    use alloy_dyn_abi::parser::Parameters;
    use alloy_primitives::{Address, Bytes, U256, address};
    use foundry_evm_core::Breakpoints;
    use foundry_evm_traces::debug::{ContractSources, DebugSourceScope, DebugVariable};
    use ratatui::{
        style::{Color, Style},
        text::Line,
    };
    use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};
    use revm_inspectors::tracing::types::{
        CallKind, CallTraceStep, DecodedCallData, DecodedCallTrace, DecodedInternalCall,
        DecodedTraceStep,
    };

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn scope(function_name: &str, parameters_src: &str) -> DebugSourceScope {
        DebugSourceScope {
            contract_name: "DebugMe".to_string(),
            function_name: function_name.to_string(),
            range: 0..100,
            body_range: 10..90,
            parameters_src: parameters_src.to_string(),
            returns_src: None,
            parameters: Vec::new(),
            returns: Vec::new(),
            locals: Vec::new(),
        }
    }

    fn trace_step(stack: Vec<U256>) -> CallTraceStep {
        CallTraceStep {
            pc: 0,
            op: OpCode::STOP,
            stack: Some(stack.into_boxed_slice()),
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

    fn internal_call_step(end_step: usize, return_data: Vec<String>) -> CallTraceStep {
        let mut step = trace_step(Vec::new());
        step.decoded = Some(Box::new(DecodedTraceStep::InternalCall(
            DecodedInternalCall {
                func_name: "DebugMe::foo".to_string(),
                args: Some(Vec::new()),
                return_data: Some(return_data),
            },
            end_step,
        )));
        step
    }

    fn internal_call_step_without_args(end_step: usize) -> CallTraceStep {
        let mut step = trace_step(Vec::new());
        step.decoded = Some(Box::new(DecodedTraceStep::InternalCall(
            DecodedInternalCall {
                func_name: "DebugMe::foo".to_string(),
                args: None,
                return_data: None,
            },
            end_step,
        )));
        step
    }

    fn debug_node(
        trace_node_idx: usize,
        step_offset: usize,
        steps: Vec<CallTraceStep>,
    ) -> DebugNode {
        let mut node = DebugNode::new(Address::ZERO, CallKind::Call, steps, Bytes::new(), 0, None);
        node.trace_node_idx = trace_node_idx;
        node.step_offset = step_offset;
        node
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

    fn abi_word(value: U256) -> [u8; 32] {
        value.to_be_bytes::<32>()
    }

    #[test]
    fn decode_external_parameter_values_decodes_named_params() {
        let scope = scope("foo", "(uint256 amount, bool ok)");
        let parameters = Parameters::parse(&scope.parameters_src).unwrap();
        let types = super::resolved_types(&parameters).unwrap();
        let mut calldata = Vec::new();
        calldata.extend_from_slice(&super::function_selector(&scope.function_name, &types));
        calldata.extend_from_slice(&abi_word(U256::from(42)));
        calldata.extend_from_slice(&abi_word(U256::from(1)));

        let values = super::decode_external_parameter_values(&scope, &calldata).unwrap();

        assert_eq!(values, ["42", "true"]);
    }

    #[test]
    fn decode_external_parameter_values_rejects_selector_mismatch() {
        let scope = scope("foo", "(uint256 amount)");
        let parameters = Parameters::parse("(uint256 amount)").unwrap();
        let types = super::resolved_types(&parameters).unwrap();
        let mut calldata = Vec::new();
        calldata.extend_from_slice(&super::function_selector("bar", &types));
        calldata.extend_from_slice(&abi_word(U256::from(42)));

        assert_eq!(super::decode_external_parameter_values(&scope, &calldata), None);
    }

    #[test]
    fn scope_function_signature_includes_resolved_parameter_types() {
        let scope = scope("foo", "(uint256 amount, bool ok)");

        assert_eq!(super::scope_function_signature(&scope).as_deref(), Some("foo(uint256,bool)"));
    }

    #[test]
    fn decode_parameter_values_rejects_decoded_call_data_for_wrong_overload() {
        let mut node = debug_node(0, 0, vec![trace_step(Vec::new())]);
        node.decoded = Some(Box::new(DecodedCallTrace {
            call_data: Some(DecodedCallData {
                signature: "foo(address)".to_string(),
                args: vec!["0x000000000000000000000000000000000000002a".to_string()],
            }),
            ..Default::default()
        }));
        let mut context = context_with_arena(vec![node]);
        let mut tui = TUIContext::new(&mut context);

        assert_eq!(tui.decode_parameter_values(&scope("foo", "(uint256 amount)")), None);
    }

    #[test]
    fn decode_step_parameters_reads_static_values_from_stack() {
        let step = trace_step(vec![U256::from(42), U256::from(1)]);
        let parameters = Parameters::parse("(uint256 amount, bool ok)").unwrap();
        let values = super::decode_step_parameters(&parameters, &step).unwrap();

        assert_eq!(values, ["42", "true"]);
    }

    #[test]
    fn decode_internal_parameter_values_uses_absolute_entry_step() {
        let mut context = context_with_arena(vec![
            debug_node(0, 0, vec![internal_call_step_without_args(2)]),
            debug_node(1, 0, vec![trace_step(Vec::new())]),
            debug_node(0, 1, vec![trace_step(vec![U256::from(42)])]),
        ]);
        let mut tui = TUIContext::new(&mut context);
        tui.draw_memory.inner_call_index = 2;

        assert_eq!(
            tui.decode_internal_parameter_values(&scope("foo", "(uint256 amount)")),
            Some(vec!["42".to_string()])
        );
    }

    #[test]
    fn decode_return_values_uses_absolute_internal_call_end_step() {
        let mut context = context_with_arena(vec![debug_node(
            0,
            3,
            vec![internal_call_step(4, vec!["99".to_string()]), trace_step(Vec::new())],
        )]);
        let mut tui = TUIContext::new(&mut context);
        tui.current_step = 1;

        assert_eq!(tui.decode_return_values(&scope("foo", "()")), Some(vec!["99".to_string()]));
    }

    #[test]
    fn decode_return_values_finds_internal_call_split_by_child_node() {
        let mut context = context_with_arena(vec![
            debug_node(0, 0, vec![internal_call_step(2, vec!["7".to_string()])]),
            debug_node(1, 0, vec![trace_step(Vec::new())]),
            debug_node(0, 2, vec![trace_step(Vec::new())]),
        ]);
        let mut tui = TUIContext::new(&mut context);
        tui.draw_memory.inner_call_index = 2;

        assert_eq!(tui.decode_return_values(&scope("foo", "()")), Some(vec!["7".to_string()]));
    }

    #[test]
    fn active_internal_call_caches_by_current_node_and_step() {
        let mut context = context_with_arena(vec![debug_node(
            0,
            0,
            vec![internal_call_step(2, vec!["1".to_string()]), trace_step(Vec::new())],
        )]);
        let mut tui = TUIContext::new(&mut context);

        assert!(tui.active_internal_call().is_some());
        let cache = tui.draw_memory.active_internal_call;
        assert!(cache.and_then(|cache| cache.location).is_some());

        assert!(tui.active_internal_call().is_some());
        assert_eq!(tui.draw_memory.active_internal_call, cache);

        tui.current_step = 1;
        assert!(tui.active_internal_call().is_some());
        assert_ne!(tui.draw_memory.active_internal_call, cache);
    }

    #[test]
    fn decoded_internal_name_matches_exact_contract_and_function() {
        let scope = scope("foo", "()");

        assert!(super::decoded_internal_name_matches("DebugMe::foo", &scope));
        assert!(!super::decoded_internal_name_matches("DebugMe::barfoo", &scope));
        assert!(!super::decoded_internal_name_matches("Other::foo", &scope));
    }

    #[test]
    fn scope_variable_line_marks_unavailable_locals() {
        let variable = super::ScopeVariable {
            kind: super::ScopeVariableKind::Local,
            name: "sum".to_string(),
            value: None,
        };

        assert_eq!(line_text(&super::scope_variable_line(variable)), "local sum = <unavailable>");
    }

    #[test]
    fn variable_name_falls_back_for_unnamed_values() {
        let variable = DebugVariable { name: None, declaration: 0..1, scope: 0..2 };

        assert_eq!(super::variable_name(&variable, 2, "arg"), "arg2");
    }

    #[test]
    fn op_list_title_includes_gas_and_subcall_stats() {
        let stats = DebuggerStats { session_trace_gas_used: 789_012, session_subcalls: 3 };
        let address = Address::from([0x42; 20]);
        let title = super::op_list_title(&address, 0x2a, 123_456, 42, 7, Some(stats));

        assert!(title.contains("pc: 0x2a (42)"));
        assert!(title.contains(&format!("address: {}", super::full_checksum_address(&address))));
        assert!(title.contains("gasLeft: 123456"));
        assert!(title.contains("sessionTraceGasUsed: 789012"));
        assert!(title.contains("sessionSubcalls: 3"));
        assert!(title.contains("callGasUsed: 42"));
        assert!(title.contains("gasRefund: 7"));
    }

    #[test]
    fn op_list_title_omits_aggregate_stats_when_unavailable() {
        let title = super::op_list_title(&Address::from([0x42; 20]), 0x2a, 123_456, 42, 7, None);

        assert!(!title.contains("sessionTraceGasUsed"));
        assert!(!title.contains("sessionSubcalls"));
    }

    #[test]
    fn op_list_title_uses_full_checksum_address() {
        let address = address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
        let title = super::op_list_title(&address, 0x2a, 123_456, 42, 7, None);

        assert!(title.contains("address: 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"));
        assert!(!title.contains('…'));
    }

    #[test]
    fn stack_item_line_includes_decimal_preview() {
        let line = super::stack_item_line(0, 2, &U256::from(42), None, false);
        let text = line_text(&line);

        assert!(text.starts_with("00| "));
        assert!(text.ends_with("2a | 42\n"));
    }

    #[test]
    fn stack_item_line_keeps_stack_labels_after_decimal_preview() {
        let param = OpcodeParam { name: "offset", index: 0 };
        let line = super::stack_item_line(0, 2, &U256::from(16), Some(&param), true);

        assert!(line_text(&line).ends_with("10 | 16 | offset\n"));
    }

    #[test]
    fn stack_item_line_highlights_decimal_preview_for_opcode_params() {
        let param = OpcodeParam { name: "offset", index: 0 };
        let line = super::stack_item_line(0, 2, &U256::from(16), Some(&param), false);
        let decimal = line.spans.iter().find(|span| span.content.as_ref() == "16").unwrap();

        assert_eq!(decimal.style, Style::new().fg(Color::Cyan));
    }

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
