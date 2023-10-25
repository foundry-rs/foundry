#![warn(unused_crate_dependencies)]

// Macros useful for testing.
mod macros;

// Utilities for making it easier to handle tests.
pub mod util;
pub use util::{TestCommand, TestProject};

pub mod script;
pub use script::{ScriptOutcome, ScriptTester};

// re-exports for convenience
pub use foundry_compilers;
pub use tempfile;
