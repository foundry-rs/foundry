#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

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

// On wasm, avoid importing foundry-config (pulls in non-wasm deps). Provide a local copy of
// FormatterConfig and related enums under `fmt_cfg`.
#[cfg(target_arch = "wasm32")]
pub use crate::fmt_cfg::*;
#[cfg(not(target_arch = "wasm32"))]
pub use foundry_config::fmt::*;

#[cfg(target_arch = "wasm32")]
mod fmt_cfg;

pub use comments::Comments;
pub use formatter::{Formatter, FormatterError};
pub use helpers::{
    Parsed, format, format_diagnostics_report, format_to, offset_to_line_column, parse, parse2,
};
pub use inline_config::InlineConfig;
pub use visit::{Visitable, Visitor};
