//! Subcommands for forge
//!
//! All subcommands should respect the `foundry_config::Config`.
//! If a subcommand accepts values that are supported by the `Config`, then the subcommand should
//! implement `figment::Provider` which allows the subcommand to override the config's defaults, see
//! [`foundry_config::Config`].
//!
//! See [`BuildArgs`] for a reference implementation.
//! And [`RunArgs`] for how to merge `Providers`.
//!
//! # Example
//!
//! create a `clap` subcommand into a `figment::Provider` and integrate it in the
//! `foundry_config::Config`:
//!
//! ```rust
//! use crate::{cmd::build::BuildArgs, foundry_common::evm::EvmArgs};
//! use clap::Parser;
//! use foundry_config::{figment::Figment, *};
//!
//! // A new clap subcommand that accepts both `EvmArgs` and `BuildArgs`
//! #[derive(Debug, Clone, Parser)]
//! pub struct MyArgs {
//!     #[clap(flatten)]
//!     evm_opts: EvmArgs,
//!     #[clap(flatten)]
//!     opts: BuildArgs,
//! }
//!
//! // add `Figment` and `Config` converters
//! foundry_config::impl_figment_convert!(MyArgs, opts, evm_opts);
//! let args = MyArgs::parse_from(["build"]);
//!
//! let figment: Figment = From::from(&args);
//! let evm_opts = figment.extract::<EvmOpts>().unwrap();
//!
//! let config: Config = From::from(&args);
//! ```

pub mod bind;
pub mod build;
pub mod cache;
pub mod config;
pub mod create;
pub mod flatten;
pub mod fmt;
pub mod init;
pub mod inspect;
pub mod install;
pub mod remappings;
pub mod run;
pub mod snapshot;
pub mod test;
pub mod tree;
pub mod verify;
pub mod watch;
