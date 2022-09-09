#![doc = include_str!("../README.md")]

mod buffer;
pub mod chunk;
mod comments;
mod formatter;
mod helpers;
pub mod inline_config;
mod macros;
pub mod solang_ext;
mod string;
pub mod visit;

pub use foundry_config::fmt::*;

pub use comments::Comments;
pub use formatter::{Formatter, FormatterError};
pub use helpers::{fmt, format, offset_to_line_column, parse, Parsed};
pub use inline_config::InlineConfig;
pub use visit::{Visitable, Visitor};
