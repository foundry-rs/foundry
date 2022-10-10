use core::fmt;
use std::{rc::Rc, path::Path};

use ethers_solc::project_util::TempProject;
use rustyline::Editor;
use serde::{Serialize, Deserialize, Serializer};

use eyre::Result;

pub use semver::Version;

/// Represents a parsed snippet of Solidity code.
#[derive(Debug)]
pub struct SolSnippet {
    /// The parsed source unit
    pub source_unit: (solang_parser::pt::SourceUnit, Vec<solang_parser::pt::Comment>),
    /// The raw source code
    pub raw: Rc<String>,
}

/// Deserialize a SourceUnit
pub fn deserialize_source_unit<'de, D>(deserializer: D) -> Result<(solang_parser::pt::SourceUnit, Vec<solang_parser::pt::Comment>), D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Grab the raw value
    let raw: Box<serde_json::value::RawValue> = match Box::deserialize(deserializer) {
        Ok(v) => v,
        Err(e) => {
            println!("Failed to deserialize into rawvalue box");
            return Err(e);
        }
    };

    // Parse the string, removing any quotes and adding them back in
    let raw_str = raw.get().trim_matches('"');

    // Parse the json value from string

    // Parse the serialized source unit string
    solang_parser::parse(&raw_str, 0).map_err(|_| serde::de::Error::custom("Failed to parse serialized string as source unit"))
}

impl Serialize for SolSnippet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(
            &format!(
                r#"{{
                    "source_unit": "{}",
                    "raw": "{}"
                }}"#,
                self.raw.as_str(),
                self.raw.as_str()
            )
        )
    }
}

/// Display impl for `SolToken`
impl fmt::Display for SolSnippet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

/// A Chisel REPL environment.
#[derive(Debug, Deserialize)]
pub struct ChiselEnv {
    /// The `TempProject` created for the REPL contract.
    pub project: TempProject,
    /// Session solidity version
    pub solc_version: Version,
    /// The `rustyline` Editor
    #[serde(skip)]
    pub rl: Editor<()>,
    /// The current session
    /// A session contains an ordered vector of source units, parsed by the solang-parser,
    /// as well as the raw source.
    pub session: Vec<SolSnippet>,
}

impl Serialize for ChiselEnv {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // We can serialize a json string 
        serializer.serialize_str(r#"{{
            "project": {},
            "solc_version": {},
            "session": {}
        }}"#
        )
    }
}

/// Chisel REPL environment impl
impl ChiselEnv {
    /// Create a new `ChiselEnv` with a specified `solc` version.
    pub fn new(solc_version: &'static str) -> Self {
        // Create initialized temporary dapptools-style project
        let mut project = Self::create_temp_project();

        // Parse the solc version
        let parsed_solc_version = Self::parse_solc_version(solc_version);

        // Set project's solc version explicitly
        project.set_solc(solc_version);

        // Create a new rustyline Editor
        let rl = Self::create_rustyline_editor();

        // Return initialized ChiselEnv with set solc version
        Self { solc_version: parsed_solc_version, project, rl, session: Vec::default() }
    }

    /// Create a default `ChiselEnv`.
    pub fn default() -> Self {
        Self {
            solc_version: ethers_solc::Solc::svm_global_version()
                .unwrap_or_else(|| Version::parse("0.8.17").unwrap()),
            project: Self::create_temp_project(),
            rl: Self::create_rustyline_editor(),
            session: Vec::default(),
        }
    }

    /// Render the full source code for the current session.
    /// TODO - Render source correctly rather than throwing
    /// everything into the fallback.
    pub fn contract_source(&self) -> String {
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {};
contract REPL {{
    fallback() {{
        {}
    }}
}}
        "#,
            self.solc_version,
            self.session.iter().map(|t| t.to_string()).collect::<Vec<String>>().join("\n")
        )
    }

    /// Writes the ChiselEnv to a file by serializing it to a JSON string
    pub fn write() -> Result<()> {
        // TODO: Write the ChiselEnv to a cache file
        Ok(())
    }

    /// The Chisel Cache Directory
    pub fn cache_dir() -> Result<String> {
        let home_dir = dirs::home_dir().ok_or(eyre::eyre!("Failed to grab home directory"))?;
        let home_dir_str = home_dir.to_str().ok_or(eyre::eyre!("Failed to convert home directory to string"))?;
        Ok(format!("{}/.chisel/", home_dir_str))
    }

    /// Gets the most recent chisel session from the cache dir
    pub fn latest_chached_session() -> Result<String> {
        let cache_dir = Self::cache_dir()?;
        let mut entries = std::fs::read_dir(cache_dir)?;
        let mut latest = entries.next().unwrap().unwrap();
        for entry in entries {
            let entry = entry.unwrap();
            if entry.metadata().unwrap().modified().unwrap() > latest.metadata().unwrap().modified().unwrap() {
                latest = entry;
            }
        }
        Ok(latest.path().to_str().unwrap().to_string())
    }

    /// Loads a ChiselEnv from the cache file
    pub fn load() -> Result<Self> {
        let last_session = Self::latest_chached_session()?;
        let last_session_contents = std::fs::read_to_string(Path::new(&last_session))?;
        let chisel_env: ChiselEnv = serde_json::from_str(&last_session_contents)?;
        Ok(chisel_env)
    }

    /// Helper function to parse a solidity version string.
    ///
    /// # Panics
    ///
    /// Panics if the version string is not a valid semver version.
    pub fn parse_solc_version(solc_version: &'static str) -> Version {
        Version::parse(solc_version).unwrap_or_else(|e| {
            tracing::error!("Error parsing provided solc version: \"{}\"", e);
            panic!("Error parsing provided solc version: \"{e}\"");
        })
    }

    /// Helper function to create a new temporary project with proper error handling.
    ///
    /// ### Panics
    ///
    /// Panics if the temporary project cannot be created.
    pub(crate) fn create_temp_project() -> TempProject {
        TempProject::dapptools_init().unwrap_or_else(|e| {
            tracing::error!(target: "chisel-env", "Failed to initialize temporary project! {}", e);
            panic!("failed to create a temporary project for the chisel environment! {e}");
        })
    }

    /// Helper function to create a new rustyline Editor with proper error handling.
    ///
    /// ### Panics
    ///
    /// Panics if the rustyline Editor cannot be created.
    pub(crate) fn create_rustyline_editor() -> Editor<()> {
        Editor::<()>::new().unwrap_or_else(|e| {
            tracing::error!(target: "chisel-env", "Failed to initialize rustyline Editor! {}", e);
            panic!("failed to create a rustyline Editor for the chisel environment! {e}");
        })
    }
}
