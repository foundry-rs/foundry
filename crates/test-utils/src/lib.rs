//! # foundry-test-utils
//!
//! Internal Foundry testing utilities.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Shouldn't use sh_* macros here, as they don't get captured by the test runner.
#![allow(clippy::disallowed_macros)]

#[macro_use]
extern crate tracing;

// Macros useful for testing.
#[macro_use]
mod macros;

pub mod rpc;

pub mod fd_lock;

mod filter;
pub use filter::Filter;

mod ext;
pub use ext::ExtTester;

mod prj;
pub use prj::{TestCommand, TestProject};

// Utilities for making it easier to handle tests.
pub mod util;

mod script;
pub use script::{ScriptOutcome, ScriptTester};

pub mod ui_runner;

// re-exports for convenience
pub use foundry_compilers;

pub use snapbox::{self, assert_data_eq, file, str};

/// Initializes tracing for tests.
pub fn init_tracing() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("RUST_BACKTRACE").is_none() {
            unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
        }
        let _ = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(env_filter())
            .with_test_writer()
            .try_init();
        let _ = ui_test::color_eyre::install();
    });
}

fn env_filter() -> tracing_subscriber::EnvFilter {
    const DEFAULT_DIRECTIVES: &[&str] = &include!("../../cli/src/utils/default_directives.txt");
    let mut filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive("foundry_test_utils=debug".parse().unwrap())
        .from_env_lossy();
    for &directive in DEFAULT_DIRECTIVES {
        filter = filter.add_directive(directive.parse().unwrap());
    }
    filter
}

pub fn test_debug(args: std::fmt::Arguments<'_>) {
    init_tracing();
    debug!("{args}");
}

pub fn test_trace(args: std::fmt::Arguments<'_>) {
    init_tracing();
    trace!("{args}");
}
