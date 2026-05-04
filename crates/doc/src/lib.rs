//! Solidity documentation generator powered by [`solar`].
//! * uses directly the [`solar`] AST
//! * emits [vocs](https://vocs.dev/docs)-flavoured MDX.
//!
//! See [`DocBuilder`].

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

mod builder;
mod extras;
mod hir_ext;
mod render;
mod vocs;

pub use builder::DocBuilder;
