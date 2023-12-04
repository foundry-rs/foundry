//! TUI draw implementation.

use super::context::DebuggerContext;
use crate::op::OpcodeParam;
use alloy_primitives::U256;
use foundry_evm_core::{debug::Instruction, utils::CallKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use revm::interpreter::opcode;
use std::{cmp, collections::VecDeque, io};

impl DebuggerContext<'_> {
    /// Draws the TUI layout and subcomponents to the given terminal.
    pub(crate) fn draw(&self, terminal: &mut super::DebuggerTerminal) -> io::Result<()> {
        terminal.draw(|f| self.draw_layout(f)).map(drop)
    }

    fn draw_layout(&self, f: &mut Frame<'_>) {
        if f.size().width < 225 {
            self.vertical_layout(f);
        } else {
            self.square_layout(f);
        }
    }

    fn vertical_layout(&self, f: &mut Frame<'_>) {
        let total_size = f.size();
        let h_height = if self.show_shortcuts { 4 } else { 0 };

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
                if self.show_shortcuts {
                    Self::draw_footer(f, footer);
                }
                self.draw_src(f, src_pane);
                self.draw_op_list(f, op_pane);
                self.draw_stack(f, stack_pane);
                self.draw_memory(f, memory_pane);
            } else {
                panic!("unable to create vertical panes")
            }
        } else {
            panic!("unable to create footer / app")
        };
    }

    fn square_layout(&self, f: &mut Frame<'_>) {
        let total_size = f.size();
        let h_height = if self.show_shortcuts { 4 } else { 0 };

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
                        if self.show_shortcuts {
                            Self::draw_footer(f, footer)
                        };
                        self.draw_src(f, src_pane);
                        self.draw_op_list(f, op_pane);
                        self.draw_stack(f, stack_pane);
                        self.draw_memory(f, memory_pane);
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

    fn draw_src(&self, f: &mut Frame<'_>, area: Rect) {
        let block_source_code = Block::default()
            .title(match self.call_kind() {
                CallKind::Create | CallKind::Create2 => "Contract creation",
                CallKind::Call => "Contract call",
                CallKind::StaticCall => "Contract staticcall",
                CallKind::CallCode => "Contract callcode",
                CallKind::DelegateCall => "Contract delegatecall",
            })
            .borders(Borders::ALL);

        let mut text_output: Text = Text::from("");

        let pc = self.current_step().pc;
        if let Some(contract_name) = self.debugger.identified_contracts.get(self.address()) {
            if let Some(files_source_code) = self.debugger.contracts_sources.0.get(contract_name) {
                let pc_ic_map = self.debugger.pc_ic_maps.get(contract_name);
                // find the contract source with the correct source_element's file_id
                if let Some((source_element, source_code)) = files_source_code.iter().find_map(
                    |(file_id, (source_code, contract_source))| {
                        // grab either the creation source map or runtime sourcemap
                        if let Some((Ok(source_map), ic)) =
                            if matches!(self.call_kind(), CallKind::Create | CallKind::Create2) {
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
            let address = self.address();
            text_output.extend(Text::from(format!("Unknown contract at address {address}")));
        }

        let paragraph =
            Paragraph::new(text_output).block(block_source_code).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    /// Draw opcode list into main component
    fn draw_op_list(&self, f: &mut Frame<'_>, area: Rect) {
        let step = self.current_step();
        let block_source_code = Block::default()
            .title(format!(
                "Address: {} | PC: {} | Gas used in call: {}",
                self.address(),
                step.pc,
                step.total_gas_used,
            ))
            .borders(Borders::ALL);
        let mut text_output: Vec<Line> = Vec::new();

        // Scroll:
        // Focused line is line that should always be at the center of the screen.
        let display_start;

        let height = area.height as i32;
        let extra_top_lines = height / 2;
        let prev_start = *self.draw_memory.current_startline.borrow();
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

        if prev_start < min_start {
            display_start = min_start;
        } else if prev_start > max_start {
            display_start = max_start;
        } else {
            display_start = prev_start;
        }
        *self.draw_memory.current_startline.borrow_mut() = display_start;

        let max_pc_len =
            self.debug_steps().iter().fold(0, |max_val, val| val.pc.max(max_val)).to_string().len();

        // Define closure that prints one more line of source code
        let mut add_new_line = |line_number| {
            let is_current_step = line_number == self.current_step;
            let bg_color = if is_current_step { Color::DarkGray } else { Color::Reset };

            // Format line number
            let line_number_format = if is_current_step {
                format!("{:0>max_pc_len$x}|▶", step.pc)
            } else if line_number < self.debug_steps().len() {
                format!("{:0>max_pc_len$x}| ", step.pc)
            } else {
                "END CALL".to_string()
            };

            if let Some(op) = self.opcode_list.get(line_number) {
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
        for number in display_start..self.opcode_list.len() {
            add_new_line(number);
        }
        // Add one more "phantom" line so we see line where current segment execution ends
        add_new_line(self.opcode_list.len());
        let paragraph =
            Paragraph::new(text_output).block(block_source_code).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// Draw the stack into the stack pane
    fn draw_stack(&self, f: &mut Frame<'_>, area: Rect) {
        let step = self.current_step();
        let stack = &step.stack;
        let stack_space =
            Block::default().title(format!("Stack: {}", stack.len())).borders(Borders::ALL);
        let min_len = usize::max(format!("{}", stack.len()).len(), 2);

        let params =
            if let Instruction::OpCode(op) = step.instruction { OpcodeParam::of(op) } else { &[] };

        let text: Vec<Line> = stack
            .iter()
            .rev()
            .enumerate()
            .skip(self.draw_memory.current_stack_startline)
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

                if self.stack_labels {
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
        stack: &[U256],
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

    /// Draw memory in memory pane
    fn draw_memory(&self, f: &mut Frame<'_>, area: Rect) {
        let step = self.current_step();
        let memory = &step.memory;
        let memory_space = Block::default()
            .title(format!("Memory (max expansion: {} bytes)", memory.len()))
            .borders(Borders::ALL);
        let max_i = memory.len() / 32;
        let min_len = format!("{:x}", max_i * 32).len();

        // color memory region based on write/read
        let mut offset: Option<usize> = None;
        let mut size: Option<usize> = None;
        let mut color = None;
        if let Instruction::OpCode(op) = step.instruction {
            let stack_len = step.stack.len();
            if stack_len > 0 {
                let (read_offset, read_size, write_offset, write_size) =
                    Self::get_memory_access(op, &step.stack);
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
                let (_, _, write_offset, write_size) =
                    Self::get_memory_access(op, &prev_step.stack);
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
            .take_while(|(i, _)| i < &end_line)
            .map(|(i, mem_word)| {
                let words: Vec<Span> = mem_word
                    .iter()
                    .enumerate()
                    .map(|(j, byte)| {
                        Span::styled(
                            format!("{byte:02x} "),
                            if let (Some(offset), Some(size), Some(color)) = (offset, size, color) {
                                if i * 32 + j >= offset && i * 32 + j < offset + size {
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

                if self.mem_utf {
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
