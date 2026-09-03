//! ChiselSession
//!
//! This module contains the `ChiselSession` struct, which is the top-level
//! wrapper for a serializable REPL session.

use crate::prelude::{SessionSource, SessionSourceConfig};
use eyre::Result;
use foundry_evm::{core::evm::FoundryEvmNetwork, executors::ExecutorBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::{OffsetDateTime, format_description};

/// A Chisel REPL Session
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct ChiselSession<FEN: FoundryEvmNetwork> {
    /// The `SessionSource` object that houses the REPL session.
    pub source: SessionSource<FEN>,
    /// The current session's identifier
    pub id: Option<String>,
}

// ChiselSession Common Associated Functions
impl<FEN: FoundryEvmNetwork> ChiselSession<FEN> {
    fn deserialize_cached(contents: &str, executor_builder: ExecutorBuilder<FEN>) -> Result<Self> {
        let mut session: Self = serde_json::from_str(contents)?;
        // A session load must not run project cleanup requested by cached configuration.
        session.source.config.foundry_config.force = false;
        session.source.config.executor_builder = executor_builder;
        Ok(session)
    }

    /// Create a new `ChiselSession` with a specified `solc` version and configuration.
    ///
    /// ### Takes
    ///
    /// An instance of [SessionSourceConfig]
    ///
    /// ### Returns
    ///
    /// A new instance of [ChiselSession]
    pub fn new(config: SessionSourceConfig<FEN>) -> Result<Self> {
        // Return initialized ChiselSession with set solc version
        Ok(Self { source: SessionSource::new(config)?, id: None })
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
        self.source.to_repl_source()
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

    /// Removes a cached session if it exists.
    pub fn remove_cached_session(id: &str) -> Result<()> {
        let cache_file = format!("{}chisel-{id}.json", Self::cache_dir()?);
        match std::fs::remove_file(cache_file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
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
        let entries = std::fs::read_dir(&cache_dir)?;
        let session_num = entries.filter(Result::is_ok).count();

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

    /// Returns a list of all available cached sessions.
    pub fn get_sessions() -> Result<Vec<(String, String)>> {
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
                OffsetDateTime::from(modified_time).format(&format_description::parse(
                    "[year]-[month]-[day] [hour]:[minute]:[second]",
                )?)?,
                file_name,
            ));
        }
        Ok(sessions)
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
    pub fn load(id: &str, executor_builder: ExecutorBuilder<FEN>) -> Result<Self> {
        let cache_dir = Self::cache_dir()?;
        let contents = std::fs::read_to_string(Path::new(&format!("{cache_dir}chisel-{id}.json")))?;
        Self::deserialize_cached(&contents, executor_builder)
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
    pub fn latest(executor_builder: ExecutorBuilder<FEN>) -> Result<Self> {
        let last_session = Self::latest_cached_session()?;
        let last_session_contents = std::fs::read_to_string(Path::new(&last_session))?;
        Self::deserialize_cached(&last_session_contents, executor_builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::{Config, SolcReq};
    use foundry_evm::core::evm::EthEvmNetwork;
    #[cfg(feature = "monad")]
    use foundry_evm::core::{constants::MONAD_CHEATCODE_ADDRESS, evm::MonadEvmNetwork};
    use semver::Version;

    #[test]
    fn deserialized_sessions_do_not_restore_force() {
        let session = ChiselSession::<EthEvmNetwork>::new(SessionSourceConfig {
            foundry_config: Config {
                force: true,
                solc: Some(SolcReq::Version(Version::new(0, 8, 29))),
                ..Default::default()
            },
            no_vm: true,
            ..Default::default()
        })
        .unwrap();
        assert!(session.source.config.foundry_config.force);

        let serialized = serde_json::to_string(&session).unwrap();
        let session = ChiselSession::<EthEvmNetwork>::deserialize_cached(
            &serialized,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap();

        assert!(!session.source.config.foundry_config.force);
    }

    #[cfg(feature = "monad")]
    #[test]
    fn deserialized_sessions_use_active_monad_tooling() {
        let session = ChiselSession::<MonadEvmNetwork>::new(SessionSourceConfig {
            executor_builder: ExecutorBuilder::<MonadEvmNetwork>::new(),
            ..Default::default()
        })
        .unwrap();
        let serialized = serde_json::to_string(&session).unwrap();

        let session = ChiselSession::<MonadEvmNetwork>::deserialize_cached(
            &serialized,
            ExecutorBuilder::<MonadEvmNetwork>::new(),
        )
        .unwrap();

        assert_eq!(
            session.source.config.executor_builder.extra_cheatcode_addresses(),
            &[MONAD_CHEATCODE_ADDRESS]
        );
    }
}
