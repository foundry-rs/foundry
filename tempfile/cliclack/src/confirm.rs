use std::fmt::Display;
use std::io;

use console::Key;

use crate::{
    prompt::interaction::{Event, PromptInteraction, State},
    theme::THEME,
};

/// A prompt that asks for a yes or no confirmation.
///
/// * Move arrows to change the selection.
/// * `Enter` to confirm.
/// * `Y/y` for immediate "yes" answer.
/// * `N/n` for immediate "no" answer.
#[derive(Default)]
pub struct Confirm {
    prompt: String,
    input: bool,
    initial_value: bool,
}

impl Confirm {
    /// Creates a new confirmation prompt.
    pub fn new(prompt: impl Display) -> Self {
        Self {
            prompt: prompt.to_string(),
            ..Default::default()
        }
    }

    /// Sets the initially selected value (yes or no).
    pub fn initial_value(mut self, initial_value: bool) -> Self {
        self.initial_value = initial_value;
        self
    }

    /// Starts the prompt interaction.
    pub fn interact(&mut self) -> io::Result<bool> {
        self.input = self.initial_value;
        <Self as PromptInteraction<bool>>::interact(self)
    }
}

impl PromptInteraction<bool> for Confirm {
    fn on(&mut self, event: &Event) -> State<bool> {
        let Event::Key(key) = event;

        match key {
            Key::ArrowDown
            | Key::ArrowRight
            | Key::ArrowUp
            | Key::ArrowLeft
            | Key::Char('h')
            | Key::Char('k')
            | Key::Char('j')
            | Key::Char('l') => {
                self.input = !self.input;
            }
            Key::Char('y') | Key::Char('Y') => {
                self.input = true;
                return State::Submit(self.input);
            }
            Key::Char('n') | Key::Char('N') => {
                self.input = false;
                return State::Submit(self.input);
            }
            Key::Enter => return State::Submit(self.input),
            _ => {}
        }

        State::Active
    }

    fn render(&mut self, state: &State<bool>) -> String {
        let theme = THEME.lock().unwrap();
        let line1 = theme.format_header(&state.into(), &self.prompt);
        let line2 = theme.format_confirm(&state.into(), self.input);
        let line3 = theme.format_footer(&state.into());

        line1 + &line2 + &line3
    }
}
