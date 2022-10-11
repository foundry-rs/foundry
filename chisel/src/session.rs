use std::{path::Path, time::SystemTime};

use ethers_solc::project_util::TempProject;
use serde::{Deserialize, Serialize};

use eyre::Result;

pub use semver::Version;
use solang_parser::pt::{Import, SourceUnitPart};

use crate::parser::ParsedSnippet;

/// A Chisel REPL Session
#[derive(Debug, Serialize, Deserialize)]
pub struct ChiselSession {
    /// The `TempProject` created for the REPL contract.
    #[serde(skip)]
    pub project: Option<TempProject>,
    /// Session solidity version]
    pub solc_version: Version,
    /// Snippets
    /// This is a list of the raw solidity source code strings and their solang_parser parsed
    /// SourceUnits.
    pub snippets: Vec<ParsedSnippet>,
    /// The current session's identifier
    pub id: Option<usize>,
}

impl Default for ChiselSession {
    fn default() -> Self {
        Self {
            project: Some(Self::create_temp_project()),
            solc_version: Version::new(0, 8, 17),
            snippets: vec![],
            id: None,
        }
    }
}

// ChiselSession Common Associated Functions
impl ChiselSession {
    /// Create a new `ChiselSession` with a specified `solc` version.
    pub fn new(solc_version: &'static str) -> Self {
        // Create initialized temporary dapptools-style project
        let mut project = Self::create_temp_project();

        // Parse the solc version
        let parsed_solc_version = Self::parse_solc_version(solc_version);

        // Set project's solc version explicitly
        project.set_solc(solc_version);

        // Return initialized ChiselSession with set solc version
        Self {
            solc_version: parsed_solc_version,
            project: Some(project),
            snippets: Vec::default(),
            id: None,
        }
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
            panic!("failed to create a temporary project for the chisel session! {e}");
        })
    }
}

impl ChiselSession {
    // TODO:::: This should follow soli's pattern of contract generation, for _each_ contract
    // defined by our sessions's ParsedSnippets TODO:::: We define generation in
    // [generator.rs](./generator.rs), following soli's pattern.

    /// Render the full source code for the current session.
    ///
    /// ### Return
    ///
    /// Returns the full, flattened source code for the current session.
    ///
    /// ### Notes
    ///
    /// This function will not panic, but gracefully handles errors.
    ///
    /// For source code to render correctly, crafting sol snippets must be done with care to ensure
    /// correct source unit ordering.
    ///
    /// For example, a sol snippet with a variable declaration, followed by an event definition will
    /// fail to render correctly. This is because the variable declaration is not a "top-level"
    /// source unit part, so the sol snippet will be placed **entirely** in the contract
    /// fallback. This will then error since events cannot be defined from within the contract
    /// fallback function.
    pub fn contract_source(&self) -> String {
        // Extract a pragma definition
        // NOTE: Optimistically uses the first pragma found
        let pragma_def = self.snippets.iter().find(|i| {
            i.source_unit
                .as_ref()
                .unwrap()
                .0
                 .0
                .iter()
                .any(|i| matches!(i, SourceUnitPart::PragmaDirective(_, _, _)))
        });

        // Extract imports
        let imports = self
            .snippets
            .iter()
            .flat_map(|i| {
                i.source_unit
                    .as_ref()
                    .unwrap()
                    .0
                     .0
                    .iter()
                    .filter(|i| matches!(i, SourceUnitPart::ImportDirective(_)))
                    .map(|sup| {
                        if let SourceUnitPart::ImportDirective(sup) = sup {
                            let string_literal = match sup {
                                Import::Plain(sl, _) => sl,
                                Import::GlobalSymbol(sl, _, _) => sl,
                                Import::Rename(sl, _, _) => sl,
                            };
                            string_literal.string.clone()
                        } else {
                            unreachable!()
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<String>>()
            .join("\n");

        // TODO: Extract contract definitions

        // Consume source units that are top-level
        // We only need to check the first source unit part for each sol snippet
        // If the first part is not top level, we should throw the sol snippet in the fallback
        let top_level_units = self
            .snippets
            .iter()
            .filter(|unit| {
                if let Some(def) = unit.source_unit.as_ref().unwrap().0 .0.get(0) {
                    match def {
                        SourceUnitPart::PragmaDirective(_, _, _) => false,
                        SourceUnitPart::ContractDefinition(_) => false,
                        SourceUnitPart::ImportDirective(_) => false,
                        SourceUnitPart::EnumDefinition(_) => true,
                        SourceUnitPart::StructDefinition(_) => true,
                        SourceUnitPart::EventDefinition(_) => true,
                        SourceUnitPart::ErrorDefinition(_) => true,
                        SourceUnitPart::FunctionDefinition(_) => true,
                        SourceUnitPart::VariableDefinition(_) => false,
                        SourceUnitPart::TypeDefinition(_) => true,
                        SourceUnitPart::Using(_) => true,
                        SourceUnitPart::StraySemicolon(_) => false,
                    }
                } else {
                    false
                }
            })
            .map(|unit| unit.raw.as_str())
            .collect::<Vec<&str>>()
            .join("\n\n");

        // Extract fallback snippets
        let fallback_snippets = self
            .snippets
            .iter()
            .filter(|unit| {
                matches!(
                    unit.source_unit.as_ref().unwrap().0 .0.get(0),
                    Some(SourceUnitPart::VariableDefinition(_))
                )
            })
            .map(|unit| unit.raw.as_str())
            .collect::<Vec<&str>>()
            .join("\n");

        // Generate the final source
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
{}

// Imports
{}

/// @title REPL
/// @notice Auto-generated by Chisel
/// @notice See: https://github.com/foundry-rs/foundry/tree/master/chisel
contract REPL {{
    {}

    fallback() {{
        {}
    }}
}}
        "#,
            pragma_def
                .map(|p| p.to_string())
                .unwrap_or_else(|| format!("pragma solidity {};", self.solc_version)),
            imports,
            top_level_units,
            fallback_snippets
        )
    }
}

// ChiselSession Cache Functionality
impl ChiselSession {
    /// Clears the cache directory
    ///
    /// ### WARNING
    ///
    /// This will delete all sessions from the cache.
    /// There is no method of recovering these deleted sessions.
    pub fn clear_cache() -> Result<()> {
        let cache_dir = Self::cache_dir()?;
        for entry in std::fs::read_dir(cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// Writes the ChiselSession to a file by serializing it to a JSON string
    ///
    /// ### Returns
    ///
    /// Returns the path of the new cache file
    pub fn write(&mut self) -> Result<String> {
        // Try to create the cache directory
        let cache_dir = Self::cache_dir()?;
        std::fs::create_dir_all(&cache_dir)?;

        // If the id field is set, we don't need to generate a new cache file
        if let Some(id) = self.id {
            return Ok(format!("{}chisel-{}.json", cache_dir, id))
        }

        // Get the next cached session name
        let (id, cache_file_name) = Self::next_cached_session()?;
        self.id = Some(id);

        // Write the current ChiselSession to that file
        let serialized_contents = serde_json::to_string_pretty(self)?;
        std::fs::write(&cache_file_name, serialized_contents)?;

        // Return the full cache file path
        // Ex: /home/user/.foundry/cache/chisel/chisel-0.json
        Ok(cache_file_name)
    }

    /// Get the next session cache file name
    pub fn next_cached_session() -> Result<(usize, String)> {
        let cache_dir = Self::cache_dir()?;
        let mut entries = std::fs::read_dir(&cache_dir)?;

        // If there are no existing cached sessions, just create the first one: "chisel-0.json"
        let mut latest = if let Some(e) = entries.next() {
            e?
        } else {
            return Ok((0, format!("{}chisel-0.json", cache_dir)))
        };

        // Get the latest cached session
        for entry in entries {
            let entry = entry?;
            if entry.metadata()?.modified()? >= latest.metadata()?.modified()? {
                latest = entry;
            }
        }

        // Get the latest session cache file name
        let latest_file_name = latest
            .file_name()
            .into_string()
            .map_err(|e| eyre::eyre!(format!("{}", e.to_string_lossy())))?;
        let session_num = latest_file_name.trim_end_matches(".json").trim_start_matches("chisel-");
        let session_num = session_num.parse::<usize>()?;

        Ok((session_num + 1, format!("{}chisel-{}.json", cache_dir, session_num + 1)))
    }

    /// The Chisel Cache Directory
    pub fn cache_dir() -> Result<String> {
        let home_dir = dirs::home_dir().ok_or(eyre::eyre!("Failed to grab home directory"))?;
        let home_dir_str =
            home_dir.to_str().ok_or(eyre::eyre!("Failed to convert home directory to string"))?;
        Ok(format!("{}/.foundry/cache/chisel/", home_dir_str))
    }

    /// Create the cache directory if it does not exist
    pub fn create_cache_dir() -> Result<()> {
        let cache_dir = Self::cache_dir()?;
        if !Path::new(&cache_dir).exists() {
            std::fs::create_dir_all(&cache_dir)?;
        }
        Ok(())
    }

    /// Lists all available cached sessions
    pub fn list_sessions() -> Result<Vec<(SystemTime, String)>> {
        // Read the cache directory entries
        let cache_dir = Self::cache_dir()?;
        let entries = std::fs::read_dir(&cache_dir)?;

        // For each entry, get the file name and modified time
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry?;
            let modified_time = entry.metadata()?.modified()?;
            let file_name = entry.file_name();
            let file_name = file_name
                .into_string()
                .map_err(|e| eyre::eyre!(format!("{}", e.to_string_lossy())))?;
            sessions.push((modified_time, file_name));
        }

        // Return the list of sessions and their modified times
        Ok(sessions)
    }

    /// Gets the most recent chisel session from the cache dir
    pub fn latest_chached_session() -> Result<String> {
        let cache_dir = Self::cache_dir()?;
        let mut entries = std::fs::read_dir(cache_dir)?;
        let mut latest = entries.next().ok_or(eyre::eyre!("No entries found!"))??;
        for entry in entries {
            let entry = entry?;
            if entry.metadata()?.modified()? > latest.metadata()?.modified()? {
                latest = entry;
            }
        }
        Ok(latest.path().to_str().ok_or(eyre::eyre!("Failed to get session path!"))?.to_string())
    }

    /// Loads a specific ChiselSession from the specified cache file
    pub fn load(name: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(Path::new(name))?;
        let chisel_env: ChiselSession = serde_json::from_str(&contents)?;
        Ok(chisel_env)
    }

    /// Loads the latest ChiselSession from the cache file
    pub fn latest() -> Result<Self> {
        let last_session = Self::latest_chached_session()?;
        let last_session_contents = std::fs::read_to_string(Path::new(&last_session))?;
        let chisel_env: ChiselSession = serde_json::from_str(&last_session_contents)?;
        Ok(chisel_env)
    }
}
