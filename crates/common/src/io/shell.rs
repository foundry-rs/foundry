//! Utility functions for writing to [`stdout`](std::io::stdout) and [`stderr`](std::io::stderr).
//!
//! Originally from [cargo](https://github.com/rust-lang/cargo/blob/35814255a1dbaeca9219fae81d37a8190050092c/src/cargo/core/shell.rs).

use super::style::*;
use anstream::AutoStream;
use anstyle::Style;
use clap::ValueEnum;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{prelude::*, IsTerminal},
    ops::DerefMut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock, PoisonError,
    },
};

/// Returns the current color choice.
pub fn color_choice() -> ColorChoice {
    Shell::get().color_choice()
}

/// Returns the currently set verbosity level.
pub fn verbosity() -> Verbosity {
    Shell::get().verbosity()
}

/// Set the verbosity level.
pub fn set_verbosity(verbosity: Verbosity) {
    Shell::get().set_verbosity(verbosity);
}

/// Returns whether the output mode is [`OutputMode::Quiet`].
pub fn is_quiet() -> bool {
    Shell::get().output_mode().is_quiet()
}

/// Returns whether the output format is [`OutputFormat::Json`].
pub fn is_json() -> bool {
    Shell::get().is_json()
}

/// The global shell instance.
static GLOBAL_SHELL: OnceLock<Mutex<Shell>> = OnceLock::new();

/// Terminal width.
pub enum TtyWidth {
    /// Not a terminal, or could not determine size.
    NoTty,
    /// A known width.
    Known(usize),
    /// A guess at the width.
    Guess(usize),
}

impl TtyWidth {
    /// Returns the width of the terminal from the environment, if known.
    pub fn get() -> Self {
        // use stderr
        #[cfg(unix)]
        let opt = terminal_size::terminal_size_of(std::io::stderr());
        #[cfg(not(unix))]
        let opt = terminal_size::terminal_size();
        match opt {
            Some((w, _)) => Self::Known(w.0 as usize),
            None => Self::NoTty,
        }
    }

    /// Returns the width used by progress bars for the tty.
    pub fn progress_max_width(&self) -> Option<usize> {
        match *self {
            Self::NoTty => None,
            Self::Known(width) | Self::Guess(width) => Some(width),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
/// The requested output mode.
pub enum OutputMode {
    /// Default output
    #[default]
    Normal,
    /// No output
    Quiet,
}

impl OutputMode {
    /// Returns true if the output mode is `Normal`.
    #[inline]
    pub fn is_normal(self) -> bool {
        self == Self::Normal
    }

    /// Returns true if the output mode is `Quiet`.
    #[inline]
    pub fn is_quiet(self) -> bool {
        self == Self::Quiet
    }
}

/// The requested output format.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    /// Plain text output.
    #[default]
    Text,
    /// JSON output.
    Json,
}

impl OutputFormat {
    /// Returns true if the output format is `Text`.
    #[inline]
    pub fn is_text(self) -> bool {
        self == Self::Text
    }

    /// Returns true if the output format is `Json`.
    #[inline]
    pub fn is_json(self) -> bool {
        self == Self::Json
    }
}

/// The verbosity level.
pub type Verbosity = u8;

/// An abstraction around console output that remembers preferences for output
/// verbosity and color.
pub struct Shell {
    /// Wrapper around stdout/stderr. This helps with supporting sending
    /// output to a memory buffer which is useful for tests.
    output: ShellOut,

    /// The format to use for message output.
    output_format: OutputFormat,

    /// The verbosity mode to use for message output.
    output_mode: OutputMode,

    /// The verbosity level to use for message output.
    verbosity: Verbosity,

    /// Flag that indicates the current line needs to be cleared before
    /// printing. Used when a progress bar is currently displayed.
    needs_clear: AtomicBool,
}

impl fmt::Debug for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Shell");
        s.field("output_format", &self.output_format);
        s.field("output_mode", &self.output_mode);
        s.field("verbosity", &self.verbosity);
        if let ShellOut::Stream { color_choice, .. } = self.output {
            s.field("color_choice", &color_choice);
        }
        s.finish()
    }
}

/// A `Write`able object, either with or without color support.
enum ShellOut {
    /// Color-enabled stdio, with information on whether color should be used.
    Stream {
        stdout: AutoStream<std::io::Stdout>,
        stderr: AutoStream<std::io::Stderr>,
        stderr_tty: bool,
        color_choice: ColorChoice,
    },
    /// A write object that ignores all output.
    Empty(std::io::Empty),
}

/// Whether messages should use color output.
#[derive(Debug, Default, PartialEq, Clone, Copy, Serialize, Deserialize, ValueEnum)]
pub enum ColorChoice {
    /// Intelligently guess whether to use color output (default).
    #[default]
    Auto,
    /// Force color output.
    Always,
    /// Force disable color output.
    Never,
}

impl Default for Shell {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    /// Creates a new shell (color choice and verbosity), defaulting to 'auto' color and verbose
    /// output.
    #[inline]
    pub fn new() -> Self {
        Self::new_with(
            OutputFormat::Text,
            OutputMode::Normal,
            ColorChoice::Auto,
            Verbosity::default(),
        )
    }

    /// Creates a new shell with the given color choice and verbosity.
    #[inline]
    pub fn new_with(
        format: OutputFormat,
        mode: OutputMode,
        color: ColorChoice,
        verbosity: Verbosity,
    ) -> Self {
        Self {
            output: ShellOut::Stream {
                stdout: AutoStream::new(std::io::stdout(), color.to_anstream_color_choice()),
                stderr: AutoStream::new(std::io::stderr(), color.to_anstream_color_choice()),
                color_choice: color,
                stderr_tty: std::io::stderr().is_terminal(),
            },
            output_format: format,
            output_mode: mode,
            verbosity,
            needs_clear: AtomicBool::new(false),
        }
    }

    /// Creates a shell that ignores all output.
    #[inline]
    pub fn empty() -> Self {
        Self {
            output: ShellOut::Empty(std::io::empty()),
            output_format: OutputFormat::Text,
            output_mode: OutputMode::Quiet,
            verbosity: 0,
            needs_clear: AtomicBool::new(false),
        }
    }

    /// Acquire a lock to the global shell.
    ///
    /// Initializes it with the default values if it has not been set yet.
    pub fn get() -> impl DerefMut<Target = Self> + 'static {
        GLOBAL_SHELL.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Set the global shell.
    ///
    /// # Panics
    ///
    /// Panics if the global shell has already been set.
    #[track_caller]
    pub fn set(self) {
        GLOBAL_SHELL
            .set(Mutex::new(self))
            .unwrap_or_else(|_| panic!("attempted to set global shell twice"))
    }

    /// Sets whether the next print should clear the current line and returns the previous value.
    #[inline]
    pub fn set_needs_clear(&self, needs_clear: bool) -> bool {
        self.needs_clear.swap(needs_clear, Ordering::Relaxed)
    }

    /// Returns `true` if the output format is JSON.
    pub fn is_json(&self) -> bool {
        self.output_format.is_json()
    }

    /// Returns `true` if the verbosity level is `Quiet`.
    pub fn is_quiet(&self) -> bool {
        self.output_mode.is_quiet()
    }

    /// Returns `true` if the `needs_clear` flag is set.
    #[inline]
    pub fn needs_clear(&self) -> bool {
        self.needs_clear.load(Ordering::Relaxed)
    }

    /// Returns `true` if the `needs_clear` flag is unset.
    #[inline]
    pub fn is_cleared(&self) -> bool {
        !self.needs_clear()
    }

    /// Returns the width of the terminal in spaces, if any.
    #[inline]
    pub fn err_width(&self) -> TtyWidth {
        match self.output {
            ShellOut::Stream { stderr_tty: true, .. } => TtyWidth::get(),
            _ => TtyWidth::NoTty,
        }
    }

    /// Gets the output format of the shell.
    #[inline]
    pub fn output_format(&self) -> OutputFormat {
        self.output_format
    }

    /// Gets the output mode of the shell.
    #[inline]
    pub fn output_mode(&self) -> OutputMode {
        self.output_mode
    }

    /// Gets the verbosity of the shell when [`OutputMode::Normal`] is set.
    #[inline]
    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
    }

    /// Sets the verbosity level.
    pub fn set_verbosity(&mut self, verbosity: Verbosity) {
        self.verbosity = verbosity;
    }

    /// Gets the current color choice.
    ///
    /// If we are not using a color stream, this will always return `Never`, even if the color
    /// choice has been set to something else.
    #[inline]
    pub fn color_choice(&self) -> ColorChoice {
        match self.output {
            ShellOut::Stream { color_choice, .. } => color_choice,
            ShellOut::Empty(_) => ColorChoice::Never,
        }
    }

    /// Returns `true` if stderr is a tty.
    #[inline]
    pub fn is_err_tty(&self) -> bool {
        match self.output {
            ShellOut::Stream { stderr_tty, .. } => stderr_tty,
            ShellOut::Empty(_) => false,
        }
    }

    /// Whether `stderr` supports color.
    #[inline]
    pub fn err_supports_color(&self) -> bool {
        match &self.output {
            ShellOut::Stream { stderr, .. } => supports_color(stderr.current_choice()),
            ShellOut::Empty(_) => false,
        }
    }

    /// Whether `stdout` supports color.
    #[inline]
    pub fn out_supports_color(&self) -> bool {
        match &self.output {
            ShellOut::Stream { stdout, .. } => supports_color(stdout.current_choice()),
            ShellOut::Empty(_) => false,
        }
    }

    /// Gets a reference to the underlying stdout writer.
    pub fn out(&mut self) -> &mut dyn Write {
        self.maybe_err_erase_line();
        self.output.stdout()
    }

    /// Gets a reference to the underlying stderr writer.
    pub fn err(&mut self) -> &mut dyn Write {
        self.maybe_err_erase_line();
        self.output.stderr()
    }

    /// Erase from cursor to end of line if needed.
    pub fn maybe_err_erase_line(&mut self) {
        if self.err_supports_color() && self.set_needs_clear(false) {
            // This is the "EL - Erase in Line" sequence. It clears from the cursor
            // to the end of line.
            // https://en.wikipedia.org/wiki/ANSI_escape_code#CSI_sequences
            let _ = self.output.stderr().write_all(b"\x1B[K");
        }
    }

    /// Prints a red 'error' message. Use the [`sh_err!`] macro instead.
    /// This will render a message in [ERROR] style with a bold `Error: ` prefix.
    ///
    /// **Note**: will log regardless of the verbosity level.
    pub fn error(&mut self, message: impl fmt::Display) -> Result<()> {
        self.maybe_err_erase_line();
        self.output.message_stderr(&"Error", &ERROR, Some(&message), false)
    }

    /// Prints an amber 'warning' message. Use the [`sh_warn!`] macro instead.
    /// This will render a message in [WARN] style with a bold `Warning: `prefix.
    ///
    /// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
    pub fn warn(&mut self, message: impl fmt::Display) -> Result<()> {
        match self.output_mode {
            OutputMode::Quiet => Ok(()),
            _ => self.print(&"Warning", &WARN, Some(&message), false),
        }
    }

    /// Write a styled fragment.
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    pub fn write_stdout(&mut self, fragment: impl fmt::Display, color: &Style) -> Result<()> {
        self.output.write_stdout(fragment, color)
    }

    /// Write a styled fragment with the default color. Use the [`sh_print!`] macro instead.
    ///
    /// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
    pub fn print_out(&mut self, fragment: impl fmt::Display) -> Result<()> {
        match self.output_mode {
            OutputMode::Quiet => Ok(()),
            _ => self.write_stdout(fragment, &Style::new()),
        }
    }

    /// Write a styled fragment
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    pub fn write_stderr(&mut self, fragment: impl fmt::Display, color: &Style) -> Result<()> {
        self.output.write_stderr(fragment, color)
    }

    /// Write a styled fragment with the default color. Use the [`sh_eprint!`] macro instead.
    ///
    /// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
    pub fn print_err(&mut self, fragment: impl fmt::Display) -> Result<()> {
        match self.output_mode {
            OutputMode::Quiet => Ok(()),
            _ => self.write_stderr(fragment, &Style::new()),
        }
    }

    /// Prints a message, where the status will have `color` color, and can be justified. The
    /// messages follows without color.
    fn print(
        &mut self,
        status: &dyn fmt::Display,
        style: &Style,
        message: Option<&dyn fmt::Display>,
        justified: bool,
    ) -> Result<()> {
        match self.output_mode {
            OutputMode::Quiet => Ok(()),
            _ => {
                self.maybe_err_erase_line();
                self.output.message_stderr(status, style, message, justified)
            }
        }
    }
}

impl ShellOut {
    /// Prints out a message with a status to stderr. The status comes first, and is bold plus the
    /// given color. The status can be justified, in which case the max width that will right
    /// align is 12 chars.
    fn message_stderr(
        &mut self,
        status: &dyn fmt::Display,
        style: &Style,
        message: Option<&dyn fmt::Display>,
        justified: bool,
    ) -> Result<()> {
        let buffer = Self::format_message(status, message, style, justified)?;
        self.stderr().write_all(&buffer)?;
        Ok(())
    }

    /// Write a styled fragment
    fn write_stdout(&mut self, fragment: impl fmt::Display, style: &Style) -> Result<()> {
        let mut buffer = Vec::new();
        write!(buffer, "{style}{fragment}{style:#}")?;
        self.stdout().write_all(&buffer)?;
        Ok(())
    }

    /// Write a styled fragment
    fn write_stderr(&mut self, fragment: impl fmt::Display, style: &Style) -> Result<()> {
        let mut buffer = Vec::new();
        write!(buffer, "{style}{fragment}{style:#}")?;
        self.stderr().write_all(&buffer)?;
        Ok(())
    }

    /// Gets stdout as a [`io::Write`](Write) trait object.
    #[inline]
    fn stdout(&mut self) -> &mut dyn Write {
        match self {
            Self::Stream { stdout, .. } => stdout,
            Self::Empty(e) => e,
        }
    }

    /// Gets stderr as a [`io::Write`](Write) trait object.
    #[inline]
    fn stderr(&mut self) -> &mut dyn Write {
        match self {
            Self::Stream { stderr, .. } => stderr,
            Self::Empty(e) => e,
        }
    }

    /// Formats a message with a status and optional message.
    fn format_message(
        status: &dyn fmt::Display,
        message: Option<&dyn fmt::Display>,
        style: &Style,
        justified: bool,
    ) -> Result<Vec<u8>> {
        let bold = anstyle::Style::new().bold();

        let mut buffer = Vec::new();
        if justified {
            write!(buffer, "{style}{status:>12}{style:#}")?;
        } else {
            write!(buffer, "{style}{status}{style:#}{bold}:{bold:#}")?;
        }
        match message {
            Some(message) => {
                writeln!(buffer, " {message}")?;
            }
            None => write!(buffer, " ")?,
        }

        Ok(buffer)
    }
}

impl ColorChoice {
    /// Converts our color choice to [`anstream`]'s version.
    #[inline]
    fn to_anstream_color_choice(self) -> anstream::ColorChoice {
        match self {
            Self::Always => anstream::ColorChoice::Always,
            Self::Never => anstream::ColorChoice::Never,
            Self::Auto => anstream::ColorChoice::Auto,
        }
    }
}

#[inline]
fn supports_color(choice: anstream::ColorChoice) -> bool {
    match choice {
        anstream::ColorChoice::Always
        | anstream::ColorChoice::AlwaysAnsi
        | anstream::ColorChoice::Auto => true,
        anstream::ColorChoice::Never => false,
    }
}
