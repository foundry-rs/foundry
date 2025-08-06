//! Config specification for Foundry.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use foundry_config::Config;
use serde::{Deserialize, Serialize};

// The `config.json` schema.
/// Foundry configuration. Learn more: <https://getfoundry.sh/config/overview>
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ConfigSchema {
    #[serde(flatten)]
    pub config: Config,
}

#[cfg(test)]
#[expect(clippy::disallowed_macros)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[cfg(feature = "schema")]
    const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/config.schema.json");

    /// Generates the configuration JSON schema.
    #[cfg(feature = "schema")]
    fn json_schema() -> String {
        serde_json::to_string_pretty(&schemars::schema_for!(ConfigSchema)).unwrap()
    }

    #[test]
    #[cfg(feature = "schema")]
    fn schema_up_to_date() {
        ensure_file_contents(Path::new(SCHEMA_PATH), &json_schema());
    }

    /// Checks that the `file` has the specified `contents`. If that is not the
    /// case, updates the file and then fails the test.
    fn ensure_file_contents(file: &Path, contents: &str) {
        if let Ok(old_contents) = fs::read_to_string(file)
            && normalize_newlines(&old_contents) == normalize_newlines(contents)
        {
            // File is already up to date.
            return;
        }

        eprintln!("\n\x1b[31;1merror\x1b[0m: {} was not up-to-date, updating\n", file.display());
        if std::env::var("CI").is_ok() {
            eprintln!("    NOTE: run `cargo spec-config` locally and commit the updated files\n");
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
