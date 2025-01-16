//! # foundry-cli
//!
//! Common CLI utilities.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

pub mod handler;
pub mod opts;
pub mod utils;

// The version of the Foundry CLI.
pub const VERSION_MESSAGE: &str = env!("FOUNDRY_SHORT_VERSION");

// The warning message for nightly versions.
pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a nightly build of Foundry. It is recommended to use the latest stable version. \
    Visit https://book.getfoundry.sh/announcements for more information. \n\
    To mute this warning set `FOUNDRY_DISABLE_NIGHTLY_WARNING` in your environment. \n";
