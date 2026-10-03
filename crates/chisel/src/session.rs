//! ChiselSession
//!
//! This module contains the `ChiselSession` struct, which is the top-level
//! wrapper for a serializable REPL session.

use crate::prelude::{SessionSource, SessionSourceConfig};
use eyre::Result;
use foundry_evm::{core::evm::FoundryEvmNetwork, executors::ExecutorBuilder};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, format_description};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

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
        session.source.config.clear_credentials();
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
        self.write_to(&Self::cache_dir()?)
    }

    fn write_to(&mut self, cache_dir: &str) -> Result<String> {
        if let Some(id) = &self.id {
            validate_session_id(id)?;
        }
        Self::secure_cache_dir(cache_dir)?;

        let cache_file_name = match self.id.as_ref() {
            Some(id) => {
                // ID is already set- use the existing cache file.
                format!("{cache_dir}chisel-{id}.json")
            }
            None => {
                // Get the next session cache ID / file
                let (id, file_name) = Self::next_cached_session_in(cache_dir)?;
                // Set the session's ID
                self.id = Some(id);
                // Return the new session's cache file name
                file_name
            }
        };

        // The temporary file is private from creation, and replacement does not follow a
        // destination symlink or retain the permissions of an older session.
        let mut file = tempfile::NamedTempFile::new_in(cache_dir)?;
        #[cfg(unix)]
        file.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(&cache_file_name).map_err(|err| err.error)?;

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
        Self::secure_cache_dir(&Self::cache_dir()?)
    }

    /// Returns a list of all available cached sessions.
    pub fn get_sessions() -> Result<Vec<(String, String)>> {
        // Read the cache directory entries
        let cache_dir = Self::cache_dir()?;
        let entries = Self::cached_session_files(&cache_dir)?;

        // For each entry, get the file name and modified time
        let mut sessions = Vec::new();
        for entry in entries {
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
        Self::secure_cache_dir(cache_dir)?;
        let contents = Self::read_cached_file(Path::new(&format!("{cache_dir}chisel-{id}.json")))?;
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
        let mut entries = Self::cached_session_files(cache_dir)?.into_iter();
        let mut latest = entries.next().ok_or_else(|| eyre::eyre!("No entries found!"))?;
        for entry in entries {
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
        let last_session_contents = Self::read_cached_file(Path::new(&last_session))?;
        let mut session = Self::deserialize_cached(&last_session_contents, executor_builder)?;
        // Bind the session to the file that was loaded.
        session.id = Self::session_id_from_cache_file_name(&last_session);
        Ok(session)
    }

    /// Extracts the session id from a `.../chisel-<id>.json` cache file path.
    fn session_id_from_cache_file_name(path: &str) -> Option<String> {
        Path::new(path).file_stem()?.to_str()?.strip_prefix("chisel-").map(str::to_string)
    }

    /// Protects private source and credentials in both new and legacy cache directories.
    fn secure_cache_dir(cache_dir: &str) -> Result<()> {
        // Remove trailing separators so symlink_metadata inspects the directory entry itself.
        let path = Path::new(cache_dir).components().collect::<PathBuf>();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&path)?;
        eyre::ensure!(
            fs::symlink_metadata(&path)?.is_dir(),
            "Chisel cache must be a directory, not a symlink"
        );
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    /// Reads a regular session file after restricting legacy file permissions.
    fn read_cached_file(path: &Path) -> Result<String> {
        eyre::ensure!(
            fs::symlink_metadata(path)?.is_file(),
            "Chisel session must be a regular file"
        );
        let mut file = File::open(path)?;
        eyre::ensure!(file.metadata()?.is_file(), "Chisel session must be a regular file");
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Ok(contents)
    }

    /// Excludes temporary saves, unrelated files, and symlinks from session discovery.
    fn cached_session_files(cache_dir: &str) -> Result<Vec<fs::DirEntry>> {
        Self::secure_cache_dir(cache_dir)?;
        let mut sessions = Vec::new();
        for entry in fs::read_dir(cache_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && let Some(name) = entry.file_name().to_str()
                && let Some(id) =
                    name.strip_prefix("chisel-").and_then(|name| name.strip_suffix(".json"))
                && validate_session_id(id).is_ok()
            {
                sessions.push(entry);
            }
        }
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::{Config, SolcReq};
    use foundry_evm::core::evm::EthEvmNetwork;
    use semver::Version;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

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

    #[cfg(unix)]
    #[test]
    fn saved_sessions_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("chisel");
        std::fs::create_dir(&cache).unwrap();
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = cache.join("chisel-private.json");
        std::fs::write(&file, "old session").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut session = session_for_normalization_tests();
        session.id = Some("private".to_string());
        session.source.run_code = "uint256 privateValue = 42;".to_string();

        session.write_to(&format!("{}/", cache.display())).unwrap();

        assert_eq!(std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        let saved: ChiselSession<EthEvmNetwork> =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(saved.source.run_code, session.source.run_code);
    }

    #[cfg(unix)]
    #[test]
    fn new_cache_directories_are_private_without_changing_existing_parents() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let cache = dir.path().join(".foundry/cache/chisel");
        let mut session = session_for_normalization_tests();

        let file = session.write_to(&format!("{}/", cache.display())).unwrap();

        assert_eq!(fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777, 0o755);
        for path in [dir.path().join(".foundry"), dir.path().join(".foundry/cache"), cache] {
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert_eq!(fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn saving_replaces_symlinks_without_changing_their_targets() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("unrelated.json");
        fs::write(&target, "untouched").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        let cache = dir.path().join("chisel");
        fs::create_dir(&cache).unwrap();
        let destination = cache.join("chisel-linked.json");
        symlink(&target, &destination).unwrap();
        let mut session = session_for_normalization_tests();
        session.id = Some("linked".to_string());

        session.write_to(&format!("{}/", cache.display())).unwrap();

        assert!(fs::symlink_metadata(&destination).unwrap().is_file());
        assert_eq!(fs::metadata(destination).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(fs::metadata(target).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[cfg(unix)]
    #[test]
    fn cache_directory_symlinks_are_rejected_without_changing_targets() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("unrelated");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let cache = dir.path().join("chisel");
        symlink(&target, &cache).unwrap();
        let mut session = session_for_normalization_tests();

        let result = session.write_to(&format!("{}/", cache.display()));

        assert_eq!(
            result.unwrap_err().to_string(),
            "Chisel cache must be a directory, not a symlink"
        );
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn loading_legacy_sessions_restricts_file_and_directory_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().display());
        let path = dir.path().join("chisel-legacy.json");
        let session = session_for_normalization_tests();
        fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let loaded = ChiselSession::<EthEvmNetwork>::load_from(
            "legacy",
            &cache_dir,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap();

        assert_eq!(loaded.id.as_deref(), Some("legacy"));
        assert_eq!(fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn loading_rejects_symlinks_without_changing_their_targets() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("unrelated.json");
        fs::write(&target, "untouched").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, dir.path().join("chisel-linked.json")).unwrap();

        let result = ChiselSession::<EthEvmNetwork>::load_from(
            "linked",
            &format!("{}/", dir.path().display()),
            ExecutorBuilder::<EthEvmNetwork>::new(),
        );

        assert_eq!(result.unwrap_err().to_string(), "Chisel session must be a regular file");
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(fs::metadata(target).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[test]
    fn session_discovery_ignores_incomplete_and_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().display());
        let mut session = session_for_normalization_tests();
        session.id = Some("saved".to_string());
        let saved = session.write_to(&cache_dir).unwrap();
        fs::write(dir.path().join(".tmp-incomplete"), "{").unwrap();
        fs::write(dir.path().join("unrelated.json"), "unrelated").unwrap();
        fs::write(dir.path().join("chisel-.json"), "invalid").unwrap();
        fs::create_dir(dir.path().join("chisel-directory.json")).unwrap();
        #[cfg(unix)]
        symlink(&saved, dir.path().join("chisel-linked.json")).unwrap();

        let sessions = ChiselSession::<EthEvmNetwork>::cached_session_files(&cache_dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].path(), Path::new(&saved));
        assert_eq!(
            ChiselSession::<EthEvmNetwork>::latest_cached_session_in(&cache_dir).unwrap(),
            saved
        );
        let latest = ChiselSession::<EthEvmNetwork>::latest_from(
            &cache_dir,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        )
        .unwrap();
        assert_eq!(latest.id.as_deref(), Some("saved"));
    }

    #[test]
    fn failed_save_cleans_up_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().display());
        let destination = dir.path().join("chisel-blocked.json");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), "untouched").unwrap();
        let mut session = session_for_normalization_tests();
        session.id = Some("blocked".to_string());

        assert!(session.write_to(&cache_dir).is_err());

        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(fs::read_to_string(destination.join("sentinel")).unwrap(), "untouched");
    }

    #[test]
    fn loading_legacy_fork_sessions_discards_cached_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = format!("{}/", dir.path().display());
        let mut session = session_for_normalization_tests();
        session.source.run_code = "uint256 privateValue = 42;".into();
        session.source.config.calldata = Some(vec![0xde, 0xad, 0xbe, 0xef]);
        let mut legacy = serde_json::to_value(&session).unwrap();
        let config = &mut legacy["source"]["config"];
        config.as_object_mut().unwrap().remove("fork_url_required");
        config["foundry_config"]["eth_rpc_url"] = "https://rpc.invalid/legacy-token".into();
        config["foundry_config"]["eth_rpc_jwt"] = "legacy-jwt".into();
        config["foundry_config"]["eth_rpc_headers"] =
            serde_json::json!(["Authorization: legacy-header"]);
        config["foundry_config"]["etherscan_api_key"] = "legacy-api-key".into();
        config["foundry_config"]["etherscan"] = serde_json::json!({
            "mainnet": { "key": "legacy-explorer-key", "chain": 1 }
        });
        config["foundry_config"]["rpc_endpoints"] = serde_json::json!({
            "mainnet": "https://rpc.invalid/legacy-endpoint"
        });
        config["evm_opts"]["eth_rpc_url"] = "https://rpc.invalid/legacy-token".into();
        config["evm_opts"]["eth_rpc_jwt"] = "legacy-jwt".into();
        config["evm_opts"]["eth_rpc_headers"] = serde_json::json!(["Authorization: legacy-header"]);
        config["evm_opts"]["fork_headers"] =
            serde_json::json!(["Authorization: legacy-fork-header"]);
        let path = dir.path().join("chisel-legacy.json");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        for loaded in [
            ChiselSession::<EthEvmNetwork>::load_from(
                "legacy",
                &cache_dir,
                ExecutorBuilder::<EthEvmNetwork>::new(),
            )
            .unwrap(),
            ChiselSession::<EthEvmNetwork>::latest_from(
                &cache_dir,
                ExecutorBuilder::<EthEvmNetwork>::new(),
            )
            .unwrap(),
        ] {
            let config = &loaded.source.config;
            assert!(config.fork_url_required);
            assert_eq!(config.foundry_config.eth_rpc_url, None);
            assert_eq!(config.foundry_config.eth_rpc_jwt, None);
            assert_eq!(config.foundry_config.eth_rpc_headers, None);
            assert_eq!(config.foundry_config.etherscan_api_key, None);
            assert!(config.foundry_config.etherscan.is_empty());
            assert!(config.foundry_config.rpc_endpoints.is_empty());
            assert_eq!(config.evm_opts.fork_url, None);
            assert_eq!(config.evm_opts.rpc_jwt, None);
            assert_eq!(config.evm_opts.rpc_headers, None);
            assert_eq!(config.evm_opts.fork_headers, None);
            assert_eq!(config.calldata, session.source.config.calldata);
            assert_eq!(loaded.source.run_code, session.source.run_code);
        }
    }
}
