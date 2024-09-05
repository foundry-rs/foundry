//! Utility functions for writing to [`stdout`](std::io::stdout) and [`stderr`](std::io::stderr).
//!
//! Originally from [cargo](https://github.com/rust-lang/cargo/blob/35814255a1dbaeca9219fae81d37a8190050092c/src/cargo/core/shell.rs).

use super::style::*;
use anstream::AutoStream;
use anstyle::Style;
use clap::ValueEnum;
use eyre::Result;
use std::{
    fmt,
    io::{prelude::*, IsTerminal},
    ops::DerefMut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock, PoisonError,
    },
};

/// Returns the currently set verbosity.
pub fn verbosity() -> Verbosity {
    Shell::get().verbosity()
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
        #[allow(clippy::useless_conversion)]
        let opt = terminal_size::terminal_size_using_fd(2.into());
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
            TtyWidth::NoTty => None,
            TtyWidth::Known(width) | TtyWidth::Guess(width) => Some(width),
        }
    }
}

/// The requested verbosity of output.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum Verbosity {
    /// All output
    Verbose,
    /// Default output
    #[default]
    Normal,
    /// No output
    Quiet,
}

impl Verbosity {
    /// Returns true if the verbosity level is `Verbose`.
    #[inline]
    pub fn is_verbose(self) -> bool {
        self == Verbosity::Verbose
    }

    /// Returns true if the verbosity level is `Normal`.
    #[inline]
    pub fn is_normal(self) -> bool {
        self == Verbosity::Normal
    }

    /// Returns true if the verbosity level is `Quiet`.
    #[inline]
    pub fn is_quiet(self) -> bool {
        self == Verbosity::Quiet
    }
}

/// An abstraction around console output that remembers preferences for output
/// verbosity and color.
pub struct Shell {
    /// Wrapper around stdout/stderr. This helps with supporting sending
    /// output to a memory buffer which is useful for tests.
    output: ShellOut,

    /// How verbose messages should be.
    verbosity: Verbosity,

    /// Flag that indicates the current line needs to be cleared before
    /// printing. Used when a progress bar is currently displayed.
    needs_clear: AtomicBool,
}

impl fmt::Debug for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Shell");
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
#[derive(Debug, Default, PartialEq, Clone, Copy, ValueEnum)]
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
        Self::new_with(ColorChoice::Auto, Verbosity::Verbose)
    }

    /// Creates a new shell with the given color choice and verbosity.
    #[inline]
    pub fn new_with(color: ColorChoice, verbosity: Verbosity) -> Self {
        Self {
            output: ShellOut::Stream {
                stdout: AutoStream::new(std::io::stdout(), color.to_anstream_color_choice()),
                stderr: AutoStream::new(std::io::stderr(), color.to_anstream_color_choice()),
                color_choice: color,
                stderr_tty: std::io::stderr().is_terminal(),
            },
            verbosity,
            needs_clear: AtomicBool::new(false),
        }
    }

    /// Creates a shell that ignores all output.
    #[inline]
    pub fn empty() -> Self {
        Self {
            output: ShellOut::Empty(std::io::empty()),
            verbosity: Verbosity::Quiet,
            needs_clear: AtomicBool::new(false),
        }
    }

    /// Get a static reference to the global shell.
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn get() -> impl DerefMut<Target = Self> + 'static {
        #[inline(never)]
        #[cold]
        #[cfg_attr(debug_assertions, track_caller)]
        fn shell_get_fail() -> Mutex<Shell> {
            if cfg!(test) {
                Mutex::new(Shell::new())
            } else {
                panic!("attempted to get global shell before it was set");
            }
        }

        GLOBAL_SHELL.get_or_init(shell_get_fail).lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Set the global shell.
    ///
    /// # Panics
    ///
    /// Panics if the global shell has already been set.
    #[inline]
    #[track_caller]
    pub fn set(self) {
        if GLOBAL_SHELL.get().is_some() {
            panic!("attempted to set global shell twice");
        }
        GLOBAL_SHELL.get_or_init(|| Mutex::new(self));
    }

    /// Sets whether the next print should clear the current line and returns the previous value.
    #[inline]
    pub fn set_needs_clear(&self, needs_clear: bool) -> bool {
        self.needs_clear.swap(needs_clear, Ordering::Relaxed)
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

    /// Gets the verbosity of the shell.
    #[inline]
    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
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
    #[inline]
    pub fn out(&mut self) -> &mut dyn Write {
        self.maybe_err_erase_line();
        self.output.stdout()
    }

    /// Gets a reference to the underlying stderr writer.
    #[inline]
    pub fn err(&mut self) -> &mut dyn Write {
        self.maybe_err_erase_line();
        self.output.stderr()
    }

    /// Erase from cursor to end of line if needed.
    #[inline]
    pub fn maybe_err_erase_line(&mut self) {
        if self.err_supports_color() && self.set_needs_clear(false) {
            // This is the "EL - Erase in Line" sequence. It clears from the cursor
            // to the end of line.
            // https://en.wikipedia.org/wiki/ANSI_escape_code#CSI_sequences
            let _ = self.output.stderr().write_all(b"\x1B[K");
        }
    }

    /// Shortcut to right-align and color green a status message.
    #[inline]
    pub fn status<T, U>(&mut self, status: T, message: U) -> Result<()>
    where
        T: fmt::Display,
        U: fmt::Display,
    {
        self.print(&status, Some(&message), &HEADER, true)
    }

    /// Shortcut to right-align and color cyan a status without a message.
    #[inline]
    pub fn status_header(&mut self, status: impl fmt::Display) -> Result<()> {
        self.print(&status, None, &NOTE, true)
    }

    /// Shortcut to right-align a status message.
    #[inline]
    pub fn status_with_color<T, U>(&mut self, status: T, message: U, color: &Style) -> Result<()>
    where
        T: fmt::Display,
        U: fmt::Display,
    {
        self.print(&status, Some(&message), color, true)
    }

    /// Runs the callback only if we are in verbose mode.
    #[inline]
    pub fn verbose(&mut self, mut callback: impl FnMut(&mut Shell) -> Result<()>) -> Result<()> {
        match self.verbosity {
            Verbosity::Verbose => callback(self),
            _ => Ok(()),
        }
    }

    /// Runs the callback if we are not in verbose mode.
    #[inline]
    pub fn concise(&mut self, mut callback: impl FnMut(&mut Shell) -> Result<()>) -> Result<()> {
        match self.verbosity {
            Verbosity::Verbose => Ok(()),
            _ => callback(self),
        }
    }

    /// Prints a red 'error' message. Use the [`sh_err!`] macro instead.
    #[inline]
    pub fn error(&mut self, message: impl fmt::Display) -> Result<()> {
        self.maybe_err_erase_line();
        self.output.message_stderr(&"error", Some(&message), &ERROR, false)
    }

    /// Prints an amber 'warning' message. Use the [`sh_warn!`] macro instead.
    #[inline]
    pub fn warn(&mut self, message: impl fmt::Display) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => self.print(&"warning", Some(&message), &WARN, false),
        }
    }

    /// Prints a cyan 'note' message. Use the [`sh_note!`] macro instead.
    #[inline]
    pub fn note(&mut self, message: impl fmt::Display) -> Result<()> {
        self.print(&"note", Some(&message), &NOTE, false)
    }

    /// Write a styled fragment.
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    #[inline]
    pub fn write_stdout(&mut self, fragment: impl fmt::Display, color: &Style) -> Result<()> {
        self.output.write_stdout(fragment, color)
    }

    /// Write a styled fragment with the default color. Use the [`sh_print!`] macro instead.
    ///
    /// **Note**: `verbosity` is ignored.
    #[inline]
    pub fn print_out(&mut self, fragment: impl fmt::Display) -> Result<()> {
        self.write_stdout(fragment, &Style::new())
    }

    /// Write a styled fragment
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    #[inline]
    pub fn write_stderr(&mut self, fragment: impl fmt::Display, color: &Style) -> Result<()> {
        self.output.write_stderr(fragment, color)
    }

    /// Write a styled fragment with the default color. Use the [`sh_eprint!`] macro instead.
    ///
    /// **Note**: if `verbosity` is set to `Quiet`, this is a no-op.
    #[inline]
    pub fn print_err(&mut self, fragment: impl fmt::Display) -> Result<()> {
        if self.verbosity == Verbosity::Quiet {
            Ok(())
        } else {
            self.write_stderr(fragment, &Style::new())
        }
    }

    /// Prints a message to stderr and translates ANSI escape code into console colors.
    #[inline]
    pub fn print_ansi_stderr(&mut self, message: &[u8]) -> Result<()> {
        self.maybe_err_erase_line();
        self.err().write_all(message)?;
        Ok(())
    }

    /// Prints a message to stdout and translates ANSI escape code into console colors.
    #[inline]
    pub fn print_ansi_stdout(&mut self, message: &[u8]) -> Result<()> {
        self.maybe_err_erase_line();
        self.out().write_all(message)?;
        Ok(())
    }

    /// Serializes an object to JSON and prints it to `stdout`.
    #[inline]
    pub fn print_json(&mut self, obj: &impl serde::Serialize) -> Result<()> {
        // Path may fail to serialize to JSON ...
        let encoded = serde_json::to_string(&obj)?;
        // ... but don't fail due to a closed pipe.
        let _ = writeln!(self.out(), "{encoded}");
        Ok(())
    }

    /// Prints a message, where the status will have `color` color, and can be justified. The
    /// messages follows without color.
    fn print(
        &mut self,
        status: &dyn fmt::Display,
        message: Option<&dyn fmt::Display>,
        color: &Style,
        justified: bool,
    ) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => {
                self.maybe_err_erase_line();
                self.output.message_stderr(status, message, color, justified)
            }
        }
    }
}

impl ShellOut {
    /// Prints out a message with a status. The status comes first, and is bold plus the given
    /// color. The status can be justified, in which case the max width that will right align is
    /// 12 chars.
    fn message_stderr(
        &mut self,
        status: &dyn fmt::Display,
        message: Option<&dyn fmt::Display>,
        style: &Style,
        justified: bool,
    ) -> Result<()> {
        let style = style.render();
        let bold = (anstyle::Style::new() | anstyle::Effects::BOLD).render();
        let reset = anstyle::Reset.render();

        let mut buffer = Vec::new();
        if justified {
            write!(&mut buffer, "{style}{status:>12}{reset}")?;
        } else {
            write!(&mut buffer, "{style}{status}{reset}{bold}:{reset}")?;
        }
        match message {
            Some(message) => writeln!(buffer, " {message}")?,
            None => write!(buffer, " ")?,
        }
        self.stderr().write_all(&buffer)?;
        Ok(())
    }

    /// Write a styled fragment
    fn write_stdout(&mut self, fragment: impl fmt::Display, style: &Style) -> Result<()> {
        let style = style.render();
        let reset = anstyle::Reset.render();

        let mut buffer = Vec::new();
        write!(buffer, "{style}{fragment}{reset}")?;
        self.stdout().write_all(&buffer)?;
        Ok(())
    }

    /// Write a styled fragment
    fn write_stderr(&mut self, fragment: impl fmt::Display, style: &Style) -> Result<()> {
        let style = style.render();
        let reset = anstyle::Reset.render();

        let mut buffer = Vec::new();
        write!(buffer, "{style}{fragment}{reset}")?;
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
}

impl ColorChoice {
    /// Converts our color choice to [`anstream`]'s version.
    fn to_anstream_color_choice(self) -> anstream::ColorChoice {
        match self {
            Self::Always => anstream::ColorChoice::Always,
            Self::Never => anstream::ColorChoice::Never,
            Self::Auto => anstream::ColorChoice::Auto,
        }
    }
}

fn supports_color(choice: anstream::ColorChoice) -> bool {
    match choice {
        anstream::ColorChoice::Always |
        anstream::ColorChoice::AlwaysAnsi |
        anstream::ColorChoice::Auto => true,
        anstream::ColorChoice::Never => false,
    }
}
