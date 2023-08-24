//! Utility functions for writing to [`stdout`](std::io::stdout) and [`stderr`](std::io::stderr).
//!
//! Originally from [cargo](https://github.com/rust-lang/cargo/blob/35814255a1dbaeca9219fae81d37a8190050092c/src/cargo/core/shell.rs).

use clap::ValueEnum;
use eyre::Result;
use once_cell::sync::Lazy;
use std::{
    fmt,
    io::{prelude::*, IsTerminal},
    ops::DerefMut,
    sync::Mutex,
};
use termcolor::{
    Color::{self, Cyan, Green, Red, Yellow},
    ColorSpec, StandardStream, WriteColor,
};

static GLOBAL_SHELL: Lazy<Mutex<Shell>> = Lazy::new(|| Mutex::new(Shell::new()));

pub enum TtyWidth {
    NoTty,
    Known(usize),
    Guess(usize),
}

impl TtyWidth {
    pub fn get() -> Self {
        // use stderr
        #[cfg(unix)]
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
    needs_clear: bool,
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
    /// A plain write object without color support.
    Write(Box<dyn Write + Send + Sync + 'static>),
    /// Color-enabled stdio, with information on whether color should be used.
    Stream {
        stdout: StandardStream,
        stderr: StandardStream,
        stderr_tty: bool,
        color_choice: ColorChoice,
    },
}

/// Whether messages should use color output.
#[derive(Debug, Default, PartialEq, Clone, Copy, ValueEnum)]
pub enum ColorChoice {
    /// Force color output.
    Always,
    /// Force disable color output.
    Never,
    /// Intelligently guess whether to use color output.
    #[default]
    Auto,
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
    pub fn new() -> Self {
        Self::new_with(ColorChoice::Auto, Verbosity::Verbose)
    }

    pub fn new_with(color: ColorChoice, verbosity: Verbosity) -> Self {
        Self {
            output: ShellOut::Stream {
                stdout: StandardStream::stdout(color.to_termcolor_color_choice(Stream::Stdout)),
                stderr: StandardStream::stderr(color.to_termcolor_color_choice(Stream::Stderr)),
                color_choice: color,
                stderr_tty: std::io::stderr().is_terminal(),
            },
            verbosity,
            needs_clear: false,
        }
    }

    /// Creates a shell from a plain writable object, with no color, and max verbosity.
    pub fn from_write(out: Box<dyn Write + Send + Sync + 'static>) -> Self {
        Self { output: ShellOut::Write(out), verbosity: Verbosity::Verbose, needs_clear: false }
    }

    /// Get a static reference to the global shell.
    #[inline]
    #[track_caller]
    pub fn get() -> impl DerefMut<Target = Self> + 'static {
        GLOBAL_SHELL.lock().unwrap()
    }

    /// Set the global shell.
    #[inline]
    #[track_caller]
    pub fn set(self) {
        *GLOBAL_SHELL.lock().unwrap() = self;
    }

    /// Prints a message, where the status will have `color` color, and can be justified. The
    /// messages follows without color.
    fn print(
        &mut self,
        status: &dyn fmt::Display,
        message: Option<&dyn fmt::Display>,
        color: Color,
        justified: bool,
    ) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => {
                if self.needs_clear {
                    self.err_erase_line();
                }
                self.output.message_stderr(status, message, color, justified)
            }
        }
    }

    /// Sets whether the next print should clear the current line.
    pub fn set_needs_clear(&mut self, needs_clear: bool) {
        self.needs_clear = needs_clear;
    }

    /// Returns `true` if the `needs_clear` flag is unset.
    pub fn is_cleared(&self) -> bool {
        !self.needs_clear
    }

    /// Returns the width of the terminal in spaces, if any.
    pub fn err_width(&self) -> TtyWidth {
        match self.output {
            ShellOut::Stream { stderr_tty: true, .. } => TtyWidth::get(),
            _ => TtyWidth::NoTty,
        }
    }

    /// Returns `true` if stderr is a tty.
    pub fn is_err_tty(&self) -> bool {
        match self.output {
            ShellOut::Stream { stderr_tty, .. } => stderr_tty,
            _ => false,
        }
    }

    /// Gets a reference to the underlying stdout writer.
    #[inline]
    pub fn out(&mut self) -> &mut dyn Write {
        if self.needs_clear {
            self.err_erase_line();
        }
        self.output.stdout()
    }

    /// Gets a reference to the underlying stderr writer.
    #[inline]
    pub fn err(&mut self) -> &mut dyn Write {
        if self.needs_clear {
            self.err_erase_line();
        }
        self.output.stderr()
    }

    /// Erase from cursor to end of line.
    pub fn err_erase_line(&mut self) {
        if self.err_supports_color() {
            // This is the "EL - Erase in Line" sequence. It clears from the cursor
            // to the end of line.
            // https://en.wikipedia.org/wiki/ANSI_escape_code#CSI_sequences
            let _ = self.output.stderr().write_all(b"\x1B[K");
            self.set_needs_clear(false);
        }
    }

    /// Shortcut to right-align and color green a status message.
    pub fn status<T, U>(&mut self, status: T, message: U) -> Result<()>
    where
        T: fmt::Display,
        U: fmt::Display,
    {
        self.print(&status, Some(&message), Green, true)
    }

    pub fn status_header<T>(&mut self, status: T) -> Result<()>
    where
        T: fmt::Display,
    {
        self.print(&status, None, Cyan, true)
    }

    /// Shortcut to right-align a status message.
    pub fn status_with_color<T, U>(&mut self, status: T, message: U, color: Color) -> Result<()>
    where
        T: fmt::Display,
        U: fmt::Display,
    {
        self.print(&status, Some(&message), color, true)
    }

    /// Runs the callback only if we are in verbose mode.
    pub fn verbose<F>(&mut self, mut callback: F) -> Result<()>
    where
        F: FnMut(&mut Shell) -> Result<()>,
    {
        match self.verbosity {
            Verbosity::Verbose => callback(self),
            _ => Ok(()),
        }
    }

    /// Runs the callback if we are not in verbose mode.
    pub fn concise<F>(&mut self, mut callback: F) -> Result<()>
    where
        F: FnMut(&mut Shell) -> Result<()>,
    {
        match self.verbosity {
            Verbosity::Verbose => Ok(()),
            _ => callback(self),
        }
    }

    /// Prints a red 'error' message.
    pub fn error<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        if self.needs_clear {
            self.err_erase_line();
        }
        self.output.message_stderr(&"error", Some(&message), Red, false)
    }

    /// Prints an amber 'warning' message.
    pub fn warn<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => self.print(&"warning", Some(&message), Yellow, false),
        }
    }

    /// Prints a cyan 'note' message.
    pub fn note<T: fmt::Display>(&mut self, message: T) -> Result<()> {
        self.print(&"note", Some(&message), Cyan, false)
    }

    /// Updates the verbosity of the shell.
    pub fn set_verbosity(&mut self, verbosity: Verbosity) {
        self.verbosity = verbosity;
    }

    /// Gets the verbosity of the shell.
    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
    }

    /// Updates the color choice (always, never, or auto) from a string..
    pub fn set_color_choice(&mut self, color: Option<&str>) -> Result<()> {
        if let ShellOut::Stream { stdout, stderr, color_choice, .. } = &mut self.output {
            let cfg = match color {
                Some("always") => ColorChoice::Always,
                Some("never") => ColorChoice::Never,

                Some("auto") | None => ColorChoice::Auto,

                Some(arg) => eyre::bail!(
                    "argument for --color must be auto, always, or \
                     never, but found `{arg}`",
                ),
            };
            *color_choice = cfg;
            *stdout = StandardStream::stdout(cfg.to_termcolor_color_choice(Stream::Stdout));
            *stderr = StandardStream::stderr(cfg.to_termcolor_color_choice(Stream::Stderr));
        }
        Ok(())
    }

    /// Gets the current color choice.
    ///
    /// If we are not using a color stream, this will always return `Never`, even if the color
    /// choice has been set to something else.
    pub fn color_choice(&self) -> ColorChoice {
        match self.output {
            ShellOut::Stream { color_choice, .. } => color_choice,
            ShellOut::Write(_) => ColorChoice::Never,
        }
    }

    /// Whether the shell supports color.
    pub fn err_supports_color(&self) -> bool {
        match &self.output {
            ShellOut::Write(_) => false,
            ShellOut::Stream { stderr, .. } => stderr.supports_color(),
        }
    }

    pub fn out_supports_color(&self) -> bool {
        match &self.output {
            ShellOut::Write(_) => false,
            ShellOut::Stream { stdout, .. } => stdout.supports_color(),
        }
    }

    /// Write a styled fragment
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    pub fn write_stdout(&mut self, fragment: impl fmt::Display, color: &ColorSpec) -> Result<()> {
        self.output.write_stdout(fragment, color)
    }

    pub fn print_out(&mut self, fragment: impl fmt::Display) -> Result<()> {
        self.write_stdout(fragment, &ColorSpec::new())
    }

    /// Write a styled fragment
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    pub fn write_stderr(&mut self, fragment: impl fmt::Display, color: &ColorSpec) -> Result<()> {
        self.output.write_stderr(fragment, color)
    }

    pub fn print_err(&mut self, fragment: impl fmt::Display) -> Result<()> {
        self.write_stderr(fragment, &ColorSpec::new())
    }

    /// Prints a message to stderr and translates ANSI escape code into console colors.
    pub fn print_ansi_stderr(&mut self, message: &[u8]) -> Result<()> {
        if self.needs_clear {
            self.err_erase_line();
        }
        #[cfg(windows)]
        if let ShellOut::Stream { stderr, .. } = &self.output {
            ::fwdansi::write_ansi(stderr, message)?;
            return Ok(())
        }
        self.err().write_all(message)?;
        Ok(())
    }

    /// Prints a message to stdout and translates ANSI escape code into console colors.
    pub fn print_ansi_stdout(&mut self, message: &[u8]) -> Result<()> {
        if self.needs_clear {
            self.err_erase_line();
        }
        #[cfg(windows)]
        if let ShellOut::Stream { stdout, .. } = &self.output {
            ::fwdansi::write_ansi(stdout, message)?;
            return Ok(())
        }
        self.out().write_all(message)?;
        Ok(())
    }

    pub fn print_json<T: serde::ser::Serialize>(&mut self, obj: &T) -> Result<()> {
        // Path may fail to serialize to JSON ...
        let encoded = serde_json::to_string(&obj)?;
        // ... but don't fail due to a closed pipe.
        let _ = writeln!(self.out(), "{encoded}");
        Ok(())
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
        color: Color,
        justified: bool,
    ) -> Result<()> {
        match self {
            Self::Stream { stderr, .. } => {
                stderr.reset()?;
                stderr.set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))?;
                if justified {
                    write!(stderr, "{status:>12}")
                } else {
                    write!(stderr, "{status}")?;
                    stderr.set_color(ColorSpec::new().set_bold(true))?;
                    write!(stderr, ":")
                }?;
                stderr.reset()?;

                stderr.write_all(b" ")?;
                if let Some(message) = message {
                    writeln!(stderr, "{message}")?;
                }
            }
            Self::Write(w) => {
                if justified { write!(w, "{status:>12}") } else { write!(w, "{status}:") }?;
                w.write_all(b" ")?;
                if let Some(message) = message {
                    writeln!(w, "{message}")?;
                }
            }
        }
        Ok(())
    }

    /// Write a styled fragment
    fn write_stdout(&mut self, fragment: impl fmt::Display, color: &ColorSpec) -> Result<()> {
        match self {
            Self::Stream { stdout, .. } => {
                stdout.reset()?;
                stdout.set_color(&color)?;
                write!(stdout, "{fragment}")?;
                stdout.reset()?;
            }
            Self::Write(w) => {
                write!(w, "{fragment}")?;
            }
        }
        Ok(())
    }

    /// Write a styled fragment
    fn write_stderr(&mut self, fragment: impl fmt::Display, color: &ColorSpec) -> Result<()> {
        match self {
            Self::Stream { stderr, .. } => {
                stderr.reset()?;
                stderr.set_color(&color)?;
                write!(stderr, "{fragment}")?;
                stderr.reset()?;
            }
            Self::Write(w) => {
                write!(w, "{fragment}")?;
            }
        }
        Ok(())
    }

    /// Gets stdout as a `io::Write`.
    #[inline]
    fn stdout(&mut self) -> &mut dyn Write {
        match self {
            Self::Stream { stdout, .. } => stdout,
            Self::Write(w) => w,
        }
    }

    /// Gets stderr as a `io::Write`.
    #[inline]
    fn stderr(&mut self) -> &mut dyn Write {
        match self {
            Self::Stream { stderr, .. } => stderr,
            Self::Write(w) => w,
        }
    }
}

impl ColorChoice {
    /// Converts our color choice to termcolor's version.
    fn to_termcolor_color_choice(self, stream: Stream) -> termcolor::ColorChoice {
        match self {
            ColorChoice::Always => termcolor::ColorChoice::Always,
            ColorChoice::Never => termcolor::ColorChoice::Never,
            ColorChoice::Auto => {
                if stream.is_terminal() {
                    termcolor::ColorChoice::Auto
                } else {
                    termcolor::ColorChoice::Never
                }
            }
        }
    }
}

enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn is_terminal(self) -> bool {
        match self {
            Self::Stdout => std::io::stdout().is_terminal(),
            Self::Stderr => std::io::stderr().is_terminal(),
        }
    }
}
