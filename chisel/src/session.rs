use crate::prelude::SessionSource;
use ethers_solc::{project_util::TempProject, Solc};
use eyre::Result;
pub use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// A Chisel REPL Session
#[derive(Debug, Serialize, Deserialize)]
pub struct ChiselSession {
    /// The `SessionSource` object that houses the REPL session.
    #[serde(skip)]
    pub session_source: Option<SessionSource>,
    /// Session solidity version]
    pub solc_version: Version,
    /// The current session's identifier
    pub id: Option<usize>,
}

impl Default for ChiselSession {
    fn default() -> Self {
        // TODO: Fetch latest version rather than hard-coding it (?)
        Self {
            session_source: Some({
                let solc = Solc::find_or_install_svm_version("0.8.17").unwrap();
                assert!(solc.version().is_ok());
                SessionSource {
                    file_name: PathBuf::from("ReplContract.sol".to_string()),
                    contract_name: "REPL".to_string(),
                    solc,
                    global_code: Default::default(),
                    top_level_code: Default::default(),
                    run_code: Default::default(),
                    compiled: None,
                    intermediate: None,
                    generated_output: None,
                }
            }),
            solc_version: Version::new(0, 8, 17),
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

        let solc = Solc::find_or_install_svm_version(solc_version);

        // TODO: Either handle gracefully or document that this
        // constructor can panic. Also should notify the dev if
        // we're installing a new solc version on their behalf.
        assert!(solc.is_ok());

        // Return initialized ChiselSession with set solc version
        Self {
            solc_version: parsed_solc_version,
            session_source: Some(SessionSource::new(&solc.unwrap())),
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

    /// Render the full source code for the current session.
    ///
    /// ### Return
    ///
    /// Returns the full, flattened source code for the current session.
    ///
    /// ### Notes
    ///
    /// This function will not panic, but will return a blank string if the
    /// session's [SessionSource] is None.
    pub fn contract_source(&self) -> String {
        if let Some(source) = &self.session_source {
            source.to_string()
        } else {
            String::default()
        }
    }

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
