//! Helpers for printing to output

use once_cell::sync::OnceCell;
use serde::Serialize;
use std::{
    error::Error,
    fmt, io,
    io::Write,
    sync::{Arc, Mutex},
};

/// Stores the configured shell for the duration of the program
static SHELL: OnceCell<Shell> = OnceCell::new();

/// Error indicating that `set_hook` was unable to install the provided ErrorHook
#[derive(Debug, Clone, Copy)]
pub struct InstallError;

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cannot install provided Shell, a shell has already been installed")
    }
}

impl Error for InstallError {}

/// Install the provided shell
pub fn set_shell(shell: Shell) -> Result<(), InstallError> {
    SHELL.set(shell).map_err(|_| InstallError)
}

/// Runs the given closure with the current shell, or default shell if none was set
pub fn with_shell<F, R>(f: F) -> R
where
    F: FnOnce(&Shell) -> R,
{
    if let Some(shell) = SHELL.get() {
        f(shell)
    } else {
        let shell = Shell::default();
        f(&shell)
    }
}

/// Prints the given message to the shell
pub fn print(msg: impl fmt::Display) -> io::Result<()> {
    with_shell(|shell| shell.write_stdout(msg))
}
/// Prints the given message to the shell
pub fn print_json<T: Serialize>(obj: &T) -> serde_json::Result<()> {
    with_shell(|shell| shell.print_json(obj))
}

/// Prints the given message to the shell
pub fn eprint(msg: impl fmt::Display) -> io::Result<()> {
    with_shell(|shell| shell.write_stderr(msg))
}

/// Returns the configured verbosity
pub fn verbosity() -> Verbosity {
    with_shell(|shell| shell.verbosity)
}

/// An abstraction around console output that also considers verbosity
#[derive(Default)]
pub struct Shell {
    /// Wrapper around stdout/stderr.
    output: ShellOut,
    /// How to emit messages.
    verbosity: Verbosity,
}

// === impl Shell ===

impl Shell {
    /// Creates a new shell instance
    pub fn new(output: ShellOut, verbosity: Verbosity) -> Self {
        Self { output, verbosity }
    }

    /// Returns a new shell that conforms to the specified verbosity arguments, where `json` takes
    /// higher precedence
    pub fn from_args(silent: bool, json: bool) -> Self {
        match (silent, json) {
            (_, true) => Self::json(),
            (true, _) => Self::silent(),
            _ => Default::default(),
        }
    }

    /// Returns a new shell that won't emit anything
    pub fn silent() -> Self {
        Self::from_verbosity(Verbosity::Silent)
    }

    /// Returns a new shell that'll only emit json
    pub fn json() -> Self {
        Self::from_verbosity(Verbosity::Json)
    }

    /// Creates a new shell instance with default output and the given verbosity
    pub fn from_verbosity(verbosity: Verbosity) -> Self {
        Self::new(Default::default(), verbosity)
    }

    /// Write a fragment to stdout
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    pub fn write_stdout(&self, fragment: impl fmt::Display) -> io::Result<()> {
        self.output.write_stdout(fragment)
    }

    /// Write a fragment to stderr
    ///
    /// Caller is responsible for deciding whether [`Shell::verbosity`] is affects output.
    pub fn write_stderr(&self, fragment: impl fmt::Display) -> io::Result<()> {
        self.output.write_stderr(fragment)
    }

    /// Prints the object to stdout as json
    pub fn print_json<T: serde::ser::Serialize>(&self, obj: &T) -> serde_json::Result<()> {
        if self.verbosity.is_json() {
            let json = serde_json::to_string(&obj)?;
            let _ = self.output.with_stdout(|out| writeln!(out, "{}", json));
        }
        Ok(())
    }
    /// Prints the object to stdout as pretty json
    pub fn pretty_print_json<T: serde::ser::Serialize>(&self, obj: &T) -> serde_json::Result<()> {
        if self.verbosity.is_json() {
            let json = serde_json::to_string_pretty(&obj)?;
            let _ = self.output.with_stdout(|out| writeln!(out, "{}", json));
        }
        Ok(())
    }
}

impl fmt::Debug for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.output {
            ShellOut::Write(_) => {
                f.debug_struct("Shell").field("verbosity", &self.verbosity).finish()
            }
            ShellOut::Stream => {
                f.debug_struct("Shell").field("verbosity", &self.verbosity).finish()
            }
        }
    }
}

/// Helper trait for custom shell output
///
/// Can be used for debugging
pub trait ShellWrite {
    /// Write the fragment
    fn write(&self, fragment: impl fmt::Display) -> io::Result<()>;

    /// Executes a closure on the current stdout
    fn with_stdout<F, R>(&self, f: F) -> R
    where
        for<'r> F: FnOnce(&'r mut (dyn Write + 'r)) -> R;

    /// Executes a closure on the current stderr
    fn with_err<F, R>(&self, f: F) -> R
    where
        for<'r> F: FnOnce(&'r mut (dyn Write + 'r)) -> R;
}

/// A guarded shell output type
pub struct WriteShellOut(Arc<Mutex<Box<dyn Write>>>);

unsafe impl Send for WriteShellOut {}
unsafe impl Sync for WriteShellOut {}

impl ShellWrite for WriteShellOut {
    fn write(&self, fragment: impl fmt::Display) -> io::Result<()> {
        if let Ok(mut lock) = self.0.lock() {
            write!(lock, "{}", fragment)?;
        }
        Ok(())
    }
    /// Executes a closure on the current stdout
    fn with_stdout<F, R>(&self, f: F) -> R
    where
        for<'r> F: FnOnce(&'r mut (dyn Write + 'r)) -> R,
    {
        let mut lock = self.0.lock().unwrap();
        f(&mut *lock)
    }

    /// Executes a closure on the current stderr
    fn with_err<F, R>(&self, f: F) -> R
    where
        for<'r> F: FnOnce(&'r mut (dyn Write + 'r)) -> R,
    {
        let mut lock = self.0.lock().unwrap();
        f(&mut *lock)
    }
}

/// A `Write`able object, either with or without color support
pub enum ShellOut {
    /// A plain write object
    ///
    /// Can be used for debug purposes
    Write(WriteShellOut),
    /// Streams to `stdio`
    Stream,
}

// === impl ShellOut ===

impl ShellOut {
    /// Creates a new shell that writes to memory
    pub fn memory() -> Self {
        ShellOut::Write(WriteShellOut(Arc::new(Mutex::new(Box::new(Vec::new())))))
    }

    /// Write a fragment to stdout
    fn write_stdout(&self, fragment: impl fmt::Display) -> io::Result<()> {
        match *self {
            ShellOut::Stream => {
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                write!(handle, "{}", fragment)?;
            }
            ShellOut::Write(ref w) => {
                w.write(fragment)?;
            }
        }
        Ok(())
    }

    /// Write output to stderr
    fn write_stderr(&self, fragment: impl fmt::Display) -> io::Result<()> {
        match *self {
            ShellOut::Stream => {
                let stderr = io::stderr();
                let mut handle = stderr.lock();
                write!(handle, "{}", fragment)?;
            }
            ShellOut::Write(ref w) => {
                w.write(fragment)?;
            }
        }
        Ok(())
    }

    /// Executes a closure on the current stdout
    fn with_stdout<F, R>(&self, f: F) -> R
    where
        for<'r> F: FnOnce(&'r mut (dyn Write + 'r)) -> R,
    {
        match *self {
            ShellOut::Stream => {
                let stdout = io::stdout();
                let mut handler = stdout.lock();
                f(&mut handler)
            }
            ShellOut::Write(ref w) => w.with_stdout(f),
        }
    }

    /// Executes a closure on the current stderr
    #[allow(unused)]
    fn with_err<F, R>(&self, f: F) -> R
    where
        for<'r> F: FnOnce(&'r mut (dyn Write + 'r)) -> R,
    {
        match *self {
            ShellOut::Stream => {
                let stderr = io::stderr();
                let mut handler = stderr.lock();
                f(&mut handler)
            }
            ShellOut::Write(ref w) => w.with_err(f),
        }
    }
}

impl Default for ShellOut {
    fn default() -> Self {
        ShellOut::Stream
    }
}

/// The requested verbosity of output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// only allow json output
    Json,
    /// print as is
    Normal,
    /// print nothing
    Silent,
}

// === impl Verbosity ===

impl Verbosity {
    /// Returns true if json mode
    pub fn is_json(&self) -> bool {
        matches!(self, Verbosity::Json)
    }

    /// Returns true if silent
    pub fn is_silent(&self) -> bool {
        matches!(self, Verbosity::Silent)
    }

    /// Returns true if normal verbosity
    pub fn is_normal(&self) -> bool {
        matches!(self, Verbosity::Normal)
    }
}

impl Default for Verbosity {
    fn default() -> Self {
        Verbosity::Normal
    }
}
