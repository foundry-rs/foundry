//! Utilities for working with standard input, output, and error.

#[macro_use]
mod macros;

pub mod shell;
pub mod stdin;
pub mod style;

#[doc(no_inline)]
pub use shell::Shell;
