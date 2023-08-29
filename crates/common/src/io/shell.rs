//! Utility functions for writing to [`stdout`](std::io::stdout) and [`stderr`](std::io::stderr).
//!
//! Originally from [cargo](https://github.com/rust-lang/cargo/blob/35814255a1dbaeca9219fae81d37a8190050092c/src/cargo/core/shell.rs).

use clap::ValueEnum;
use eyre::Result;
use std::{
    fmt,
    io::{prelude::*, IsTerminal},
    ops::DerefMut,
    sync::atomic::{AtomicBool, Ordering},
};
use termcolor::{
    Color::{self, Cyan, Green, Red, Yellow},
    ColorSpec, StandardStream, WriteColor,
};

/// The global shell instance.
///
/// # Safety
///
/// This instance is only ever initialized in `main`, and its fields are as follows:
/// - `output`
///   - `Stream`'s fields are not modified, and the underlying streams can only be the standard ones
///     which lock on write
///   - `Write` is not thread safe, but it's only used in tests (as of writing, not even there)
/// - `verbosity` cannot modified after initialization
/// - `needs_clear` is an atomic boolean
///
/// In general this is probably fine.
static mut GLOBAL_SHELL: Option<Shell> = None;

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
        stdout: StandardStream,
        stderr: StandardStream,
        stderr_tty: bool,
        color_choice: ColorChoice,
    },
    /// A plain write object without color support.
    Write(Box<dyn Write + Send + Sync + 'static>),
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
                stdout: StandardStream::stdout(color.to_termcolor_color_choice(Stream::Stdout)),
                stderr: StandardStream::stderr(color.to_termcolor_color_choice(Stream::Stderr)),
                color_choice: color,
                stderr_tty: std::io::stderr().is_terminal(),
            },
            verbosity,
            needs_clear: AtomicBool::new(false),
        }
    }

    /// Creates a shell from a plain writable object, with no color, and max verbosity.
    ///
    /// Not thread safe, so not exposed outside of tests.
    #[inline]
    pub fn from_write(out: Box<dyn Write + Send + Sync + 'static>) -> Self {
        let needs_clear = AtomicBool::new(false);
        Self { output: ShellOut::Write(out), verbosity: Verbosity::Verbose, needs_clear }
    }

    /// Get a static reference to the global shell.
    #[inline]
    #[track_caller]
    pub fn get() -> impl DerefMut<Target = Self> + 'static {
        // SAFETY: See [GLOBAL_SHELL]
        match unsafe { &mut GLOBAL_SHELL } {
            Some(shell) => shell,
            // This shouldn't happen outside of tests
            none => {
                if cfg!(test) {
                    none.insert(Self::new())
                } else {
                    // use `expect` to get `#[cold]`
                    none.as_mut().expect("attempted to get global shell before it was set")
                }
            }
        }
    }

    /// Set the global shell.
    ///
    /// # Safety
    ///
    /// See [GLOBAL_SHELL].
    #[inline]
    #[track_caller]
    pub unsafe fn set(self) {
        let shell = unsafe { &mut GLOBAL_SHELL };
        if shell.is_none() {
            *shell = Some(self);
        } else {
            panic!("attempted to set global shell twice");
        }
    }

    /// Sets whether the next print should clear the current line and returns the previous value.
    #[inline]
    pub fn set_needs_clear(&mut self, needs_clear: bool) -> bool {
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
            ShellOut::Write(_) => ColorChoice::Never,
        }
    }

    /// Returns `true` if stderr is a tty.
    #[inline]
    pub fn is_err_tty(&self) -> bool {
        match self.output {
            ShellOut::Stream { stderr_tty, .. } => stderr_tty,
            ShellOut::Write(_) => false,
        }
    }

    /// Whether `stderr` supports color.
    #[inline]
    pub fn err_supports_color(&self) -> bool {
        match &self.output {
            ShellOut::Stream { stderr, .. } => stderr.supports_color(),
            ShellOut::Write(_) => false,
        }
    }

    /// Whether `stdout` supports color.
    #[inline]
    pub fn out_supports_color(&self) -> bool {
        match &self.output {
            ShellOut::Stream { stdout, .. } => stdout.supports_color(),
            ShellOut::Write(_) => false,
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
        self.print(&status, Some(&message), Green, true)
    }

    /// Shortcut to right-align and color cyan a status without a message.
    #[inline]
    pub fn status_header(&mut self, status: impl fmt::Display) -> Result<()> {
        self.print(&status, None, Cyan, true)
    }

    /// Shortcut to right-align a status message.
    #[inline]
    pub fn status_with_color<T, U>(&mut self, status: T, message: U, color: Color) -> Result<()>
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
        self.output.message_stderr(&"error", Some(&message), Red, false)
    }

    /// Prints an amber 'warning' message. Use the [`sh_warn!`] macro instead.
    #[inline]
    pub fn warn(&mut self, message: impl fmt::Display) -> Result<()> {
        match self.verbosity {
            Verbosity::Quiet => Ok(()),
            _ => self.print(&"warning", Some(&message), Yellow, false),
        }
    }

    /// Prints a cyan 'note' message. Use the [`sh_note!`] macro instead.
    #[inline]
    pub fn note(&mut self, message: impl fmt::Display) -> Result<()> {
        self.print(&"note", Some(&message), Cyan, false)
    }

    /// Write a styled fragment.
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    #[inline]
    pub fn write_stdout(&mut self, fragment: impl fmt::Display, color: &ColorSpec) -> Result<()> {
        self.output.write_stdout(fragment, color)
    }

    /// Write a styled fragment with the default color. Use the [`sh_print!`] macro instead.
    ///
    /// **Note**: `verbosity` is ignored.
    #[inline]
    pub fn print_out(&mut self, fragment: impl fmt::Display) -> Result<()> {
        self.write_stdout(fragment, &ColorSpec::new())
    }

    /// Write a styled fragment
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    #[inline]
    pub fn write_stderr(&mut self, fragment: impl fmt::Display, color: &ColorSpec) -> Result<()> {
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
            self.write_stderr(fragment, &ColorSpec::new())
        }
    }

    /// Prints a message to stderr and translates ANSI escape code into console colors.
    #[inline]
    pub fn print_ansi_stderr(&mut self, message: &[u8]) -> Result<()> {
        self.maybe_err_erase_line();
        #[cfg(windows)]
        if let ShellOut::Stream { stderr, .. } = &self.output {
            ::fwdansi::write_ansi(stderr, message)?;
            return Ok(())
        }
        self.err().write_all(message)?;
        Ok(())
    }

    /// Prints a message to stdout and translates ANSI escape code into console colors.
    #[inline]
    pub fn print_ansi_stdout(&mut self, message: &[u8]) -> Result<()> {
        self.maybe_err_erase_line();
        #[cfg(windows)]
        if let ShellOut::Stream { stdout, .. } = &self.output {
            ::fwdansi::write_ansi(stdout, message)?;
            return Ok(())
        }
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
        color: Color,
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
