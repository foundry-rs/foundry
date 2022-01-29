use ethers::abi::Abi;
use std::{
    cmp::{max, min},
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};
use tui::text::Text;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ethers::solc::artifacts::ContractBytecodeSome;
use std::{
    io::{self},
    sync::mpsc,
    thread,
};

use evm_adapters::sputnik::cheatcodes::debugger::DebugStep;
use eyre::Result;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};

use ethers::types::Address;

/// Trait for starting the ui
pub trait Ui {
    /// Start the agent that will now take over.
    fn start(self) -> Result<TUIExitReason>;
}

/// Used to indicate why the Ui stopped
pub enum TUIExitReason {
    /// 'q' exit
    CharExit,
}

pub struct Tui {
    debug_arena: Vec<(Address, Vec<DebugStep>, bool)>,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    /// Buffer for keys prior to execution, i.e. '10' + 'k' => move up 10 operations
    key_buffer: String,
    /// current step in the debug steps
    current_step: usize,
    identified_contracts: BTreeMap<Address, (String, Abi)>,
    known_contracts: BTreeMap<String, ContractBytecodeSome>,
    source_code: BTreeMap<u32, String>,
}

impl Tui {
    /// Create a tui
    #[allow(unused_must_use)]
    pub fn new(
        debug_arena: Vec<(Address, Vec<DebugStep>, bool)>,
        current_step: usize,
        identified_contracts: BTreeMap<Address, (String, Abi)>,
        known_contracts: BTreeMap<String, ContractBytecodeSome>,
        source_code: BTreeMap<u32, String>,
    ) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor();
        Ok(Tui {
            debug_arena,
            terminal,
            key_buffer: String::new(),
            current_step,
            identified_contracts,
            known_contracts,
            source_code,
        })
    }

    /// Grab number from buffer. Used for something like '10k' to move up 10 operations
    fn buffer_as_number(buffer: &str, default_value: usize) -> usize {
        if let Ok(num) = buffer.parse() {
            if num >= 1 {
                num
            } else {
                default_value
            }
        } else {
            default_value
        }
    }

    /// Create layout and subcomponents
    #[allow(clippy::too_many_arguments)]
    fn draw_layout<B: Backend>(
        f: &mut Frame<B>,
        address: Address,
        identified_contracts: &BTreeMap<Address, (String, Abi)>,
        known_contracts: &BTreeMap<String, ContractBytecodeSome>,
        source_code: &BTreeMap<u32, String>,
        debug_steps: &[DebugStep],
        opcode_list: &[String],
        current_step: usize,
        creation: bool,
        draw_memory: &mut DrawMemory,
    ) {
        let total_size = f.size();

        // split in 2 vertically

        if let [app, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(98, 100), Constraint::Ratio(2, 100)].as_ref())
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
                        Tui::draw_footer(f, footer);
                        Tui::draw_src(
                            f,
                            address,
                            identified_contracts,
                            known_contracts,
                            source_code,
                            debug_steps[current_step].ic,
                            creation,
                            src_pane,
                        );
                        Tui::draw_op_list(
                            f,
                            address,
                            debug_steps,
                            opcode_list,
                            current_step,
                            draw_memory,
                            op_pane,
                        );
                        Tui::draw_stack(f, debug_steps, current_step, stack_pane);
                        Tui::draw_memory(f, debug_steps, current_step, memory_pane);
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

    fn draw_footer<B: Backend>(f: &mut Frame<B>, area: Rect) {
        let block_controls = Block::default();

        let text_output = Text::from(Span::styled(
            "[q]: Quit | [k/j]: prev/next op | [a/s]: prev/next jump | [c/C]: prev/next call | [g/G]: start/end",
            Style::default().add_modifier(Modifier::DIM)
        ));
        let paragraph = Paragraph::new(text_output)
            .block(block_controls)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_src<B: Backend>(
        f: &mut Frame<B>,
        address: Address,
        identified_contracts: &BTreeMap<Address, (String, Abi)>,
        known_contracts: &BTreeMap<String, ContractBytecodeSome>,
        source_code: &BTreeMap<u32, String>,
        ic: usize,
        creation: bool,
        area: Rect,
    ) {
        let block_source_code = Block::default()
            .title(format!("Contract construction: {}", creation))
            .borders(Borders::ALL);

        let mut text_output: Text = Text::from("");

        if let Some(contract_name) = identified_contracts.get(&address) {
            if let Some(known) = known_contracts.get(&contract_name.0) {
                // grab either the creation source map or runtime sourcemap
                if let Some(sourcemap) = if creation {
                    known.bytecode.source_map()
                } else {
                    known.deployed_bytecode.bytecode.as_ref().expect("no bytecode").source_map()
                } {
                    match sourcemap {
                        Ok(sourcemap) => {
                            // we are handed a vector of SourceElements that give
                            // us a span of sourcecode that is currently being executed
                            // This includes an offset and length. This vector is in
                            // instruction pointer order, meaning the location of
                            // the instruction - sum(push_bytes[..pc])
                            if let Some(source_idx) = sourcemap[ic].index {
                                if let Some(source) = source_code.get(&source_idx) {
                                    let offset = sourcemap[ic].offset;
                                    let len = sourcemap[ic].length;

                                    // split source into before, relevant, and after chunks
                                    // split by line as well to do some formatting stuff
                                    let mut before = source[..offset]
                                        .split_inclusive('\n')
                                        .collect::<Vec<&str>>();
                                    let actual = source[offset..offset + len]
                                        .split_inclusive('\n')
                                        .map(|s| s.to_string())
                                        .collect::<Vec<String>>();
                                    let mut after = source[offset + len..]
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
                                                text_output.lines.push(Spans::from(vec![
                                                    Span::styled(
                                                        format!(
                                                            "{: >max_line_num$}",
                                                            line_number.to_string(),
                                                            max_line_num = max_line_num
                                                        ),
                                                        Style::default()
                                                            .fg(Color::Gray)
                                                            .bg(Color::DarkGray),
                                                    ),
                                                    Span::styled(
                                                        "\u{2800} ".to_string() + line,
                                                        Style::default()
                                                            .add_modifier(Modifier::DIM),
                                                    ),
                                                ]));
                                                line_number += 1;
                                            });

                                            text_output.lines.push(Spans::from(vec![
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
                                                    Style::default()
                                                        .fg(Color::Cyan)
                                                        .add_modifier(Modifier::BOLD),
                                                ),
                                            ]));
                                            line_number += 1;

                                            actual.iter().skip(1).for_each(|s| {
                                                text_output.lines.push(Spans::from(vec![
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
                                                text_output.lines.push(Spans::from(vec![
                                                    Span::styled(
                                                        format!(
                                                            "{: >max_line_num$}",
                                                            line_number.to_string(),
                                                            max_line_num = max_line_num
                                                        ),
                                                        Style::default()
                                                            .fg(Color::Gray)
                                                            .bg(Color::DarkGray),
                                                    ),
                                                    Span::styled(
                                                        "\u{2800} ".to_string() + line,
                                                        Style::default()
                                                            .add_modifier(Modifier::DIM),
                                                    ),
                                                ]));

                                                line_number += 1;
                                            });
                                            actual.iter().for_each(|s| {
                                                text_output.lines.push(Spans::from(vec![
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
                                            text_output.lines.push(Spans::from(vec![
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

                                    // fill in the rest of the line as unhighlighted
                                    if let Some(last) = actual.last() {
                                        if !last.ends_with('\n') {
                                            if let Some(post) = after.pop_front() {
                                                if let Some(last) = text_output.lines.last_mut() {
                                                    last.0.push(Span::raw(post));
                                                }
                                            }
                                        }
                                    }

                                    // add after highlighted text
                                    while mid_len + after.len() > end_line {
                                        after.pop_back();
                                    }
                                    after.iter().for_each(|line| {
                                        text_output.lines.push(Spans::from(vec![
                                            Span::styled(
                                                format!(
                                                    "{: >max_line_num$}",
                                                    line_number.to_string(),
                                                    max_line_num = max_line_num
                                                ),
                                                Style::default()
                                                    .fg(Color::Gray)
                                                    .bg(Color::DarkGray),
                                            ),
                                            Span::styled(
                                                "\u{2800} ".to_string() + line,
                                                Style::default().add_modifier(Modifier::DIM),
                                            ),
                                        ]));
                                        line_number += 1;
                                    });
                                } else {
                                    text_output.extend(Text::from("No source for srcmap index"));
                                }
                            } else {
                                text_output.extend(Text::from("No srcmap index"));
                            }
                        }
                        Err(e) => text_output.extend(Text::from(format!(
                            "Error in source map parsing: '{}', please open an issue",
                            e
                        ))),
                    }
                } else {
                    text_output.extend(Text::from("No sourcemap for contract"));
                }
            } else {
                text_output.extend(Text::from(format!("Unknown contract at address {}", address)));
            }
        } else {
            text_output.extend(Text::from(format!("Unknown contract at address {}", address)));
        }

        let paragraph =
            Paragraph::new(text_output).block(block_source_code).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    /// Draw opcode list into main component
    fn draw_op_list<B: Backend>(
        f: &mut Frame<B>,
        address: Address,
        debug_steps: &[DebugStep],
        opcode_list: &[String],
        current_step: usize,
        draw_memory: &mut DrawMemory,
        area: Rect,
    ) {
        let block_source_code = Block::default()
            .title(format!(
                " Address: {}, pc: {}, call's gas used: {} ",
                address,
                if let Some(step) = debug_steps.get(current_step) {
                    step.pc.to_string()
                } else {
                    "END".to_string()
                },
                debug_steps[current_step].total_gas_used,
            ))
            .borders(Borders::ALL);
        let mut text_output: Vec<Spans> = Vec::new();

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
                format!("{:0>max_pc_len$x} ▶", step.pc, max_pc_len = max_pc_len)
            } else if line_number < debug_steps.len() {
                let step: &DebugStep = &debug_steps[line_number];
                format!("{:0>max_pc_len$x}: ", step.pc, max_pc_len = max_pc_len)
            } else {
                "END CALL".to_string()
            };

            if let Some(op) = opcode_list.get(line_number) {
                text_output.push(Spans::from(Span::styled(
                    format!("{} {}", line_number_format, op),
                    Style::default().fg(Color::White).bg(bg_color),
                )));
            } else {
                text_output.push(Spans::from(Span::styled(
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
    fn draw_stack<B: Backend>(
        f: &mut Frame<B>,
        debug_steps: &[DebugStep],
        current_step: usize,
        area: Rect,
    ) {
        let stack_space =
            Block::default().title(format!(" Stack: {} ", current_step)).borders(Borders::ALL);
        let stack = &debug_steps[current_step].stack;
        let min_len = usize::max(format!("{}", stack.len()).len(), 2);

        let text: Vec<Spans> = stack
            .iter()
            .enumerate()
            .map(|(i, stack_item)| {
                Spans::from(Span::styled(
                    format!("{: <min_len$}: {:?} \n", i, stack_item, min_len = min_len),
                    Style::default().fg(Color::White),
                ))
            })
            .collect();
        let paragraph = Paragraph::new(text).block(stack_space).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    // cargo r --manifest-path ../foundry/Cargo.toml --bin forge run ./src/test/Locke.t.sol -t
    // StreamTest --sig "test_fundStream()" --debug

    /// Draw memory in memory pane
    fn draw_memory<B: Backend>(
        f: &mut Frame<B>,
        debug_steps: &[DebugStep],
        current_step: usize,
        area: Rect,
    ) {
        let memory = &debug_steps[current_step].memory;
        let stack_space = Block::default()
            .title(format!(" Memory - Max Expansion: {} bytes", memory.effective_len()))
            .borders(Borders::ALL);
        let memory = memory.data();
        let max_i = memory.len() / 32;
        let min_len = format!("{:x}", max_i * 32).len();

        let text: Vec<Spans> = memory
            .chunks(32)
            .enumerate()
            .map(|(i, mem_word)| {
                let strings: String = mem_word
                    .chunks(4)
                    .map(|bytes4| {
                        bytes4
                            .iter()
                            .map(|byte| {
                                let v: Vec<u8> = vec![*byte];
                                hex::encode(&v[..])
                            })
                            .collect::<Vec<String>>()
                            .join(" ")
                    })
                    .collect::<Vec<String>>()
                    .join("  ");
                Spans::from(Span::styled(
                    format!("{:0min_len$x}: {} \n", i * 32, strings, min_len = min_len),
                    Style::default().fg(Color::White),
                ))
            })
            .collect();
        let paragraph = Paragraph::new(text).block(stack_space).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }
}

impl Ui for Tui {
    fn start(mut self) -> Result<TUIExitReason> {
        // if something panics inside here, we should do everything we can to
        // not corrupt the user's terminal.
        std::panic::set_hook(Box::new(|e| {
            disable_raw_mode().expect("Unable to disable raw mode");
            execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
                .expect("unable to execute disable mouse capture");
            println!("{}", e);
        }));
        // this is the recommend tick rate from tui-rs, based on their examples
        let tick_rate = Duration::from_millis(200);

        // setup a channel to send interrupts
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                // poll events since last tick
                if event::poll(tick_rate - last_tick.elapsed()).unwrap() {
                    let event = event::read().unwrap();
                    if let Event::Key(key) = event {
                        if tx.send(Interrupt::KeyPressed(key)).is_err() {
                            return
                        }
                    } else if let Event::Mouse(mouse) = event {
                        if tx.send(Interrupt::MouseEvent(mouse)).is_err() {
                            return
                        }
                    }
                }
                // force update if time has passed
                if last_tick.elapsed() > tick_rate {
                    if tx.send(Interrupt::IntervalElapsed).is_err() {
                        return
                    }
                    last_tick = Instant::now();
                }
            }
        });

        self.terminal.clear()?;
        let mut draw_memory: DrawMemory = DrawMemory::default();

        let debug_call: Vec<(Address, Vec<DebugStep>, bool)> = self.debug_arena.clone();
        let mut opcode_list: Vec<String> =
            debug_call[0].1.iter().map(|step| step.pretty_opcode()).collect();
        let mut last_index = 0;
        // UI thread that manages drawing
        loop {
            if last_index != draw_memory.inner_call_index {
                opcode_list = debug_call[draw_memory.inner_call_index]
                    .1
                    .iter()
                    .map(|step| step.pretty_opcode())
                    .collect();
                last_index = draw_memory.inner_call_index;
            }
            // grab interrupt
            match rx.recv()? {
                // key press
                Interrupt::KeyPressed(event) => match event.code {
                    // Exit
                    KeyCode::Char('q') => {
                        disable_raw_mode()?;
                        execute!(
                            self.terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        return Ok(TUIExitReason::CharExit)
                    }
                    // Move down
                    KeyCode::Char('j') | KeyCode::Down => {
                        // grab number of times to do it
                        for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
                            if self.current_step < opcode_list.len() - 1 {
                                self.current_step += 1;
                            } else if draw_memory.inner_call_index < debug_call.len() - 1 {
                                draw_memory.inner_call_index += 1;
                                self.current_step = 0;
                            }
                        }
                        self.key_buffer.clear();
                    }
                    // Move up
                    KeyCode::Char('k') | KeyCode::Up => {
                        for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
                            if self.current_step > 0 {
                                self.current_step -= 1;
                            } else if draw_memory.inner_call_index > 0 {
                                draw_memory.inner_call_index -= 1;
                                self.current_step =
                                    debug_call[draw_memory.inner_call_index].1.len() - 1;
                            }
                        }
                        self.key_buffer.clear();
                    }
                    // Go to top of file
                    KeyCode::Char('g') => {
                        draw_memory.inner_call_index = 0;
                        self.current_step = 0;
                        self.key_buffer.clear();
                    }
                    // Go to bottom of file
                    KeyCode::Char('G') => {
                        draw_memory.inner_call_index = debug_call.len() - 1;
                        self.current_step = debug_call[draw_memory.inner_call_index].1.len() - 1;
                        self.key_buffer.clear();
                    }
                    // Go to previous call
                    KeyCode::Char('c') => {
                        draw_memory.inner_call_index =
                            draw_memory.inner_call_index.saturating_sub(1);
                        self.current_step = debug_call[draw_memory.inner_call_index].1.len() - 1;
                        self.key_buffer.clear();
                    }
                    // Go to next call
                    KeyCode::Char('C') => {
                        if debug_call.len() > draw_memory.inner_call_index + 1 {
                            draw_memory.inner_call_index += 1;
                            self.current_step = 0;
                        }
                        self.key_buffer.clear();
                    }
                    // Step forward
                    KeyCode::Char('s') => {
                        for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
                            let remaining_ops = opcode_list[self.current_step..].to_vec().clone();
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
                                .unwrap_or(opcode_list.len() - 1);
                            if self.current_step > opcode_list.len() {
                                self.current_step = opcode_list.len() - 1
                            };
                        }
                        self.key_buffer.clear();
                    }
                    // Step backwards
                    KeyCode::Char('a') => {
                        for _ in 0..Tui::buffer_as_number(&self.key_buffer, 1) {
                            let prev_ops = opcode_list[..self.current_step].to_vec().clone();
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
                    KeyCode::Char(other) => match other {
                        '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => {
                            self.key_buffer.push(other);
                        }
                        _ => {
                            // Invalid key, clear buffer
                            self.key_buffer.clear();
                        }
                    },
                    _ => {
                        self.key_buffer.clear();
                    }
                },
                Interrupt::MouseEvent(event) => match event.kind {
                    MouseEventKind::ScrollUp => {
                        if self.current_step > 0 {
                            self.current_step -= 1;
                        } else if draw_memory.inner_call_index > 0 {
                            draw_memory.inner_call_index -= 1;
                            self.current_step =
                                debug_call[draw_memory.inner_call_index].1.len() - 1;
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if self.current_step < opcode_list.len() - 1 {
                            self.current_step += 1;
                        } else if draw_memory.inner_call_index < debug_call.len() - 1 {
                            draw_memory.inner_call_index += 1;
                            self.current_step = 0;
                        }
                    }
                    _ => {}
                },
                Interrupt::IntervalElapsed => {}
            }
            // Draw
            let current_step = self.current_step;
            self.terminal.draw(|f| {
                Tui::draw_layout(
                    f,
                    debug_call[draw_memory.inner_call_index].0,
                    &self.identified_contracts,
                    &self.known_contracts,
                    &self.source_code,
                    &debug_call[draw_memory.inner_call_index].1[..],
                    &opcode_list,
                    current_step,
                    debug_call[draw_memory.inner_call_index].2,
                    &mut draw_memory,
                )
            })?;
        }
    }
}

/// Why did we wake up drawing thread?
enum Interrupt {
    KeyPressed(KeyEvent),
    MouseEvent(MouseEvent),
    IntervalElapsed,
}

/// This is currently used to remember last scroll
/// position so screen doesn't wiggle as much.
struct DrawMemory {
    pub current_startline: usize,
    pub inner_call_index: usize,
}
impl DrawMemory {
    fn default() -> Self {
        DrawMemory { current_startline: 0, inner_call_index: 0 }
    }
}
