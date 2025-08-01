//! Foundry Language Server Protocol implementation
//!
//! This crate provides a native LSP server for Solidity development using Foundry's
//! compilation and linting infrastructure.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

pub mod lsp;
pub mod server;
pub mod utils;

pub use lsp::ForgeLsp;
pub use server::ForgeLspServer;
