use console::{Key, Term};
use std::io::{self, Read, Write};

use super::cursor::StringCursor;

pub enum State<T> {
    Active,
    Submit(T),
    Cancel,
    Error(String),
}

#[derive(PartialEq, Eq)]
pub enum Event {
    Key(Key),
}

/// Wraps text to fit the terminal width.
fn wrap(text: &str, width: usize) -> String {
    use textwrap::{core::Word, fill, Options, WordSeparator};

    fill(
        text,
        Options::new(width).word_separator(
            // Workaround to prevent textwrap from splitting words by spaces
            // which breaks the layout of the prompt. Instead, we treat
            // each line as a single word which forces wrapping it hardly
            // at the end of the terminal width.
            WordSeparator::Custom(|line| Box::new(vec![Word::from(line)].into_iter())),
        ),
    )
}

/// A component that renders itself as a prompt and handles user input.
///
/// Two methods are mandatory to implement:
/// [`render()`](PromptInteraction::render) and [`on()`](PromptInteraction::on).
///
/// Interaction with the user starts with [`interact()`](PromptInteraction::interact).
pub trait PromptInteraction<T> {
    /// Renders the prompt according to the interaction state.
    fn render(&mut self, state: &State<T>) -> String;

    /// Handles user input.
    fn on(&mut self, event: &Event) -> State<T>;

    /// Returns the cursor object which is going to be manipulated and modified
    /// during the user interaction.
    fn input(&mut self) -> Option<&mut StringCursor> {
        None
    }

    /// Whether features like Alt-Backspace and Alt-ArrowLeft/Right are allowed.
    /// Word editing is disabled for password prompts, for example.
    fn allow_word_editing(&self) -> bool {
        true
    }

    /// Starts the interaction with the user via stderr.
    fn interact(&mut self) -> io::Result<T> {
        self.interact_on(&mut Term::stderr())
    }

    /// Starts the interaction with the user via the given terminal.
    fn interact_on(&mut self, term: &mut Term) -> io::Result<T> {
        if !term.is_term() {
            return Err(io::ErrorKind::NotConnected.into());
        }

        term.hide_cursor()?;
        let result = self.interact_on_prepared(term);
        term.show_cursor()?;
        result
    }

    /// Starts the interaction with the user via the prepared terminal.
    /// This is a common boilerplate code.
    fn interact_on_prepared(&mut self, term: &mut Term) -> io::Result<T> {
        let mut state = State::Active;
        let mut prev_frame = String::new();

        loop {
            let frame = self.render(&state);

            if frame != prev_frame {
                let prev_frame_check = wrap(&prev_frame, term.size().1 as usize);

                term.clear_last_lines(prev_frame_check.lines().count())?;
                term.write_all(frame.as_bytes())?;
                term.flush()?;

                prev_frame = frame;
            }

            match state {
                State::Submit(result) => return Ok(result),
                State::Cancel => return Err(io::ErrorKind::Interrupted.into()),
                _ => {}
            }

            match term.read_key() {
                Ok(Key::Escape) => {
                    state = State::Cancel;

                    // WORKAROUND: for the `Esc` key, `Cancel` means "cancellation of cancellation".
                    if let State::Cancel = self.on(&Event::Key(Key::Escape)) {
                        state = State::Active;
                    }
                }

                Ok(key) => {
                    let word_editing = self.allow_word_editing();
                    if let Some(cursor) = self.input() {
                        match key {
                            Key::Char(chr) if !chr.is_ascii_control() => cursor.insert(chr),
                            Key::Backspace => cursor.delete_left(),
                            Key::Del => cursor.delete_right(),
                            Key::ArrowLeft => cursor.move_left(),
                            Key::ArrowRight => cursor.move_right(),
                            Key::ArrowUp => cursor.move_up(),
                            Key::ArrowDown => cursor.move_down(),
                            Key::Home => cursor.move_home(),
                            Key::End => cursor.move_end(),

                            // Alt-Backspace
                            Key::Char('\u{17}') if word_editing => cursor.delete_word_to_the_left(),

                            // Alt/Ctrl
                            Key::UnknownEscSeq(ref chars) if word_editing => {
                                match chars.as_slice() {
                                    // Alt | Ctrl-Backspace
                                    ['\u{7f}'] => cursor.delete_word_to_the_left(),
                                    // Alt-ArrowLeft | Alt-b
                                    ['b'] => cursor.move_left_by_word(),
                                    // Alt-ArrowRight | Alt-f
                                    ['f'] => cursor.move_right_by_word(),
                                    // Alt | Ctrl
                                    ['[', '1', ';'] => {
                                        let mut two_chars = [0; 2];
                                        term.read_exact(&mut two_chars)?;
                                        match two_chars {
                                            // Alt | Ctrl-ArrowLeft
                                            [b'3', b'D'] | [b'5', b'D'] => {
                                                cursor.move_left_by_word()
                                            }
                                            // Alt | Ctrl-ArrowRight
                                            [b'3', b'C'] | [b'5', b'C'] => {
                                                cursor.move_right_by_word()
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }

                    state = self.on(&Event::Key(key));
                }

                // Handle Ctrl-C as a cancel event.
                Err(e) if e.kind() == io::ErrorKind::Interrupted => state = State::Cancel,

                // Don't handle other errors, just break the loop and propagate
                // them.
                Err(e) => return Err(e),
            }
        }
    }
}
