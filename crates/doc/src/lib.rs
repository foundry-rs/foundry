#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

mod builder;
mod hir_ext;
mod render;
mod utils;
mod vocs;

pub use builder::{BuildStats, DocBuilder};
