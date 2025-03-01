#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

pub mod backend;
pub mod cache;
pub mod error;

pub use backend::{BackendHandler, SharedBackend};
pub use cache::BlockchainDb;
pub use error::{DatabaseError, DatabaseResult};
