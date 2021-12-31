use eyre::WrapErr;
use std::time::Duration;
use std::time::Instant;
use std::cmp::{max, min};

use std::io::{self};
use std::thread;
use std::sync::mpsc;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use tui::backend::{Backend, CrosstermBackend};
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::terminal::Frame;
use tui::widgets::{Block, Borders, Paragraph, Wrap};
use tui::text::{Span, Spans};
use tui::Terminal;
use evm_adapters::sputnik::cheatcodes::debugger::DebugStep;
use eyre::Result;

/// This trait describes structure that takes care of
/// interacting with user.
pub trait UiAgent {
    /// Start the agent that will now take over.
    fn start(self) -> Result<ApplicationExitReason>;
}

/// Used to indicate why did UiAgent stop
pub enum ApplicationExitReason {
    /// User wants to exit the application
    UserExit,
}


pub struct Tui {
    debug_steps: Vec<DebugStep>,
    opcode_list: Vec<String>,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    /// Pressed keys that should be stored but can't be processed now.
    /// For example, user can press "10k". The "1" and "0" are important and should
    /// be stored, but can't be executed because we don't know what to do (move up) until
    /// we see the "k" character. The instant we see it, we read the whole buffer and clear it.
    pressed_keys_buffer: String,
    /// Remembers at which state are we currently. User can step back and forth.
    current_step: usize,
}

impl Tui {
    /// Create new TUI that gathers data from the debugger.
    ///
    /// This consumes the debugger, as it's used to advance debugging state.
    #[allow(unused_must_use)]
    // NOTE: We don't care that some actions here fail (for example mouse handling),
    // as some features that we're trying to enable here are not necessary for desed.
    pub fn new(debug_steps: Vec<DebugStep>, current_step: usize) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.hide_cursor();
        let opcode_list = debug_steps.iter().map(|step| step.pretty_opcode()).collect();
        Ok(Tui {
            debug_steps,
            opcode_list,
            terminal,
            pressed_keys_buffer: String::new(),
            current_step
        })
    }

    /// Reads given buffer and returns it as a number.
    ///
    /// A default value will be return if the number is non-parsable (typically empty buffer) or is
    /// not at least 1.
    fn get_pressed_key_buffer_as_number(buffer: &String, default_value: usize) -> usize {
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

    /// Generate layout and call individual draw methods for each layout part.
    fn draw_layout_and_subcomponents<B: Backend>(
        f: &mut Frame<B>,
        debug_steps: Vec<DebugStep>,
        opcode_list: Vec<String>,
        // Line (0-based) which user has selected via cursor
        current_step: usize,
        draw_memory: &mut DrawMemory,
    ) {
        let total_size = f.size();

        if let [left_plane, right_plane] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
            .split(total_size)[..]
        {
            if let [stack_plane, memory_plane] = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Ratio(1, 2),
                        Constraint::Ratio(1, 2),
                    ]
                    .as_ref(),
                )
                .split(right_plane)[..]
            {
                Tui::draw_op_list(
                    f,
                    opcode_list,
                    current_step,
                    draw_memory,
                    left_plane,
                );
                Tui::draw_stack(
                    f,
                    &debug_steps,
                    current_step,
                    stack_plane,
                );
                Tui::draw_memory(
                    f,
                    &debug_steps,
                    current_step,
                    memory_plane,
                );
            } else {
                panic!("Failed to generate vertically split layout 1:1:1:1.");
            }
        } else {
            panic!("Failed to generate horizontally split layout 1:2.");
        }
    }

    /// Draw Opcode list into main window.
    ///
    /// Handles scrolling and breakpoint display as well.
    ///
    /// TODO: syntax highlighting
    fn draw_op_list<B: Backend>(
        f: &mut Frame<B>,
        opcode_list: Vec<String>,
        // Line (0-based) which user has selected via cursor
        current_step: usize,
        draw_memory: &mut DrawMemory,
        area: Rect,
    ) {
        let block_source_code = Block::default()
            .title(format!(" Op: {} - q: quit, a: JUMPDEST-, s: JUMPDEST+, j: OP+, k: OP-, g: OP0, G: OP_LAST", current_step))
            .borders(Borders::ALL);
        let mut text_output: Vec<Spans> = Vec::new();

        // Scroll:
        // Focused line is line that should always be at the center of the screen.
        let display_start;
        {
            let grace_lines = 10;
            let height = area.height as i32;
            let previous_startline = draw_memory.current_startline;
            // Minimum startline that should be possible to have in any case
            let minimum_startline = 0;
            // Maximum startline that should be possible to have in any case
            // Magical number 4: I don't know what it's doing here, but it works this way. Otherwise
            // we just keep maximum scroll four lines early.
            let maximum_startline = (opcode_list.len() as i32 - 1) - height + 4;
            // Minimum startline position that makes sense - we want visible code but within limits of the source code height.
            let mut minimum_viable_startline = max(
                current_step as i32 - height + grace_lines,
                minimum_startline,
            ) as usize;
            // Maximum startline position that makes sense - we want visible code but within limits of the source code height
            let mut maximum_viable_startline = max(
                min(current_step as i32 - grace_lines, maximum_startline),
                minimum_startline,
            ) as usize;
            // Sometimes, towards end of file, maximum and minim viable lines have swapped values.
            // No idea why, but swapping them helps the problem.
            if minimum_viable_startline > maximum_viable_startline {
                minimum_viable_startline ^= maximum_viable_startline;
                maximum_viable_startline ^= minimum_viable_startline;
                minimum_viable_startline ^= maximum_viable_startline;
            }
            // Try to keep previous startline as it was, but scroll up or down as
            // little as possible to keep within bonds
            if previous_startline < minimum_viable_startline {
                display_start = minimum_viable_startline;
            } else if previous_startline > maximum_viable_startline {
                display_start = maximum_viable_startline;
            } else {
                display_start = previous_startline;
            }
            draw_memory.current_startline = display_start;
        }

        // Define closure that prints one more line of source code
        let mut add_new_line = |line_number| {
            // Define background color depending on whether we have cursor here
            let linenr_bg_color = if line_number == current_step {
                Color::DarkGray
            } else {
                Color::Reset
            };
            // Format line indicator. It's different if the currently executing line is here
            let linenr_format = if line_number == current_step {
                format!("{: <3} ▶", (line_number + 1))
            } else {
                format!("{: <4}", (line_number + 1))
            };

            if let Some(op) = opcode_list.get(line_number) {
                text_output.push(Spans::from(Span::styled(
                    format!("{} {} \n", linenr_format, op),
                    Style::default().fg(Color::White).bg(linenr_bg_color)
                )));
            }
        };
        for number in display_start..opcode_list.len() {
            add_new_line(number);
        }
        // Add one more "phantom" line so we see line where current segment execution ends
        add_new_line(opcode_list.len());
        let paragraph = Paragraph::new(text_output)
            .block(block_source_code)
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// Draw stack.
    fn draw_stack<B: Backend>(f: &mut Frame<B>, debug_steps: &Vec<DebugStep>, current_step: usize, area: Rect) {
        let stack_space = Block::default()
            .title(format!(" Stack: {} ", current_step))
            .borders(Borders::ALL);
        let stack = &debug_steps[current_step].stack;
        let min_len = usize::max(format!("{}", stack.len()).len(), 2);

        let text: Vec<Spans> = stack.iter().enumerate().map(|(i, stack_item)| {
            Spans::from(Span::styled(
                format!("{: <min_len$}: {:?} \n", i, stack_item, min_len=min_len),
                Style::default().fg(Color::White)
            ))
        }).collect();
        let paragraph = Paragraph::new(text)
            .block(stack_space)
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// Draw stack.
    fn draw_memory<B: Backend>(f: &mut Frame<B>, debug_steps: &Vec<DebugStep>, current_step: usize, area: Rect) {
        let stack_space = Block::default()
            .title(format!(" Memory: {} ", current_step))
            .borders(Borders::ALL);
        let memory = &debug_steps[current_step].memory.data();
        let max_i = memory.len() / 32;
        let min_len = format!("{:x}", max_i*32).len();

        let text: Vec<Spans> = memory.chunks(32).enumerate().map(|(i, mem_word)| {
            let strings: String = mem_word.chunks(4).map(|bytes4| {
                bytes4.into_iter().map(|byte| {
                    let v: Vec<u8> = vec![*byte];
                    format!("{}", hex::encode(&v[..]))
                }).collect::<Vec<String>>().join(" ")
            }).collect::<Vec<String>>().join("  ");
            Spans::from(Span::styled(
                format!("{:0min_len$x}: {} \n", i*32, strings, min_len=min_len),
                Style::default().fg(Color::White)
            ))
        }).collect();
        let paragraph = Paragraph::new(text)
            .block(stack_space)
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// Use crossterm and stdout to restore terminal state.
    ///
    /// This shall be called on application exit.
    #[allow(unused_must_use)]
    // NOTE: We don't care if we fail to do something here. Terminal might not support everything,
    // but we try to restore as much as we can.
    pub fn restore_terminal_state() -> Result<()> {
        let mut stdout = io::stdout();
        // Disable mouse control
        execute!(stdout, event::DisableMouseCapture);
        // Disable raw mode that messes up with user's terminal and show cursor again
        crossterm::terminal::disable_raw_mode();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.show_cursor();
        // And clear as much as we can before handing the control of terminal back to user.
        terminal.clear();
        Ok(())
    }
}


impl UiAgent for Tui {
    fn start(mut self) -> Result<ApplicationExitReason> {
        // Setup event loop and input handling
        let (tx, rx) = mpsc::channel();
        let tick_rate = Duration::from_millis(50);

        let opcode_list = self.opcode_list.clone();
        // Thread that will send interrupt singals to UI thread (this one)
        thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                // Oh we got an event from user
                // UNWRAP: We need to use it because I don't know how to return Result
                // from this, and I doubt it can even be done.
                if event::poll(tick_rate - last_tick.elapsed()).unwrap() {
                    // Send interrupt
                    // UNWRAP: We are guaranteed that the following call will succeed
                    // as we know there already something waiting for us (see event::poll)
                    let event = event::read().unwrap();
                    if let Event::Key(key) = event {
                        if let Err(_) = tx.send(Interrupt::KeyPressed(key)) {
                            return;
                        }
                    } else if let Event::Mouse(mouse) = event {
                        if let Err(_) = tx.send(Interrupt::MouseEvent(mouse)) {
                            return;
                        }
                    }
                }
                if last_tick.elapsed() > tick_rate {
                    if let Err(_) = tx.send(Interrupt::IntervalElapsed) {
                        return;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        self.terminal.clear().with_context(|| {
            "Failed to clear terminal during drawing state. Do you have modern term?"
        })?;
        let mut draw_memory: DrawMemory = DrawMemory::default();

        // UI thread that manages drawing
        loop {
            let debug_steps = self.debug_steps.clone();
            // Wait for interrupt
            match rx.recv()? {
                // Handle user input. Vi-like controls are available,
                // including prefixing a command with number to execute it
                // multiple times (in case of breakpoint toggles breakpoint on given line).
                Interrupt::KeyPressed(event) => match event.code {
                    // Exit
                    KeyCode::Char('q') => {
                        disable_raw_mode()?;
                        execute!(
                            self.terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        return Ok(ApplicationExitReason::UserExit);
                    }
                    // Move cursor down
                    KeyCode::Char('j') | KeyCode::Down => {
                        for _ in
                            0..Tui::get_pressed_key_buffer_as_number(&self.pressed_keys_buffer, 1)
                        {
                            if self.current_step < opcode_list.len() - 1 {
                                self.current_step += 1;
                            }
                        }
                        self.pressed_keys_buffer.clear();
                    }
                    // Move cursor up
                    KeyCode::Char('k') | KeyCode::Up => {
                        for _ in
                            0..Tui::get_pressed_key_buffer_as_number(&self.pressed_keys_buffer, 1)
                        {
                            if self.current_step > 0 {
                                self.current_step -= 1;
                            }
                        }
                        self.pressed_keys_buffer.clear();
                    }
                    // Go to top of file
                    KeyCode::Char('g') => {
                        self.current_step = 0;
                        self.pressed_keys_buffer.clear();
                    }
                    // Go to bottom of file
                    KeyCode::Char('G') => {
                        self.current_step = opcode_list.len() - 1;
                        self.pressed_keys_buffer.clear();
                    }
                    // Step forward
                    KeyCode::Char('s') => {
                        for _ in
                            0..Tui::get_pressed_key_buffer_as_number(&self.pressed_keys_buffer, 1)
                        {
                            let remaining_ops = opcode_list[self.current_step..].to_vec().clone();
                            self.current_step += remaining_ops.iter().enumerate().find_map(|(i, op)| {
                                if i < remaining_ops.len() - 1 {
                                    match (op.contains("JUMP") && op != "JUMPDEST", &*remaining_ops[i+1]) {
                                        (true, "JUMPDEST") => Some(i+1),
                                        _ => None
                                    }
                                } else {
                                    None
                                }
                            }).unwrap_or(opcode_list.len() - 1);
                            if self.current_step > opcode_list.len() { self.current_step = opcode_list.len() - 1};
                        }
                        self.pressed_keys_buffer.clear();
                    }
                    // Step backwards
                    KeyCode::Char('a') => {
                        for _ in
                            0..Tui::get_pressed_key_buffer_as_number(&self.pressed_keys_buffer, 1)
                        {
                            let prev_ops = opcode_list[..self.current_step].to_vec().clone();
                            self.current_step = prev_ops.iter().enumerate().rev().find_map(|(i, op)| {
                                if i > 0 {
                                    match (prev_ops[i-1].contains("JUMP") && prev_ops[i-1] != "JUMPDEST", &**op) {
                                        (true, "JUMPDEST") => Some(i - 1),
                                        _ => None
                                    }
                                } else {
                                    None
                                }
                            }).unwrap_or_default();
                        }
                        self.pressed_keys_buffer.clear();
                    }
                    KeyCode::Char(other) => match other {
                        '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => {
                            self.pressed_keys_buffer.push(other);
                        }
                        _ => {
                            // Invalid key, clear buffer
                            self.pressed_keys_buffer.clear();
                        }
                    },
                    _ => {
                        self.pressed_keys_buffer.clear();
                    }
                },
                Interrupt::MouseEvent(event) => match event.kind {
                    // Button pressed, mark current line as breakpoint
                    MouseEventKind::ScrollUp => {
                        if self.current_step > 0 {
                            self.current_step -= 1;
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if self.current_step < opcode_list.len() - 1 {
                            self.current_step += 1;
                        }
                    }
                    _ => {}
                },
                Interrupt::IntervalElapsed => {}
            }
            // Draw
            let current_step = self.current_step;
            self.terminal.draw(|mut f| {
                Tui::draw_layout_and_subcomponents(
                    &mut f,
                    debug_steps,
                    opcode_list.clone(),
                    current_step,
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
    current_startline: usize,
}
impl DrawMemory {
    fn default() -> Self {
        DrawMemory {
            current_startline: 0,
        }
    }
}