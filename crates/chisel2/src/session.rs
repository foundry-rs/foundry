//! ChiselSession
//!
//! This module contains the `ChiselSession` struct, which is the top-level
//! wrapper for a serializable REPL session.

use crate::prelude::{SessionSource, SessionSourceConfig};
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::{format_description, OffsetDateTime};

/// A Chisel REPL Session
#[derive(Debug, Serialize, Deserialize)]
pub struct ChiselSession {
    /// The `SessionSource` object that houses the REPL session.
    pub session_source: SessionSource,
    /// The current session's identifier
    pub id: Option<String>,
}

// ChiselSession Common Associated Functions
impl ChiselSession {
    /// Create a new `ChiselSession` with a specified `solc` version and configuration.
    ///
    /// ### Takes
    ///
    /// An instance of [SessionSourceConfig]
    ///
    /// ### Returns
    ///
    /// A new instance of [ChiselSession]
    pub fn new(config: SessionSourceConfig) -> Result<Self> {
        let solc = config.solc()?;
        // Return initialized ChiselSession with set solc version
        Ok(Self { session_source: SessionSource::new(solc, config), id: None })
    }

    /// Render the full source code for the current session.
    ///
    /// ### Returns
    ///
    /// Returns the full, flattened source code for the current session.
    ///
    /// ### Notes
    ///
    /// This function will not panic, but will return a blank string if the
    /// session's [SessionSource] is None.
    pub fn contract_source(&self) -> String {
        self.session_source.to_repl_source()
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

        let cache_file_name = match self.id.as_ref() {
            Some(id) => {
                // ID is already set- use the existing cache file.
                format!("{cache_dir}chisel-{id}.json")
            }
            None => {
                // Get the next session cache ID / file
                let (id, file_name) = Self::next_cached_session()?;
                // Set the session's ID
                self.id = Some(id);
                // Return the new session's cache file name
                file_name
            }
        };

        // Write the current ChiselSession to that file
        let serialized_contents = serde_json::to_string_pretty(self)?;
        std::fs::write(&cache_file_name, serialized_contents)?;

        // Return the full cache file path
        // Ex: /home/user/.foundry/cache/chisel/chisel-0.json
        Ok(cache_file_name)
    }

    /// Get the next default session cache file name
    ///
    /// ### Returns
    ///
    /// Optionally, returns a tuple containing the next cached session's id and file name.
    pub fn next_cached_session() -> Result<(String, String)> {
        let cache_dir = Self::cache_dir()?;
        let mut entries = std::fs::read_dir(&cache_dir)?;

        // If there are no existing cached sessions, just create the first one: "chisel-0.json"
        let mut latest = if let Some(e) = entries.next() {
            e?
        } else {
            return Ok((String::from("0"), format!("{cache_dir}chisel-0.json")))
        };

        let mut session_num = 1;
        // Get the latest cached session
        for entry in entries {
            let entry = entry?;
            if entry.metadata()?.modified()? >= latest.metadata()?.modified()? {
                latest = entry;
            }

            // Increase session_num counter rather than cloning the iterator and using `.count`
            session_num += 1;
        }

        Ok((format!("{session_num}"), format!("{cache_dir}chisel-{session_num}.json")))
    }

    /// The Chisel Cache Directory
    ///
    /// ### Returns
    ///
    /// Optionally, the directory of the chisel cache.
    pub fn cache_dir() -> Result<String> {
        let home_dir =
            dirs::home_dir().ok_or_else(|| eyre::eyre!("Failed to grab home directory"))?;
        let home_dir_str = home_dir
            .to_str()
            .ok_or_else(|| eyre::eyre!("Failed to convert home directory to string"))?;
        Ok(format!("{home_dir_str}/.foundry/cache/chisel/"))
    }

    /// Create the cache directory if it does not exist
    ///
    /// ### Returns
    ///
    /// The unit type if the operation was successful.
    pub fn create_cache_dir() -> Result<()> {
        let cache_dir = Self::cache_dir()?;
        if !Path::new(&cache_dir).exists() {
            std::fs::create_dir_all(&cache_dir)?;
        }
        Ok(())
    }

    /// Lists all available cached sessions
    ///
    /// ### Returns
    ///
    /// Optionally, a vector containing tuples of session IDs and cache-file names.
    pub fn list_sessions() -> Result<Vec<(String, String)>> {
        // Read the cache directory entries
        let cache_dir = Self::cache_dir()?;
        let entries = std::fs::read_dir(cache_dir)?;

        // For each entry, get the file name and modified time
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry?;
            let modified_time = entry.metadata()?.modified()?;
            let file_name = entry.file_name();
            let file_name = file_name
                .into_string()
                .map_err(|e| eyre::eyre!(format!("{}", e.to_string_lossy())))?;
            sessions.push((
                systemtime_strftime(modified_time, "[year]-[month]-[day] [hour]:[minute]:[second]")
                    .unwrap(),
                file_name,
            ));
        }

        if sessions.is_empty() {
            eyre::bail!("No sessions found!")
        } else {
            // Return the list of sessions and their modified times
            Ok(sessions)
        }
    }

    /// Loads a specific ChiselSession from the specified cache file
    ///
    /// ### Takes
    ///
    /// The ID of the chisel session that you wish to load.
    ///
    /// ### Returns
    ///
    /// Optionally, an owned instance of the loaded chisel session.
    pub fn load(id: &str) -> Result<Self> {
        let cache_dir = Self::cache_dir()?;
        let contents = std::fs::read_to_string(Path::new(&format!("{cache_dir}chisel-{id}.json")))?;
        let chisel_env: Self = serde_json::from_str(&contents)?;
        Ok(chisel_env)
    }

    /// Gets the most recent chisel session from the cache dir
    ///
    /// ### Returns
    ///
    /// Optionally, the file name of the most recently modified cached session.
    pub fn latest_cached_session() -> Result<String> {
        let cache_dir = Self::cache_dir()?;
        let mut entries = std::fs::read_dir(cache_dir)?;
        let mut latest = entries.next().ok_or_else(|| eyre::eyre!("No entries found!"))??;
        for entry in entries {
            let entry = entry?;
            if entry.metadata()?.modified()? > latest.metadata()?.modified()? {
                latest = entry;
            }
        }
        Ok(latest
            .path()
            .to_str()
            .ok_or_else(|| eyre::eyre!("Failed to get session path!"))?
            .to_string())
    }

    /// Loads the latest ChiselSession from the cache file
    ///
    /// ### Returns
    ///
    /// Optionally, an owned instance of the most recently modified cached session.
    pub fn latest() -> Result<Self> {
        let last_session = Self::latest_cached_session()?;
        let last_session_contents = std::fs::read_to_string(Path::new(&last_session))?;
        let chisel_env: Self = serde_json::from_str(&last_session_contents)?;
        Ok(chisel_env)
    }
}

/// Generic helper function that attempts to convert a type that has
/// an [`Into<OffsetDateTime>`] implementation into a formatted date string.
fn systemtime_strftime<T>(dt: T, format: &str) -> Result<String>
where
    T: Into<OffsetDateTime>,
{
    Ok(dt.into().format(&format_description::parse(format)?)?)
}
