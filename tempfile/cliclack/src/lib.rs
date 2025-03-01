//! Beautiful, minimal, opinionated CLI prompts inspired by the
//! [@clack/prompts](https://www.npmjs.com/package/@clack/prompts) `npm` package.
//!
//! "Effortlessly build beautiful command-line apps" (C)
//! [original @clack](https://www.npmjs.com/package/@clack/prompts).
//!
//! 💎 Fancy minimal UI.<br>
//! ✅ Simple API.<br>
//! 🧱 Comes with [`input`](fn@input), [`password`](fn@password),
//!    [`confirm`](fn@confirm), [`select`](fn@select),
//!    [`multiselect`](fn@multiselect), [`spinner`](fn@spinner),
//!    [`progress_bar`](fn@progress_bar), and
//!    [`multi_progress`](fn@multi_progress) prompts.<br>
//! 🧱 Styled non-interactive messages with [`log`] submodule.<br>
//! 🎨 [`Theme`] support.<br>
//!
//! <img src="https://github.com/fadeevab/cliclack/raw/main/media/cliclack-demo.gif" width="50%">
//!
//! # Usage
//!
//! API is similar to the original Clack API besides of a few exceptions.
//!
//! ## Setup
//!
//! The [`intro`] and [`outro`]/[`outro_cancel`] functions will
//! print a message to begin and end a prompt session respectively.
//!
//! ```
//! use cliclack::{intro, outro};
//!
//! intro("create-my-app")?;
//! // Do stuff
//! outro("You're all set!")?;
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! ## Cancellation
//!
//! `Esc` cancels the prompt sequence with a nice message.
//! `Ctrl+C` will be handled gracefully (same as `Esc`) if you set up a Ctrl+C
//! handler, eg. with the `ctrlc` crate.
//!
//! # Components
//!
//! All prompts can be constructed either directly, e.g. with [`Input::new`],
//! or with the convenience function, e.g. [`input()`].
//!
//! ## Input
//!
//! The input prompt accepts a single line (or multiple lines) of text
//! trying to parse it into a target type.
//!
//! Multiline editing can be enabled by [`Input::multiline`].
//!
//! ```
//! use cliclack::input;
//!
//! # fn test() -> std::io::Result<()> {
//! let number: String = input("What is the meaning of life?")
//!     .placeholder("Not sure")
//!     .validate(|input: &String| {
//!         if input.is_empty() {
//!             Err("Value is required!")
//!         } else {
//!             Ok(())
//!         }
//!     })
//!     .interact()?;
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Password
//!
//! The password prompt is similar to the input prompt, but it doesn't echo the
//! actual characters.
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::password;
//!
//! let password = password("Provide a password")
//!     .mask('▪')
//!     .interact()?;
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Confirm
//!
//! The confirm prompt asks for a yes/no answer. It returns a boolean (`true`/`false`).
//!
//! '`Y`' and '`N`' keys are accepted as an immediate answer.
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::confirm;
//!
//! let should_continue = confirm("Do you want to continue?").interact()?;
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Select
//!
//! The select prompt asks to choose one of the options from the list.
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::select;
//!
//! let selected = select("Pick a project type")
//!     .item("ts", "TypeScript", "")
//!     .item("js", "JavaScript", "")
//!     .item("coffee", "CoffeeScript", "oh no")
//!     .interact()?;
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Multi-Select
//!
//! The multi-select prompt asks to choose one or more options from the list.
//! The result is a vector of selected items.
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::multiselect;
//!
//! let additional_tools = multiselect("Select additional tools.")
//!     .item("eslint", "ESLint", "recommended")
//!     .item("prettier", "Prettier", "")
//!     .item("gh-action", "GitHub Actions", "")
//!     .interact()?;
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Fuzzy Search
//!
//! Both [`Select`] and [`MultiSelect`] prompts support items filtering by
//! typing enabled by [`Select::filter_mode`] and [`MultiSelect::filter_mode`]
//! respectively.
//!
//! ## Spinner
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::spinner;
//!
//! let spinner = spinner();
//! spinner.start("Installing...");
//! // Do installation.
//! spinner.stop("Installation complete");
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Progress Bar
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::progress_bar;
//!
//! let progress = progress_bar(100);
//! progress.start("Installation...");
//! for _ in 0..100 {
//!      progress.inc(1);
//! }
//! progress.stop("Installation complete");
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Multi-Progress Bar
//!
//! <img src="https://github.com/fadeevab/cliclack/raw/main/media/cliclack-multi-progress-bar.gif" width="70%">
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::{multi_progress, progress_bar, spinner};
//!
//! let multi = multi_progress("Doing stuff...");
//! let pb1 = multi.add(progress_bar(100));
//! let pb2 = multi.add(progress_bar(100).with_download_template());
//! let spinner = multi.add(spinner());
//!
//! pb1.start("Installation...");
//! pb2.start("Downloading...");
//! spinner.start("Waiting...");
//!
//! pb1.stop("Installation complete");
//! pb2.stop("Download complete");
//! spinner.stop("Done");
//!
//! multi.stop();
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Logging
//!
//! Plain text output without any interaction.
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::log;
//!
//! log::info("Hello, world!")?;
//! log::warning("Something is wrong")?;
//! log::error("Something is terribly wrong")?;
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! ## Theme
//!
//! Custom UI is supported via the [`Theme`] trait.
//!
//! ```
//! # fn test() -> std::io::Result<()> {
//! use cliclack::{set_theme, Theme, ThemeState};
//!
//! struct MagentaTheme;
//!
//! impl Theme for MagentaTheme {
//!     fn state_symbol_color(&self, _state: &ThemeState) -> console::Style {
//!        console::Style::new().magenta()
//!    }
//! }
//!
//! set_theme(MagentaTheme);
//! # Ok(())
//! # }
//! # test().ok(); // Ignoring I/O runtime errors.
//! ```
//!
//! See `examples/theme.rs` for a complete example.
//!
//! ```bash
//! cargo run --example theme
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unused_qualifications)]

mod confirm;
mod filter;
mod input;
mod multiprogress;
mod multiselect;
mod password;
mod progress;
mod prompt;
mod select;
mod theme;
mod validate;

use console::Term;
use std::fmt::Display;
use std::io;

use theme::THEME;

// 🎨 Export of the theme API.
pub use theme::{reset_theme, set_theme, Theme, ThemeState};
// 🎨 Re-export for some `Theme` trait methods.
pub use prompt::cursor::StringCursor;

pub use confirm::Confirm;
pub use input::Input;
pub use multiprogress::MultiProgress;
pub use multiselect::MultiSelect;
pub use password::Password;
pub use progress::ProgressBar;
pub use select::Select;
pub use validate::Validate;

fn term_write(line: impl Display) -> io::Result<()> {
    Term::stderr().write_str(line.to_string().as_str())
}

/// Clears the terminal.
pub fn clear_screen() -> io::Result<()> {
    Term::stdout().clear_screen()?;
    Term::stderr().clear_screen()
}

/// Prints a header of the prompt sequence.
pub fn intro(title: impl Display) -> io::Result<()> {
    term_write(THEME.lock().unwrap().format_intro(&title.to_string()))
}

/// Prints a footer of the prompt sequence.
pub fn outro(message: impl Display) -> io::Result<()> {
    term_write(THEME.lock().unwrap().format_outro(&message.to_string()))
}

/// Prints a footer of the prompt sequence with a failure style.
pub fn outro_cancel(message: impl Display) -> io::Result<()> {
    term_write(
        THEME
            .lock()
            .unwrap()
            .format_outro_cancel(&message.to_string()),
    )
}

/// Prints a footer of the prompt sequence with a note style.
pub fn outro_note(prompt: impl Display, message: impl Display) -> io::Result<()> {
    term_write(
        THEME
            .lock()
            .unwrap()
            .format_outro_note(&prompt.to_string(), &message.to_string()),
    )
}

/// Constructs a new [`Input`] prompt.
///
/// See [`Input`] for chainable methods.
pub fn input(prompt: impl Display) -> Input {
    Input::new(prompt)
}

/// Constructs a new [`Password`] prompt.
///
/// See [`Password`] for chainable methods.
pub fn password(prompt: impl Display) -> Password {
    Password::new(prompt)
}

/// Constructs a new [`Select`] prompt.
///
/// See [`Select`] for chainable methods.
pub fn select<T: Clone + Eq>(prompt: impl Display) -> Select<T> {
    Select::new(prompt)
}

/// Constructs a new [`MultiSelect`] prompt.
///
/// See [`MultiSelect`] for chainable methods.
pub fn multiselect<T: Clone + Eq>(prompt: impl Display) -> MultiSelect<T> {
    MultiSelect::new(prompt)
}

/// Constructs a new [`Confirm`] prompt.
///
/// See [`Confirm`] for chainable methods.
pub fn confirm(prompt: impl Display) -> Confirm {
    Confirm::new(prompt)
}

/// Constructs a new [`ProgressBar::with_spinner_template`] prompt.
///
/// See [`ProgressBar`] for chainable methods.
pub fn spinner() -> ProgressBar {
    ProgressBar::new(0).with_spinner_template()
}

/// Constructs a new [`ProgressBar`] prompt.
///
/// See [`progress::ProgressBar`] for chainable methods.
pub fn progress_bar(len: u64) -> ProgressBar {
    ProgressBar::new(len)
}

/// Constructs a new [`MultiProgress`] prompt.
///
/// See [`MultiProgress`] for chainable methods.
pub fn multi_progress(prompt: impl Display) -> MultiProgress {
    MultiProgress::new(prompt)
}

/// Prints a note message.
pub fn note(prompt: impl Display, message: impl Display) -> io::Result<()> {
    term_write(
        THEME
            .lock()
            .unwrap()
            .format_note(&prompt.to_string(), &message.to_string()),
    )
}

/// Non-interactive information messages of different styles.
pub mod log {
    use super::*;

    fn log(text: impl Display, symbol: impl Display) -> io::Result<()> {
        term_write(
            THEME
                .lock()
                .unwrap()
                .format_log(&text.to_string(), &symbol.to_string()),
        )
    }

    /// Prints a remark message.
    pub fn remark(text: impl Display) -> io::Result<()> {
        let symbol = THEME.lock().unwrap().remark_symbol();
        log(text, symbol)
    }

    /// Prints an info message.
    pub fn info(text: impl Display) -> io::Result<()> {
        let symbol = THEME.lock().unwrap().info_symbol();
        log(text, symbol)
    }

    /// Prints a warning message.
    pub fn warning(message: impl Display) -> io::Result<()> {
        let symbol = THEME.lock().unwrap().warning_symbol();
        log(message, symbol)
    }

    /// Prints an error message.
    pub fn error(message: impl Display) -> io::Result<()> {
        let symbol = THEME.lock().unwrap().error_symbol();
        log(message, symbol)
    }

    /// Prints a success message.
    pub fn success(message: impl Display) -> io::Result<()> {
        let symbol = THEME.lock().unwrap().active_symbol();
        log(message, symbol)
    }

    /// Prints a submitted step message.
    pub fn step(message: impl Display) -> io::Result<()> {
        let symbol = THEME.lock().unwrap().submit_symbol();
        log(message, symbol)
    }
}
