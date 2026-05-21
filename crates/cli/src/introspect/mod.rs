//! Machine-readable introspection for Foundry CLIs.
//!
//! This module implements the `--introspect` flag described in
//! [`docs/agents/spec.md`](../../../../docs/agents/spec.md). It walks a
//! `clap::Command` tree, merges in metadata from a per-binary
//! [`CommandRegistry`], and emits an [`IntrospectDocument`] suitable for
//! agent consumption.

mod build;
mod document;
mod registry;

pub use build::{
    build_document, collect_command_ids, duplicate_command_ids, render_introspect_document,
};
pub use document::*;
pub use registry::{CommandMeta, CommandRegistry};

/// Stable schema id for the introspect document.
pub const INTROSPECT_SCHEMA_ID: &str = "foundry:introspect@v1";

/// Schema version for the introspect document.
pub const INTROSPECT_SCHEMA_VERSION: u32 = 1;
