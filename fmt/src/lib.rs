#![doc = include_str!("../README.md")]

mod comments;
mod formatter;
mod macros;
pub mod solang_ext;
mod visit;

pub use foundry_config::fmt::*;

pub use comments::Comments;
pub use formatter::{Formatter, FormatterError};
pub use visit::{Visitable, Visitor};
