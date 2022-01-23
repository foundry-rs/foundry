//! Subcommands
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].

pub mod bind;
pub mod build;
pub mod config;
pub mod create;
pub mod flatten;
pub mod init;
pub mod install;
pub mod node;
pub mod remappings;
pub mod run;
pub mod snapshot;
pub mod test;
pub mod verify;

// Re-export our shared utilities
mod utils;
pub use utils::*;
