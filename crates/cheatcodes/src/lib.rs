//! # foundry-cheatcodes
//!
//! Foundry cheatcodes definitions and implementations.

#![warn(missing_docs, unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]
#![allow(elided_lifetimes_in_paths)] // Cheats context uses 3 lifetimes

#[cfg(feature = "impls")]
#[macro_use]
extern crate tracing;

use alloy_primitives::{address, Address};

mod defs;
pub use defs::{Cheatcode, CheatcodeDef, Group, Mutability, Safety, Status, Visibility, Vm};

#[cfg(feature = "impls")]
pub mod impls;
#[cfg(feature = "impls")]
pub use impls::{Cheatcodes, CheatsConfig};

/// The cheatcode handler address.
///
/// This is the same address as the one used in DappTools's HEVM.
/// It is calculated as:
/// `address(bytes20(uint160(uint256(keccak256('hevm cheat code')))))`
pub const CHEATCODE_ADDRESS: Address = address!("7109709ECfa91a80626fF3989D68f67F5b1DD12D");

/// The Hardhat console address.
///
/// See: <https://github.com/nomiclabs/hardhat/blob/master/packages/hardhat-core/console.sol>
pub const HARDHAT_CONSOLE_ADDRESS: Address = address!("000000000000000000636F6e736F6c652e6c6f67");

/// Address of the default `CREATE2` deployer.
pub const DEFAULT_CREATE2_DEPLOYER: Address = address!("4e59b44847b379578588920ca78fbf26c0b4956c");

/// Generates the `cheatcodes.json` file contents.
pub fn json_cheatcodes() -> String {
    serde_json::to_string_pretty(Vm::CHEATCODES).unwrap()
}

/// Generates the [cheatcodes](json_cheatcodes) JSON schema.
#[cfg(feature = "schema")]
pub fn json_schema() -> String {
    // use a custom type to add a title and description to the schema
    /// Foundry cheatcodes. Learn more: <https://book.getfoundry.sh/cheatcodes/>
    #[derive(schemars::JsonSchema)]
    struct Cheatcodes([Cheatcode<'static>]);

    serde_json::to_string_pretty(&schemars::schema_for!(Cheatcodes)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    const JSON_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cheatcodes.json");
    #[cfg(feature = "schema")]
    const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cheatcodes.schema.json");

    #[test]
    fn defs_up_to_date() {
        ensure_file_contents(Path::new(JSON_PATH), &json_cheatcodes());
    }

    #[test]
    #[cfg(feature = "schema")]
    fn schema_up_to_date() {
        ensure_file_contents(Path::new(SCHEMA_PATH), &json_schema());
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
