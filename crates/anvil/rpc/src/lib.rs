//! # anvil-rpc
//!
//! JSON-RPC types.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

/// JSON-RPC request bindings
pub mod request;

/// JSON-RPC response bindings
pub mod response;

/// JSON-RPC error bindings
pub mod error;
