use std::sync::Mutex;

use console::{style, Emoji, Style};
use once_cell::sync::Lazy;
use textwrap::core::display_width;

use crate::prompt::{cursor::StringCursor, interaction::State};

const S_STEP_ACTIVE: Emoji = Emoji("◆", "*");
const S_STEP_CANCEL: Emoji = Emoji("■", "x");
const S_STEP_ERROR: Emoji = Emoji("▲", "x");
const S_STEP_SUBMIT: Emoji = Emoji("◇", "o");

const S_BAR_START: Emoji = Emoji("┌", "T");
const S_BAR: Emoji = Emoji("│", "|");
const S_BAR_END: Emoji = Emoji("└", "—");

const S_RADIO_ACTIVE: Emoji = Emoji("●", ">");
const S_RADIO_INACTIVE: Emoji = Emoji("○", " ");
const S_CHECKBOX_ACTIVE: Emoji = Emoji("◻", "[•]");
const S_CHECKBOX_SELECTED: Emoji = Emoji("◼", "[+]");
const S_CHECKBOX_INACTIVE: Emoji = Emoji("◻", "[ ]");
const S_PASSWORD_MASK: Emoji = Emoji("▪", "•");

const S_BAR_H: Emoji = Emoji("─", "-");
const S_CORNER_TOP_RIGHT: Emoji = Emoji("╮", "+");
const S_CONNECT_LEFT: Emoji = Emoji("├", "+");
const S_CORNER_BOTTOM_RIGHT: Emoji = Emoji("╯", "+");

const S_INFO: Emoji = Emoji("●", "•");
const S_WARN: Emoji = Emoji("▲", "!");
const S_ERROR: Emoji = Emoji("■", "x");

const S_SPINNER: Emoji = Emoji("◒◐◓◑", "•oO0");
const S_PROGRESS: Emoji = Emoji("■□", "#-");

/// The state of the prompt rendering.
pub enum ThemeState {
    /// The prompt is active.
    Active,
    /// `Esc` key hit.
    Cancel,
    /// `Enter` key hit.
    Submit,
    /// Validation error occurred.
    Error(String),
}

impl<T> From<&State<T>> for ThemeState {
    fn from(state: &State<T>) -> Self {
        match state {
            State::Active => Self::Active,
            State::Cancel => Self::Cancel,
            State::Submit(_) => Self::Submit,
            State::Error(e) => Self::Error(e.clone()),
        }
    }
}

/// Defines rendering of the visual elements. By default, it implements the
/// original [@Clack/prompts](https://www.npmjs.com/package/@clack/prompts) theme.
///
/// ```
/// # use cliclack::*;
/// # struct ClackTheme;
/// #
/// /// The default @clack/prompts theme is literally implemented like this.
/// impl Theme for ClackTheme {}
/// ```
///
/// In order to create a custom theme, implement the [`Theme`] trait, and redefine
/// the required methods:
///
/// ```
/// # use console::Style;
/// # use cliclack::*;
/// #
/// struct MagentaTheme;
///
/// impl Theme for MagentaTheme {
///     fn state_symbol_color(&self, _state: &ThemeState) -> Style {
///         Style::new().magenta()
///     }
/// }
/// ```
///
/// Then, set the theme with [`set_theme`] function.
///
/// ```
/// # use cliclack::*;
/// # struct MagentaTheme;
/// # impl Theme for MagentaTheme {}
/// #
/// set_theme(MagentaTheme);
/// ```
///
/// Many theme methods render the visual elements differently depending on the
/// current rendering state. The state is passed to the theme methods as an argument.
pub trait Theme {
    /// Returns the color of the vertical side bar.
    fn bar_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active => Style::new().cyan(),
            ThemeState::Cancel => Style::new().red(),
            ThemeState::Submit => Style::new().bright().black(),
            ThemeState::Error(_) => Style::new().yellow(),
        }
    }

    /// Returns the color of the symbol of the current rendering state.
    fn state_symbol_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Submit => Style::new().green(),
            _ => self.bar_color(state),
        }
    }

    /// Returns the symbol of the current rendering state.
    fn state_symbol(&self, state: &ThemeState) -> String {
        let color = self.state_symbol_color(state);

        match state {
            ThemeState::Active => color.apply_to(S_STEP_ACTIVE),
            ThemeState::Cancel => color.apply_to(S_STEP_CANCEL),
            ThemeState::Submit => color.apply_to(S_STEP_SUBMIT),
            ThemeState::Error(_) => color.apply_to(S_STEP_ERROR),
        }
        .to_string()
    }

    /// Returns the symbol of the radio item of the select list.
    fn radio_symbol(&self, state: &ThemeState, selected: bool) -> String {
        match state {
            ThemeState::Active if selected => style(S_RADIO_ACTIVE).green(),
            ThemeState::Active if !selected => style(S_RADIO_INACTIVE).dim(),
            _ => style(Emoji("", "")),
        }
        .to_string()
    }

    /// Returns the symbol of the checkbox item of the multiselect list.
    fn checkbox_symbol(&self, state: &ThemeState, selected: bool, active: bool) -> String {
        match state {
            ThemeState::Active | ThemeState::Error(_) => {
                if selected {
                    style(S_CHECKBOX_SELECTED).green()
                } else if active && !selected {
                    style(S_CHECKBOX_ACTIVE).cyan()
                } else if !active && !selected {
                    style(S_CHECKBOX_INACTIVE).dim()
                } else {
                    style(Emoji("", ""))
                }
            }
            _ => style(Emoji("", "")),
        }
        .to_string()
    }

    /// Returns the symbol of the remark.
    fn remark_symbol(&self) -> String {
        self.bar_color(&ThemeState::Submit)
            .apply_to(S_CONNECT_LEFT)
            .to_string()
    }

    /// Returns the symbol of the info message.
    fn info_symbol(&self) -> String {
        style(S_INFO).blue().to_string()
    }

    /// Returns the symbol of the warning message.
    fn warning_symbol(&self) -> String {
        style(S_WARN).yellow().to_string()
    }

    /// Returns the symbol of the error message.
    fn error_symbol(&self) -> String {
        style(S_ERROR).red().to_string()
    }

    /// Returns the symbol of the active step.
    fn active_symbol(&self) -> String {
        style(S_STEP_ACTIVE).green().to_string()
    }

    /// Returns the symbol of the cancel step.
    fn submit_symbol(&self) -> String {
        style(S_STEP_SUBMIT).green().to_string()
    }

    /// Returns the console style of the checkbox item.
    fn checkbox_style(&self, state: &ThemeState, selected: bool, active: bool) -> Style {
        match state {
            ThemeState::Cancel if selected => Style::new().dim().strikethrough(),
            ThemeState::Submit if selected => Style::new().dim(),
            _ if !active => Style::new().dim(),
            _ => Style::new(),
        }
    }

    /// Returns the console style of the input text of a prompt.
    fn input_style(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Cancel => Style::new().dim().strikethrough(),
            ThemeState::Submit => Style::new().dim(),
            _ => Style::new(),
        }
    }

    /// Returns the console style of the placeholder text.
    fn placeholder_style(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Cancel => Style::new().hidden(),
            _ => Style::new().dim(),
        }
    }

    /// Highlights the cursor character in the input text formatting the whole
    /// string with the given style.
    fn cursor_with_style(&self, cursor: &StringCursor, new_style: &Style) -> String {
        let (left, cursor, right) = cursor.split();
        format!(
            "{left}{cursor}{right}",
            left = new_style.apply_to(left),
            cursor = style(cursor).reverse(),
            right = new_style.apply_to(right),
        )
    }

    /// Returns the password mask character.
    fn password_mask(&self) -> char {
        S_PASSWORD_MASK.to_string().chars().next().unwrap()
    }

    /// Formats the intro message (like `┌  title`).
    fn format_intro(&self, title: &str) -> String {
        let color = self.bar_color(&ThemeState::Submit);
        format!(
            "{start_bar}  {title}\n{bar}\n",
            start_bar = color.apply_to(S_BAR_START),
            bar = color.apply_to(S_BAR),
        )
    }

    /// Formats the outro message (like `└  {message}`).
    fn format_outro(&self, message: &str) -> String {
        let color = self.bar_color(&ThemeState::Submit);
        format!(
            "{bar_end}  {message}\n",
            bar_end = color.apply_to(S_BAR_END)
        )
    }

    /// Formats the outro message with a failure style
    /// (like `└  {message}` with a red style).
    fn format_outro_cancel(&self, message: &str) -> String {
        let color = self.bar_color(&ThemeState::Submit);
        format!(
            "{bar}  {message}\n",
            bar = color.apply_to(S_BAR_END),
            message = style(message).red()
        )
    }

    /// Formats the header of the prompt (like `◇  Input data`).
    fn format_header(&self, state: &ThemeState, prompt: &str) -> String {
        let mut lines = vec![];

        for (i, line) in prompt.lines().enumerate() {
            if i == 0 {
                lines.push(format!(
                    "{state_symbol}  {line}\n",
                    state_symbol = self.state_symbol(state)
                ));
            } else {
                lines.push(format!(
                    "{bar}  {line}\n",
                    bar = self.bar_color(state).apply_to(S_BAR)
                ));
            }
        }

        lines.join("")
    }

    /// Formats the footer of the prompt (like `└  Operation cancelled.`).
    fn format_footer(&self, state: &ThemeState) -> String {
        self.format_footer_with_message(state, "")
    }

    /// Formats the footer with a custom message (like `└  {message}`).
    fn format_footer_with_message(&self, state: &ThemeState, message: &str) -> String {
        format!(
            "{}\n", // '\n' vanishes by style applying, thus exclude it from styling
            self.bar_color(state).apply_to(match state {
                ThemeState::Active => format!("{S_BAR_END}  {message}"),
                ThemeState::Cancel => format!("{S_BAR_END}  Operation cancelled."),
                ThemeState::Submit => format!("{S_BAR}"),
                ThemeState::Error(err) => format!("{S_BAR_END}  {err}"),
            })
        )
    }

    /// Formats the input cursor with the given style adding frame bars around.
    ///
    /// It hides the cursor when the input is not active.
    fn format_input(&self, state: &ThemeState, cursor: &StringCursor) -> String {
        let new_style = &self.input_style(state);

        let input = &mut match state {
            ThemeState::Active | ThemeState::Error(_) => self.cursor_with_style(cursor, new_style),
            _ => cursor.to_string(),
        };
        if input.ends_with('\n') {
            input.push('\n');
        }

        input.lines().fold(String::new(), |acc, line| {
            format!(
                "{}{}  {}\n",
                acc,
                self.bar_color(state).apply_to(S_BAR),
                new_style.apply_to(line)
            )
        })
    }

    /// Formats the input cursor with the dimmed style of placeholder.
    ///
    /// Additionally:
    /// * Hides the placeholder fully at the cancelled state.
    /// * Hides the cursor character at the submitted state keeping the text
    ///   (it's used to draw the final result built from the string cursor object).
    fn format_placeholder(&self, state: &ThemeState, cursor: &StringCursor) -> String {
        let new_style = &self.placeholder_style(state);

        let placeholder = &match state {
            ThemeState::Active | ThemeState::Error(_) => self.cursor_with_style(cursor, new_style),
            ThemeState::Cancel => "".to_string(),
            _ => cursor.to_string(),
        };
        placeholder.lines().fold(String::new(), |acc, line| {
            format!(
                "{}{}  {}\n",
                acc,
                self.bar_color(state).apply_to(S_BAR),
                new_style.apply_to(line)
            )
        })
    }

    /// Returns the radio item without frame bars around the item.
    ///
    /// The radio item is used in the selection list and in the confirmation prompt.
    /// There are [`Theme::format_select_item`] and [`Theme::format_confirm`]
    /// for the full item formatting respectively.
    ///
    /// Hides the item if not selected on the submit and cancel states.
    fn radio_item(&self, state: &ThemeState, selected: bool, label: &str, hint: &str) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => return String::new(),
            _ => {}
        }

        let radio = self.radio_symbol(state, selected);
        let input_style = &self.input_style(state);
        let inactive_style = &self.placeholder_style(state);

        let label = if selected {
            input_style.apply_to(label)
        } else {
            inactive_style.apply_to(label)
        }
        .to_string();

        let hint = match state {
            ThemeState::Active | ThemeState::Error(_) if !hint.is_empty() && selected => {
                inactive_style.apply_to(format!("({})", hint)).to_string()
            }
            _ => String::new(),
        };

        format!(
            "{radio}{space1}{label}{space2}{hint}",
            space1 = if radio.is_empty() { "" } else { " " },
            space2 = if label.is_empty() { "" } else { " " }
        )
    }

    /// Returns the full select list item formatting with frame bars around.
    ///
    /// Hides the item if not selected on the submit and cancel states.
    fn format_select_item(
        &self,
        state: &ThemeState,
        selected: bool,
        label: &str,
        hint: &str,
    ) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => return String::new(),
            _ => {}
        }

        format!(
            "{bar}  {radio_item}\n",
            bar = self.bar_color(state).apply_to(S_BAR),
            radio_item = self.radio_item(state, selected, label, hint)
        )
    }

    /// Returns the checkbox item without frame bars around the item.
    ///
    /// Hides the item if not selected on the submit and cancel states.
    fn checkbox_item(
        &self,
        state: &ThemeState,
        selected: bool, // when item is selected/checked
        active: bool,   // when cursors highlights item
        label: &str,
        hint: &str,
    ) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => return String::new(),
            _ => {}
        }

        let checkbox = self.checkbox_symbol(state, selected, active);
        let label_style = self.checkbox_style(state, selected, active);
        let hint_style = self.placeholder_style(state);
        let label = label_style.apply_to(label).to_string();

        let hint = match state {
            ThemeState::Active | ThemeState::Error(_) if !hint.is_empty() && active => {
                hint_style.apply_to(format!("({})", hint)).to_string()
            }
            _ => String::new(),
        };

        format!(
            "{checkbox}{space1}{label}{space2}{hint}",
            space1 = if checkbox.is_empty() { "" } else { " " },
            space2 = if label.is_empty() { "" } else { " " }
        )
    }

    /// Returns the full multiselect list item formatting with frame bars around.
    ///
    /// Hides the item if not selected on the submit and cancel states.
    fn format_multiselect_item(
        &self,
        state: &ThemeState,
        selected: bool, // when item is selected/checked
        active: bool,   // when cursors highlights item
        label: &str,
        hint: &str,
    ) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => return String::new(),
            _ => {}
        }

        format!(
            "{bar}  {checkbox_item}\n",
            bar = self.bar_color(state).apply_to(S_BAR),
            checkbox_item = self.checkbox_item(state, selected, active, label, hint),
        )
    }

    /// Returns the full confirmation prompt rendering.
    fn format_confirm(&self, state: &ThemeState, confirm: bool) -> String {
        let yes = self.radio_item(state, confirm, "Yes", "");
        let no = self.radio_item(state, !confirm, "No", "");

        let inactive_style = &self.placeholder_style(state);
        let divider = match state {
            ThemeState::Active => inactive_style.apply_to(" / ").to_string(),
            _ => "".to_string(),
        };

        format!(
            "{bar}  {yes}{divider}{no}\n",
            bar = self.bar_color(state).apply_to(S_BAR),
        )
    }

    /// Returns a progress bar template.
    fn default_progress_template(&self) -> String {
        "{msg} [{elapsed_precise}] {bar:30.magenta} ({pos}/{len})".into()
    }

    /// Returns a spinner bar template.
    fn default_spinner_template(&self) -> String {
        "{msg}".into()
    }

    /// Return a default download template.
    fn default_download_template(&self) -> String {
        "{msg} [{elapsed_precise}] [{bar:30.cyan/blue}] {bytes}/{total_bytes} ({eta})".into()
    }

    /// Returns a progress bar message with multiline rendering.
    ///
    /// This function adds the left-bar to all lines but the first, encompasses
    /// the remainder of the message (including new-lines) with the side-bar,
    /// and finally ends with the section end character.
    fn format_progress_message(&self, text: &str) -> String {
        let bar = self.bar_color(&ThemeState::Submit).apply_to(S_BAR);
        let end = self.bar_color(&ThemeState::Submit).apply_to(S_BAR_END);

        let lines: Vec<_> = text.lines().collect();

        let parts: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| match i {
                0 => line.to_string(),
                _ if i < lines.len() - 1 => format!("{bar}  {line}"),
                _ => format!("{end}  {line}"),
            })
            .collect();

        parts.join("\n")
    }

    /// Returns the progress bar start style for the [`indicatif::ProgressBar`].
    fn format_progress_start(&self, template: &str, grouped: bool, last: bool) -> String {
        let space = if grouped { " " } else { "  " };
        self.format_progress_with_state(
            &format!("{{spinner:.magenta}}{space}{template}"),
            grouped,
            last,
            &ThemeState::Active,
        )
    }

    /// Returns the progress bar with formatted prefix and suffix, e.g. `│  ◒ Downloading`.
    fn format_progress_with_state(
        &self,
        msg: &str,
        grouped: bool,
        last: bool,
        state: &ThemeState,
    ) -> String {
        let prefix = if grouped {
            self.bar_color(state).apply_to(S_BAR).to_string() + "  "
        } else {
            match state {
                ThemeState::Active => "".to_string(),
                _ => self.state_symbol(state).to_string() + "  ",
            }
        };

        let suffix = if grouped && last {
            format!("\n{}", self.format_footer(state)) // | or └ with message
        } else if grouped && !last {
            "".to_string() // Nothing.
        } else {
            match state {
                ThemeState::Active => "".to_string(), // No footer.
                _ => format!("\n{}", self.bar_color(&ThemeState::Submit).apply_to(S_BAR)), // |
            }
        };

        if !msg.is_empty() {
            format!("{prefix}{msg}{suffix}")
        } else {
            suffix
        }
    }

    /// Returns the spinner character sequence.
    fn spinner_chars(&self) -> String {
        S_SPINNER.to_string()
    }

    /// Returns the progress bar character sequence.
    fn progress_chars(&self) -> String {
        S_PROGRESS.to_string()
    }

    /// Returns the multiline note message rendering, taking into account whether
    /// or not it's an inline vs. outro note.
    fn format_note_generic(&self, is_outro: bool, prompt: &str, message: &str) -> String {
        let message = format!("\n{message}\n");
        let width = 2 + message
            .split('\n')
            .fold(0usize, |acc, line| display_width(line).max(acc))
            .max(display_width(prompt));

        let bar_color = self.bar_color(&ThemeState::Submit);
        let text_color = self.input_style(&ThemeState::Submit);

        // If we're rendering an outro note, we use the connecting left bar
        // instead of the step symbol.
        let symbol = if is_outro {
            bar_color.apply_to(S_CONNECT_LEFT).to_string()
        } else {
            self.state_symbol(&ThemeState::Submit)
        };

        // Render the header.
        let header = format!(
            "{symbol}  {prompt} {horizontal_bar}{corner}\n",
            horizontal_bar =
                bar_color.apply_to(S_BAR_H.to_string().repeat(width - display_width(prompt))),
            corner = bar_color.apply_to(S_CORNER_TOP_RIGHT),
        );

        // Render the body, with multi-line support.
        #[allow(clippy::format_collect)]
        let body = message
            .lines()
            .map(|line| {
                format!(
                    "{bar}  {line}{spaces}{bar}\n",
                    bar = bar_color.apply_to(S_BAR),
                    line = text_color.apply_to(line),
                    spaces = " ".repeat(width - display_width(line) + 1)
                )
            })
            .collect::<String>();

        // Render the footer. Depending on whether or not this is an outro note,
        // we'll either use the bar end or the connecting left bar.
        let footer = if is_outro {
            bar_color
                .apply_to(format!(
                    "{S_BAR_END}{horizontal_bar}{S_CORNER_BOTTOM_RIGHT}\n",
                    horizontal_bar = S_BAR_H.to_string().repeat(width + 3),
                ))
                .to_string()
        } else {
            bar_color
                .apply_to(format!(
                    "{S_CONNECT_LEFT}{horizontal_bar}{S_CORNER_BOTTOM_RIGHT}\n{bar}\n",
                    horizontal_bar = S_BAR_H.to_string().repeat(width + 3),
                    bar = bar_color.apply_to(S_BAR),
                ))
                .to_string()
        };

        header + &body + &footer
    }

    /// Formats an inline note message.
    fn format_note(&self, prompt: &str, message: &str) -> String {
        self.format_note_generic(false, prompt, message)
    }

    /// Formats an outro note message.
    fn format_outro_note(&self, prompt: &str, message: &str) -> String {
        self.format_note_generic(true, prompt, message)
    }

    /// Returns a log message rendering with a chosen symbol.
    fn format_log(&self, text: &str, symbol: &str) -> String {
        self.format_log_with_spacing(text, symbol, true)
    }

    /// Returns a log message rendering with a chosen symbol, with an optional trailing empty line.
    fn format_log_with_spacing(&self, text: &str, symbol: &str, spacing: bool) -> String {
        let mut parts = vec![];
        let chain = match spacing {
            true => "\n",
            false => "",
        };
        let mut lines = text.lines().chain(chain.lines());

        if let Some(first) = lines.next() {
            parts.push(format!("{symbol}  {first}"));
        }
        for line in lines {
            parts.push(format!(
                "{bar}  {line}",
                bar = self.bar_color(&ThemeState::Submit).apply_to(S_BAR)
            ));
        }
        parts.push("".into());
        parts.join("\n")
    }
}

/// Default @clack/prompts theme.
struct ClackTheme;

/// Using default @clack/prompts theme implementation from the [`Theme`] trait.
impl Theme for ClackTheme {}

/// The global theme instance (singleton).
///
/// It can be set with [`set_theme`] function.
pub(crate) static THEME: Lazy<Mutex<Box<dyn Theme + Send + Sync>>> =
    Lazy::new(|| Mutex::new(Box::new(ClackTheme)));

/// Sets the global theme, which is used by all prompts.
///
/// See [`reset_theme`] for returning to the default theme.
pub fn set_theme<T: Theme + Sync + Send + 'static>(theme: T) {
    *THEME.lock().unwrap() = Box::new(theme);
}

/// Resets the global theme to the default one.
pub fn reset_theme() {
    *THEME.lock().unwrap() = Box::new(ClackTheme);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_note() {
        // A simple backward compatibility check.
        ClackTheme.format_note("my prompt", "my message");
    }
}
