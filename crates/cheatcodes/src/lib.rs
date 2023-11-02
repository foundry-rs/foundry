//! # foundry-cheatcodes
//!
//! Foundry cheatcodes definitions and implementations.

#![warn(missing_docs, unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]
#![allow(elided_lifetimes_in_paths)] // Cheats context uses 3 lifetimes

#[cfg(feature = "impls")]
#[macro_use]
extern crate tracing;

// Silence the "unused crate" warning.
#[cfg(not(feature = "impls"))]
use alloy_primitives as _;

pub mod defs;
pub use defs::{Cheatcode, CheatcodeDef, Vm};

#[cfg(feature = "impls")]
pub mod impls;
#[cfg(feature = "impls")]
pub use impls::{Cheatcodes, CheatsConfig};

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    const JSON_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cheatcodes.json");
    #[cfg(feature = "schema")]
    const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cheatcodes.schema.json");
    const IFACE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/cheats/Vm.sol");

    /// Generates the `cheatcodes.json` file contents.
    fn json_cheatcodes() -> String {
        serde_json::to_string_pretty(&defs::Cheatcodes::new()).unwrap()
    }

    /// Generates the [cheatcodes](json_cheatcodes) JSON schema.
    #[cfg(feature = "schema")]
    fn json_schema() -> String {
        serde_json::to_string_pretty(&schemars::schema_for!(defs::Cheatcodes)).unwrap()
    }

    fn sol_iface() -> String {
        let cheats = defs::Cheatcodes::new().to_string().trim().replace('\n', "\n    ");
        format!(
            "\
// Automatically generated from `foundry-cheatcodes` Vm definitions. Do not modify manually.
// This interface is just for internal testing purposes. Use `forge-std` instead.

interface Vm {{
    {cheats}
}}
"
        )
    }

    #[test]
    fn defs_up_to_date() {
        ensure_file_contents(Path::new(JSON_PATH), &json_cheatcodes());
    }

    #[test]
    #[cfg(feature = "schema")]
    fn schema_up_to_date() {
        ensure_file_contents(Path::new(SCHEMA_PATH), &json_schema());
    }

    #[test]
    fn iface_up_to_date() {
        ensure_file_contents(Path::new(IFACE_PATH), &sol_iface());
    }

    /// Checks that the `file` has the specified `contents`. If that is not the
    /// case, updates the file and then fails the test.
    fn ensure_file_contents(file: &Path, contents: &str) {
        if let Ok(old_contents) = fs::read_to_string(file) {
            if normalize_newlines(&old_contents) == normalize_newlines(contents) {
                // File is already up to date.
                return
            }
        }

        eprintln!("\n\x1b[31;1merror\x1b[0m: {} was not up-to-date, updating\n", file.display());
        if std::env::var("CI").is_ok() {
            eprintln!("    NOTE: run `cargo test` locally and commit the updated files\n");
        }
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(file, contents).unwrap();
        panic!("some file was not up to date and has been updated, simply re-run the tests");
    }

    fn normalize_newlines(s: &str) -> String {
        s.replace("\r\n", "\n")
    }
}
