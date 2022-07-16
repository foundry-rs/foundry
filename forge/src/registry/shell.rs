//! shell abstraction used to interact with stdout

use std::io::Write;

use termcolor::{
    self, Color,
    Color::{Cyan, Green, Red, Yellow},
    ColorChoice, ColorSpec, StandardStream, WriteColor,
};

/// Provides a configurable abstraction for how messages should be emitted, like verbosity and color
#[derive(Default)]
pub struct Shell {
    output: Output,
    verbosity: Verbosity,
    needs_clear: bool,
}

// === impl Shell ===

impl Shell {
    /// Creates a shell that will always write into the given `out`
    pub fn plain(out: Box<dyn Write>) -> Shell {
        Shell { output: Output::Plain(out), verbosity: Verbosity::Verbose, needs_clear: false }
    }

    /// Returns access to  stdout
    fn stdout(&mut self) -> &mut dyn Write {
        match self {
            Output::TermColored(term) => &mut term.stdout,
            Output::Plain(ref mut w) => w,
        }
    }

    /// Returns access to `io::Write`.
    fn stderr(&mut self) -> &mut dyn Write {
        match self {
            Output::TermColored(term) => &mut term.stderr,
            Output::Plain(ref mut w) => w,
        }
    }
}

/// The `Write`able output abstract
enum Output {
    /// Writes messages as they come into the `Write`
    ///
    /// Main purpose for this variant is testing
    Plain(Box<dyn Write>),
    /// Uses `termcolor` for writing colored text to stdout/stdwerr
    TermColored(TermConfig),
}

/// An advanced, configured `Output` wrapper
struct TermConfig {
    stdout: StandardStream,
    stderr: StandardStream,
    stderr_tty: bool,
}

impl Default for TermConfig {
    fn default() -> Self {
        fn choice(stream: atty::Stream) -> ColorChoice {
            if atty::is(stream) {
                ColorChoice::Auto
            } else {
                ColorChoice::Never
            }
        }
        Self {
            stdout: StandardStream::stdout(choice(atty::Stream::Stdout)),
            stderr: StandardStream::stderr(choice(atty::Stream::Stderr)),
            stderr_tty: atty::is(atty::Stream::Stderr),
        }
    }
}

/// Verbosity level of the output
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verbosity {
    Verbose,
    Normal,
    Silent,
}

impl Default for Verbosity {
    fn default() -> Self {
        Verbosity::Verbose
    }
}
