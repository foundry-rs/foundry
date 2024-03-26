#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[macro_use]
extern crate tracing;

mod buffer;
pub mod chunk;
mod comments;
mod formatter;
mod helpers;
pub mod inline_config;
mod macros;
pub mod shadow;
pub mod solang_ext;
mod string;
pub mod visit;

pub use foundry_config::fmt::*;

pub use comments::Comments;
pub use formatter::{Formatter, FormatterError};
pub use helpers::{
    format, format_to, offset_to_line_column, parse, print_diagnostics_report, Parsed,
};
pub use inline_config::InlineConfig;
pub use visit::{Visitable, Visitor};
