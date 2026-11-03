//! Encrypted store for named secrets, such as RPC URLs and explorer API keys.
//!
//! Each secret is a Web3 Secret Storage file, the format of `cast wallet` keystores, named after
//! the secret in `~/.foundry/secrets` (or `$FOUNDRY_SECRETS_DIR`). All secrets share one password.
//! `${NAME}` placeholders in the config resolve from the store when `NAME` is not set in the
//! environment, see [`crate::resolve::set_secret_resolver`].

use crate::Config;
use eth_keystore::KeystoreError;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

/// Environment variable that overrides the secret store directory.
pub const SECRETS_DIR_ENV: &str = "FOUNDRY_SECRETS_DIR";

/// Errors of the [`SecretStore`].
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    #[error(
        "invalid secret name `{0}`: use letters, digits and `_`, and do not start with a digit"
    )]
    InvalidName(String),
    #[error("secret `{0}` not found")]
    NotFound(String),
    #[error("wrong password for the secret store")]
    WrongPassword,
    #[error("secret `{0}` is not valid UTF-8")]
    NotUtf8(String),
    #[error("could not determine the home directory for the secret store")]
    NoHomeDir,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("secret store: {0}")]
    Keystore(String),
}

impl From<KeystoreError> for SecretStoreError {
    fn from(err: KeystoreError) -> Self {
        match err {
            KeystoreError::MacMismatch => Self::WrongPassword,
            err => Self::Keystore(err.to_string()),
        }
    }
}

/// A directory of encrypted secrets that share one password.
#[derive(Clone, Debug)]
pub struct SecretStore {
    dir: PathBuf,
}

impl SecretStore {
    /// Returns the store in `dir`, which does not need to exist yet.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Returns the store in `$FOUNDRY_SECRETS_DIR`, or else `~/.foundry/secrets`.
    pub fn open_default() -> Result<Self, SecretStoreError> {
        let dir = std::env::var_os(SECRETS_DIR_ENV)
            .map(PathBuf::from)
            .or_else(|| Config::foundry_dir().map(|dir| dir.join("secrets")))
            .ok_or(SecretStoreError::NoHomeDir)?;
        Ok(Self::new(dir))
    }

    /// The directory that holds the secrets.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the names of the stored secrets, sorted.
    pub fn names(&self) -> Result<Vec<String>, SecretStoreError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry?;
            if let Ok(name) = entry.file_name().into_string()
                && is_valid_name(&name)
                && entry.path().metadata()?.is_file()
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    /// Returns `true` if a secret called `name` is stored.
    pub fn contains(&self, name: &str) -> Result<bool, SecretStoreError> {
        if !is_valid_name(name) {
            return Ok(false);
        }
        match fs::metadata(self.path(name)) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Decrypts the secret called `name`.
    pub fn get(&self, name: &str, password: &str) -> Result<String, SecretStoreError> {
        validate_name(name)?;
        let path = self.path(name);
        if !self.contains(name)? {
            return Err(SecretStoreError::NotFound(name.to_string()));
        }
        let value = eth_keystore::decrypt_key(&path, password)?;
        String::from_utf8(value).map_err(|_| SecretStoreError::NotUtf8(name.to_string()))
    }

    /// Encrypts `value` as the secret called `name`, replacing any previous value.
    ///
    /// The password must match the one of the secrets already stored.
    pub fn set(&self, name: &str, value: &str, password: &str) -> Result<(), SecretStoreError> {
        validate_name(name)?;
        let _lock = self.lock()?;
        self.check_password(password)?;
        let temporary = tempfile::tempdir_in(&self.dir)?;
        eth_keystore::encrypt_key(
            temporary.path(),
            &mut rand_08::thread_rng(),
            value.as_bytes(),
            password,
            Some(name),
        )?;
        let encrypted = temporary.path().join(name);
        restrict_permissions(&encrypted)?;
        File::options().write(true).open(&encrypted)?.sync_all()?;
        fs::rename(encrypted, self.path(name))?;
        #[cfg(unix)]
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    /// Deletes the secret called `name`.
    pub fn remove(&self, name: &str) -> Result<(), SecretStoreError> {
        validate_name(name)?;
        let _lock = self.lock()?;
        let path = self.path(name);
        if !self.contains(name)? {
            return Err(SecretStoreError::NotFound(name.to_string()));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    /// Checks `password` against a stored secret. An empty store accepts any password.
    pub fn check_password(&self, password: &str) -> Result<(), SecretStoreError> {
        match self.names()?.first() {
            Some(name) => self.get(name, password).map(drop),
            None => Ok(()),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn lock(&self) -> Result<File, SecretStoreError> {
        create_private_dir(&self.dir)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(self.dir.join(".lock"))?;
        file.lock()?;
        Ok(file)
    }
}

/// Returns `true` if `name` can name a secret: an environment variable name.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn validate_name(name: &str) -> Result<(), SecretStoreError> {
    if is_valid_name(name) { Ok(()) } else { Err(SecretStoreError::InvalidName(name.to_string())) }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(dir)?;
    #[cfg(unix)]
    {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn set_get_list_remove() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets"));
        assert!(store.names().unwrap().is_empty());

        // Not 32 bytes long, unlike a private key.
        let url = "https://eth-sepolia.g.alchemy.com/v2/some-api-key";
        store.set("SEPOLIA_RPC_URL", url, "pw").unwrap();
        store.set("ETHERSCAN_KEY", "ABC123", "pw").unwrap();
        assert_eq!(store.names().unwrap(), ["ETHERSCAN_KEY", "SEPOLIA_RPC_URL"]);
        assert!(store.contains("SEPOLIA_RPC_URL").unwrap());
        assert_eq!(store.get("SEPOLIA_RPC_URL", "pw").unwrap(), url);

        // The file holds no plaintext.
        let raw = fs::read_to_string(store.dir().join("SEPOLIA_RPC_URL")).unwrap();
        assert!(!raw.contains("some-api-key"));

        store.set("ETHERSCAN_KEY", "XYZ", "pw").unwrap();
        assert_eq!(store.get("ETHERSCAN_KEY", "pw").unwrap(), "XYZ");

        store.remove("ETHERSCAN_KEY").unwrap();
        assert_eq!(store.names().unwrap(), ["SEPOLIA_RPC_URL"]);
        assert!(matches!(store.remove("ETHERSCAN_KEY"), Err(SecretStoreError::NotFound(_))));
    }

    #[test]
    fn one_password_for_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path());
        store.set("A", "1", "pw").unwrap();
        assert!(matches!(store.get("A", "other"), Err(SecretStoreError::WrongPassword)));
        assert!(matches!(store.set("B", "2", "other"), Err(SecretStoreError::WrongPassword)));
        assert!(!store.contains("B").unwrap());
    }

    #[test]
    fn rejects_names_that_are_not_variable_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path());
        for name in ["", "1ABC", "../x", "A-B", "a/b", "A.B"] {
            assert!(
                matches!(store.set(name, "v", "pw"), Err(SecretStoreError::InvalidName(_))),
                "{name}"
            );
            assert!(!store.contains(name).unwrap(), "{name}");
        }
        assert!(is_valid_name("_PRIVATE_1"));
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets"));
        store.set("A", "1", "pw").unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.dir()), 0o700);
        assert_eq!(mode(&store.dir().join("A")), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn replacement_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets"));
        store.set("A", "original", "pw").unwrap();
        let original = dir.path().join("backup");
        fs::rename(store.path("A"), &original).unwrap();
        let before = fs::read(&original).unwrap();
        std::os::unix::fs::symlink(&original, store.path("A")).unwrap();
        store.set("A", "replacement", "pw").unwrap();
        assert_eq!(store.get("A", "pw").unwrap(), "replacement");
        assert_eq!(fs::read(original).unwrap(), before);
        assert!(!store.path("A").is_symlink());
    }

    #[test]
    fn concurrent_first_writers_share_one_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path());
        let barrier = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                store.set("A", "first", "password-a")
            });
            let second = scope.spawn(|| {
                barrier.wait();
                store.set("B", "second", "password-b")
            });
            let results = [first.join().unwrap(), second.join().unwrap()];
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            assert!(
                results.iter().any(|result| matches!(result, Err(SecretStoreError::WrongPassword)))
            );
        });
        assert_eq!(store.names().unwrap().len(), 1);
    }
}
