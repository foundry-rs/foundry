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

/// Rejects a session id that would let `chisel-<id>.json` escape the cache directory when
/// concatenated into a path (e.g. `../../etc/cron.d/evil`, which yields the literal path
/// component `chisel-..`, followed by a real `..` component once the id itself contains a `/`).
/// Also rejects `:` to prevent targeting Windows Alternate Data Streams (ADS).
fn validate_session_id(id: &str) -> Result<()> {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\', ':']) {
        eyre::bail!(
            "invalid Chisel session id `{id}`: must not be empty, `.`, `..`, or contain a path \
             separator or `:`"
        );
    }
    Ok(())
}

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
        validate_session_id(id)?;
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
                validate_session_id(id)?;
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
    ///
    /// Uses one past the highest numeric ID to avoid collisions after deletion.
    pub fn next_cached_session() -> Result<(String, String)> {
        Self::next_cached_session_in(&Self::cache_dir()?)
    }

    fn next_cached_session_in(cache_dir: &str) -> Result<(String, String)> {
        let next_id = std::fs::read_dir(cache_dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()?
                    .strip_prefix("chisel-")?
                    .strip_suffix(".json")?
                    .parse::<usize>()
                    .ok()
            })
            .max()
            .map_or(Some(0), |max| max.checked_add(1))
            .ok_or_else(|| eyre::eyre!("no unused chisel session id available"))?;

        Ok((format!("{next_id}"), format!("{cache_dir}chisel-{next_id}.json")))
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
        Self::load_from(id, &Self::cache_dir()?, executor_builder)
    }

    fn load_from(
        id: &str,
        cache_dir: &str,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<Self> {
        validate_session_id(id)?;
        let contents = std::fs::read_to_string(Path::new(&format!("{cache_dir}chisel-{id}.json")))?;
        let mut session = Self::deserialize_cached(&contents, executor_builder)?;
        // Use the requested ID even if the cached ID is missing or stale.
        session.id = Some(id.to_string());
        Ok(session)
    }

    /// Gets the most recent chisel session from the cache dir
    ///
    /// ### Returns
    ///
    /// Optionally, the file name of the most recently modified cached session.
    pub fn latest_cached_session() -> Result<String> {
        Self::latest_cached_session_in(&Self::cache_dir()?)
    }

    fn latest_cached_session_in(cache_dir: &str) -> Result<String> {
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
        Self::latest_from(&Self::cache_dir()?, executor_builder)
    }

    fn latest_from(cache_dir: &str, executor_builder: ExecutorBuilder<FEN>) -> Result<Self> {
        let last_session = Self::latest_cached_session_in(cache_dir)?;
        let last_session_contents = std::fs::read_to_string(Path::new(&last_session))?;
        let mut session = Self::deserialize_cached(&last_session_contents, executor_builder)?;
        // Bind the session to the file that was loaded.
        session.id = Self::session_id_from_cache_file_name(&last_session);
        Ok(session)
    }

    /// Extracts the session id from a `.../chisel-<id>.json` cache file path.
    fn session_id_from_cache_file_name(path: &str) -> Option<String> {
        Path::new(path).file_stem()?.to_str()?.strip_prefix("chisel-").map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::{Config, SolcReq};
    use foundry_evm::core::evm::EthEvmNetwork;
    use semver::Version;

    #[cfg(feature = "monad")]
    use foundry_evm::core::{constants::MONAD_CHEATCODE_ADDRESS, evm::MonadEvmNetwork};

    /// Deleted sessions must not cause the next ID to collide with an existing file.
    #[test]
    fn next_cached_session_skips_gaps_left_by_deleted_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().to_str().unwrap());

        // Sessions 0 and 2 exist; session 1 was deleted or renamed away, leaving a gap.
        std::fs::write(format!("{cache_dir}chisel-0.json"), "{\"id\":\"0\"}").unwrap();
        std::fs::write(format!("{cache_dir}chisel-2.json"), "{\"id\":\"2\"}").unwrap();

        let (next_id, next_file) =
            ChiselSession::<EthEvmNetwork>::next_cached_session_in(&cache_dir).unwrap();

        // Counting entries would select the occupied ID 2.
        assert_eq!(next_id, "3", "must skip past the gap instead of reusing the occupied id 2");
        assert_eq!(next_file, format!("{cache_dir}chisel-3.json"));

        assert_eq!(
            std::fs::read_to_string(format!("{cache_dir}chisel-0.json")).unwrap(),
            "{\"id\":\"0\"}"
        );
        assert_eq!(
            std::fs::read_to_string(format!("{cache_dir}chisel-2.json")).unwrap(),
            "{\"id\":\"2\"}"
        );
    }

    #[test]
    fn next_cached_session_does_not_overflow_on_a_usize_max_named_session() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().to_str().unwrap());
        std::fs::write(format!("{cache_dir}chisel-{}.json", usize::MAX), "{}").unwrap();

        let result = ChiselSession::<EthEvmNetwork>::next_cached_session_in(&cache_dir);
        assert!(result.is_err(), "must error instead of panicking or wrapping to a reused id");
    }

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

    /// A session id containing a path separator lets `chisel-<id>.json` escape the cache
    /// directory once resolved: `chisel-x/../../../foo.json` has real `..` path components
    /// after the `x` segment, walking back out past the cache directory entirely.
    /// Also verifies that `:` is rejected to prevent targeting NTFS Alternate Data Streams (ADS).
    #[test]
    fn path_traversal_ids_are_rejected() {
        for id in [
            "../evil",
            "x/../../../../../../tmp/pwned",
            "..",
            ".",
            "",
            "sub/dir",
            "back\\slash",
            ":colon",
            "foo:bar",
            "session:1",
        ] {
            let err = validate_session_id(id).unwrap_err();
            assert!(err.to_string().contains("invalid Chisel session id"), "{id:?}: {err}");
        }

        // ordinary numeric and name-like ids remain accepted
        for id in ["0", "42", "my-session", "my_session"] {
            validate_session_id(id).unwrap();
        }
    }

    #[test]
    fn load_rejects_path_traversal_id() {
        let err = ChiselSession::<EthEvmNetwork>::load(
            "../../evil",
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid Chisel session id"), "{err}");
    }

    #[test]
    fn remove_cached_session_rejects_path_traversal_id() {
        let err = ChiselSession::<EthEvmNetwork>::remove_cached_session("../../evil").unwrap_err();
        assert!(err.to_string().contains("invalid Chisel session id"), "{err}");
    }

    fn session_for_normalization_tests() -> ChiselSession<EthEvmNetwork> {
        ChiselSession::<EthEvmNetwork>::new(SessionSourceConfig {
            foundry_config: Config {
                solc: Some(SolcReq::Version(Version::new(0, 8, 29))),
                ..Default::default()
            },
            no_vm: true,
            ..Default::default()
        })
        .unwrap()
    }

    /// Loading uses the filename rather than a stale or missing cached ID.
    #[test]
    fn load_normalizes_id_ignoring_a_stale_or_missing_embedded_id() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().to_str().unwrap());

        let mut session = session_for_normalization_tests();
        session.id = Some("stale-name".to_string());
        let serialized = serde_json::to_string(&session).unwrap();
        std::fs::write(format!("{cache_dir}chisel-5.json"), &serialized).unwrap();

        let loaded = ChiselSession::<EthEvmNetwork>::load_from(
            "5",
            &cache_dir,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap();
        assert_eq!(loaded.id.as_deref(), Some("5"), "must use the requested id, not the stale one");

        let without_id = serialized.replacen("\"stale-name\"", "null", 1);
        std::fs::write(format!("{cache_dir}chisel-7.json"), without_id).unwrap();
        let loaded = ChiselSession::<EthEvmNetwork>::load_from(
            "7",
            &cache_dir,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap();
        assert_eq!(loaded.id.as_deref(), Some("7"), "a null embedded id must not survive the load");
    }

    #[test]
    fn latest_normalizes_id_from_the_resolved_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().to_str().unwrap());

        let session = session_for_normalization_tests();
        // New sessions serialize with a null ID.
        let serialized = serde_json::to_string(&session).unwrap();
        std::fs::write(format!("{cache_dir}chisel-9.json"), serialized).unwrap();

        let loaded = ChiselSession::<EthEvmNetwork>::latest_from(
            &cache_dir,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap();
        assert_eq!(loaded.id.as_deref(), Some("9"));
    }

    #[test]
    fn session_id_from_cache_file_name_strips_prefix_and_extension() {
        assert_eq!(
            ChiselSession::<EthEvmNetwork>::session_id_from_cache_file_name(
                "/home/user/.foundry/cache/chisel/chisel-42.json"
            ),
            Some("42".to_string())
        );
        assert_eq!(
            ChiselSession::<EthEvmNetwork>::session_id_from_cache_file_name(
                "/home/user/.foundry/cache/chisel/not-a-session-file.json"
            ),
            None
        );
    }

    #[test]
    fn write_rejects_path_traversal_id() {
        let mut session = ChiselSession::<EthEvmNetwork>::new(SessionSourceConfig {
            foundry_config: Config {
                solc: Some(SolcReq::Version(Version::new(0, 8, 29))),
                ..Default::default()
            },
            no_vm: true,
            ..Default::default()
        })
        .unwrap();
        session.id = Some("../../evil".to_string());

        let err = session.write().unwrap_err();
        assert!(err.to_string().contains("invalid Chisel session id"), "{err}");
    }
}
