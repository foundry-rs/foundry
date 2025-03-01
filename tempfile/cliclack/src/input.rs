use std::io;
use std::{fmt::Display, str::FromStr};

use console::Key;

use crate::{
    prompt::{
        cursor::StringCursor,
        interaction::{Event, PromptInteraction, State},
    },
    theme::THEME,
    validate::Validate,
};

type ValidationCallback = Box<dyn Fn(&String) -> Result<(), String>>;

#[derive(Default, PartialEq)]
enum Multiline {
    #[default]
    Disabled,
    Preview,
    Editing,
}

/// A prompt that accepts a text input: either single-line or multiline.
///
/// # Example
///
/// ```
/// use cliclack::Input;
///
/// # fn test() -> std::io::Result<()> {
/// let input: String = Input::new("Tea or coffee?")
///     .placeholder("Yes")
///     .interact()?;
/// # Ok(())
/// # }
/// # test().ok();
/// ```
///
/// # Multiline
///
/// [`Input::multiline`] enables multiline text editing.
///
/// ```
/// use cliclack::Input;
///
/// # fn test() -> std::io::Result<()> {
/// let path: String = Input::new("Input multiple lines: ")
///     .multiline()
///     .interact()?;
/// # Ok(())
/// # }
/// # test().ok(); // Ignoring I/O runtime errors.
/// ```
#[derive(Default)]
pub struct Input {
    prompt: String,
    input: StringCursor,
    input_required: bool,
    default: Option<String>,
    placeholder: StringCursor,
    multiline: Multiline,
    validate_on_enter: Option<ValidationCallback>,
    validate_interactively: Option<ValidationCallback>,
}

impl Input {
    /// Creates a new input prompt.
    pub fn new(prompt: impl Display) -> Self {
        Self {
            prompt: prompt.to_string(),
            input_required: true,
            ..Default::default()
        }
    }

    /// Sets the placeholder (hint) text for the input.
    pub fn placeholder(mut self, placeholder: &str) -> Self {
        self.placeholder.extend(placeholder);
        self
    }

    /// Sets the default value for the input and also a hint (placeholder) if one is not already set.
    ///
    /// [`Input::placeholder`] overrides a hint set by `default()`, however, default value
    /// is used is no value has been supplied.
    pub fn default_input(mut self, value: &str) -> Self {
        self.default = Some(value.into());
        self
    }

    /// Sets whether the input is required. Default: `true`.
    ///
    /// [`Input::default_input`] is used if no value is supplied.
    pub fn required(mut self, required: bool) -> Self {
        self.input_required = required;
        self
    }

    /// Enables multiline input.
    ///
    /// 1. Press `Esc` to review and submit.
    /// 2. Start typing to get back into the editing mode.
    pub fn multiline(mut self) -> Self {
        self.multiline = Multiline::Editing;
        self
    }

    /// Sets a validation callback for the input that is called when the user submits.
    /// The same as [`Input::validate_on_enter`].
    pub fn validate<V>(mut self, validator: V) -> Self
    where
        V: Validate<String> + 'static,
        V::Err: ToString,
    {
        self.validate_on_enter = Some(Box::new(move |input: &String| {
            validator.validate(input).map_err(|err| err.to_string())
        }));
        self
    }

    /// Sets a validation callback for the input that is called when the user submits.
    pub fn validate_on_enter<V>(self, validator: V) -> Self
    where
        V: Validate<String> + 'static,
        V::Err: ToString,
    {
        self.validate(validator)
    }

    /// Validates input while user is typing.
    pub fn validate_interactively<V>(mut self, validator: V) -> Self
    where
        V: Validate<String> + 'static,
        V::Err: ToString,
    {
        self.validate_interactively = Some(Box::new(move |input: &String| {
            validator.validate(input).map_err(|err| err.to_string())
        }));
        self
    }

    /// Starts the prompt interaction.
    pub fn interact<T>(&mut self) -> io::Result<T>
    where
        T: FromStr,
    {
        if self.placeholder.is_empty() {
            if let Some(default) = &self.default {
                self.placeholder.extend(default);
                self.placeholder.extend(" (default)");

                if self.multiline == Multiline::Editing {
                    // The preview mode is convenient for immediate submission of the default value.
                    self.multiline = Multiline::Preview;
                }
            }
        }
        <Self as PromptInteraction<T>>::interact(self)
    }
}

impl<T> PromptInteraction<T> for Input
where
    T: FromStr,
{
    fn input(&mut self) -> Option<&mut StringCursor> {
        if self.multiline == Multiline::Preview {
            return None;
        }
        Some(&mut self.input)
    }

    fn on(&mut self, event: &Event) -> State<T> {
        let Event::Key(key) = event;
        let mut submit = false;

        match key {
            // Multiline: editing -> preview.
            Key::Escape if self.multiline == Multiline::Editing => {
                self.multiline = Multiline::Preview;
                return State::Cancel; // Workaround for `Esc`: "cancel cancelling".
            }
            Key::Enter => {
                if self.multiline == Multiline::Editing {
                    self.input.insert('\n')
                } else {
                    submit = true;
                }
            }
            // Multiline: don't lose 1 char switching from the preview mode to editing.
            Key::Char(c) if !c.is_ascii_control() && self.multiline == Multiline::Preview => {
                self.input.insert(*c);
            }
            Key::Backspace if self.multiline == Multiline::Preview => self.input.delete_left(),
            _ => {}
        }

        // Multiline: preview -> editing.
        if self.multiline == Multiline::Preview {
            self.multiline = Multiline::Editing;
        }

        if submit && self.input.is_empty() {
            if let Some(default) = &self.default {
                self.input.extend(default);
            } else if self.input_required {
                return State::Error("Input required".to_string());
            }
        }

        if let Some(validator) = &self.validate_interactively {
            if let Err(err) = validator(&self.input.to_string()) {
                return State::Error(err);
            }

            if self.input.to_string().parse::<T>().is_err() {
                return State::Error("Invalid value format".to_string());
            }
        }

        if submit {
            if let Some(validator) = &self.validate_on_enter {
                if let Err(err) = validator(&self.input.to_string()) {
                    return State::Error(err);
                }
            }

            match self.input.to_string().parse::<T>() {
                Ok(value) => return State::Submit(value),
                Err(_) => return State::Error("Invalid value format".to_string()),
            }
        }

        State::Active
    }

    fn render(&mut self, state: &State<T>) -> String {
        let theme = THEME.lock().unwrap();

        let part1 = theme.format_header(&state.into(), &self.prompt);
        let part2 = if self.input.is_empty() {
            theme.format_placeholder(&state.into(), &self.placeholder)
        } else {
            theme.format_input(&state.into(), &self.input)
        };
        let part3 = theme.format_footer_with_message(
            &state.into(),
            match self.multiline {
                Multiline::Editing => "[Esc](Preview)",
                Multiline::Preview => "[Enter](Submit)",
                _ => "",
            },
        );

        part1 + &part2 + &part3
    }
}
