use std::fs::{File, self};
use std::io::{Write, BufWriter};
use std::path::{Path, PathBuf};
use std::fmt::Display;
use std::ffi::{OsStr, OsString};
use std::collections::HashMap;

use tempfile::TempDir;
use parking_lot::Mutex;

use crate::error::Result;

// TODO: Clear environment variables before entering this? Will they mess with
// anything else?
/// A "sandboxed" environment with isolated env and file system namespace.
///
/// `Jail` creates a pseudo-sandboxed (not _actually_ sandboxed) environment for
/// testing configurations. Specifically, `Jail`:
///
///   * Synchronizes all calls to [`Jail::expect_with()`] and
///     [`Jail::try_with()`] to prevent environment variables races.
///   * Switches into a fresh temporary directory ([`Jail::directory()`]) where
///     files can be created with [`Jail::create_file()`].
///   * Keeps track of environment variables created with [`Jail::set_env()`]
///     and clears them when the `Jail` exits.
///   * Deletes the temporary directory and all of its contents when exiting.
///
/// Additionally, because `Jail` expects functions that return a [`Result`],
/// the `?` operator can be used liberally in a jail:
///
/// ```rust
/// use figment::{Figment, Jail, providers::{Format, Toml, Env}};
/// # #[derive(serde::Deserialize)]
/// # struct Config {
/// #     name: String,
/// #     authors: Vec<String>,
/// #     publish: bool
/// # }
///
/// figment::Jail::expect_with(|jail| {
///     jail.create_file("Cargo.toml", r#"
///       name = "test"
///       authors = ["bob"]
///       publish = false
///     "#)?;
///
///     jail.set_env("CARGO_NAME", "env-test");
///
///     let config: Config = Figment::new()
///         .merge(Toml::file("Cargo.toml"))
///         .merge(Env::prefixed("CARGO_"))
///         .extract()?;
///
///     Ok(())
/// });
/// ```
#[cfg_attr(nightly, doc(cfg(feature = "test")))]
pub struct Jail {
    _directory: TempDir,
    canonical_dir: PathBuf,
    saved_env_vars: HashMap<OsString, Option<OsString>>,
    saved_cwd: PathBuf,
}

/// Convert a `T: Display` to a `String`.
fn as_string<S: Display>(s: S) -> String { s.to_string() }

/// Remove any dots from the path by popping as needed.
fn dedot(path: &Path) -> PathBuf {
    use std::path::Component::*;

    let mut comps = vec![];
    for component in path.components() {
        match component {
            p@Prefix(_) => comps = vec![p],
            r@RootDir if comps.iter().all(|c| matches!(c, Prefix(_))) => comps.push(r),
            r@RootDir => comps = vec![r],
            CurDir => { },
            ParentDir if comps.iter().all(|c| matches!(c, Prefix(_) | RootDir)) => { },
            ParentDir => { comps.pop(); },
            c@Normal(_) => comps.push(c),
        }
    }

    comps.iter().map(|c| c.as_os_str()).collect()
}

static LOCK: Mutex<()> = parking_lot::const_mutex(());

impl Jail {
    /// Creates a new jail that calls `f`, passing itself to `f`.
    ///
    /// # Panics
    ///
    /// Panics if `f` panics or if [`Jail::try_with(f)`](Jail::try_with) returns
    /// an `Err`; prints the error message.
    ///
    /// # Example
    ///
    /// ```rust
    /// figment::Jail::expect_with(|jail| {
    ///     /* in the jail */
    ///
    ///     Ok(())
    /// });
    /// ```
    #[track_caller]
    pub fn expect_with<F: FnOnce(&mut Jail) -> Result<()>>(f: F) {
        if let Err(e) = Jail::try_with(f) {
            panic!("jail failed: {}", e)
        }
    }

    /// Creates a new jail that calls `f`, passing itself to `f`. Returns the
    /// result from `f` if `f` does not panic.
    ///
    /// # Panics
    ///
    /// Panics if `f` panics.
    ///
    /// # Example
    ///
    /// ```rust
    /// let result = figment::Jail::try_with(|jail| {
    ///     /* in the jail */
    ///
    ///     Ok(())
    /// });
    /// ```
    #[track_caller]
    pub fn try_with<F: FnOnce(&mut Jail) -> Result<()>>(f: F) -> Result<()> {
        let _lock = LOCK.lock();
        let directory = TempDir::new().map_err(as_string)?;
        let mut jail = Jail {
            canonical_dir: directory.path().canonicalize().map_err(as_string)?,
            _directory: directory,
            saved_cwd: std::env::current_dir().map_err(as_string)?,
            saved_env_vars: HashMap::new(),
        };

        std::env::set_current_dir(jail.directory()).map_err(as_string)?;
        f(&mut jail)
    }

    /// Returns the directory the jail has switched into. The contents of this
    /// directory will be cleared when `Jail` is dropped.
    ///
    /// # Example
    ///
    /// ```rust
    /// figment::Jail::expect_with(|jail| {
    ///     let tmp_directory = jail.directory();
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn directory(&self) -> &Path {
        &self.canonical_dir
    }

    fn safe_jailed_path(&self, path: &Path) -> Result<PathBuf> {
        let path = dedot(path);
        if path.is_absolute() && path.starts_with(self.directory()) {
            return Ok(path);
        }

        if !path.is_relative() {
            return Err("Jail: input path is outside of jail directory".to_string().into());
        }

        Ok(path)
    }

    /// Creates a file with contents `contents` within the jail's directory. The
    /// file is deleted when the jail is dropped.
    ///
    /// # Errors
    ///
    /// An error is returned if `path` is not relative or is outside of the
    /// jail's directory. I/O errors while creating the file are returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("MyConfig.json", "contents...")?;
    ///     Ok(())
    /// });
    /// ```
    pub fn create_file<P: AsRef<Path>>(&self, path: P, contents: &str) -> Result<File> {
        self.create_binary(path.as_ref(), contents.as_bytes())
    }

    /// Creates a file with binary contents `bytes` within the jail's directory.
    /// The file is deleted when the jail is dropped.
    ///
    /// # Errors
    ///
    /// An error is returned if `path` is not relative or is outside of the
    /// jail's directory. I/O errors while creating the file are returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_binary("file.bin", &[0xFF, 0x4F, 0xFF, 0x51])?;
    ///     Ok(())
    /// });
    /// ```
    pub fn create_binary<P: AsRef<Path>>(&self, path: P, bytes: &[u8]) -> Result<File> {
        let path = self.safe_jailed_path(path.as_ref())?;
        let file = File::create(path).map_err(as_string)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(bytes).map_err(as_string)?;
        Ok(writer.into_inner().map_err(as_string)?)
    }

    /// Creates a directory at `path` within the jail's directory and returns
    /// the relative path to the subdirectory in the jail. Recursively creates
    /// directories for all of its parent components if they are missing.
    ///
    /// The directory and all of its contents are deleted when the jail is
    /// dropped.
    ///
    /// # Errors
    ///
    /// An error is returned if `path` is not relative or is outside of the
    /// jail's directory. Any I/O errors encountered while creating the
    /// subdirectory are returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     let dir = jail.create_dir("subdir")?;
    ///     jail.create_file(dir.join("config.json"), "{ foo: 123 }")?;
    ///
    ///     let dir = jail.create_dir("subdir/1/2")?;
    ///     jail.create_file(dir.join("secret.toml"), "secret = 1337")?;
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn create_dir<P: AsRef<Path>>(&self, path: P) -> Result<PathBuf> {
        let path = self.safe_jailed_path(path.as_ref())?;
        fs::create_dir_all(&path).map_err(as_string)?;
        Ok(path)
    }

    /// Sets the jail's current working directory to `path` if `path` is within
    /// [`Jail::directory()`]. Otherwise returns an error.
    ///
    /// # Errors
    ///
    /// An error is returned if `path` is not relative or is outside of the
    /// jail's directory. Any I/O errors encountered while creating the
    /// subdirectory are returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     assert_eq!(std::env::current_dir().unwrap(), jail.directory());
    ///
    ///     let subdir = jail.create_dir("subdir")?;
    ///     jail.change_dir(&subdir)?;
    ///     assert_eq!(std::env::current_dir().unwrap(), jail.directory().join(subdir));
    ///
    ///     let file = jail.create_file("foo.txt", "contents")?;
    ///     assert!(!jail.directory().join("foo.txt").exists());
    ///     assert!(jail.directory().join("subdir").join("foo.txt").exists());
    ///
    ///     jail.change_dir(jail.directory())?;
    ///     assert_eq!(std::env::current_dir().unwrap(), jail.directory());
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn change_dir<P: AsRef<Path>>(&self, path: P) -> Result<PathBuf> {
        let path = self.safe_jailed_path(path.as_ref())?;
        std::env::set_current_dir(&path).map_err(as_string)?;
        Ok(path)
    }

    /// Remove all environment variables. All variables will be restored when
    /// the jail is dropped.
    ///
    /// # Example
    ///
    /// ```rust
    /// let init_count = std::env::vars_os().count();
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     // We start with _something_ in the env vars.
    ///     assert!(std::env::vars_os().count() != 0);
    ///
    ///     // Clear them all, and it's empty!
    ///     jail.clear_env();
    ///     assert!(std::env::vars_os().count() == 0);
    ///
    ///     // Set a value.
    ///     jail.set_env("FIGMENT_SPECIAL_JAIL_VALUE", "value");
    ///     assert!(std::env::vars_os().count() == 1);
    ///
    ///     // If we clear again, the new values are removed.
    ///     jail.clear_env();
    ///     assert!(std::env::vars_os().count() == 0);
    ///
    ///     Ok(())
    /// });
    ///
    /// // After the drop, we have our original env vars.
    /// assert!(std::env::vars_os().count() == init_count);
    /// assert!(std::env::var("FIGMENT_SPECIAL_JAIL_VALUE").is_err());
    /// ```
    pub fn clear_env(&mut self) {
        for (key, val) in std::env::vars_os() {
            std::env::remove_var(&key);
            if !self.saved_env_vars.contains_key(&key) {
                self.saved_env_vars.insert(key, Some(val));
            }
        }
    }

    /// Set the environment variable `k` to value `v`. The variable will be
    /// removed when the jail is dropped.
    ///
    /// # Example
    ///
    /// ```rust
    /// const VAR_NAME: &str = "my-very-special-figment-var";
    ///
    /// assert!(std::env::var(VAR_NAME).is_err());
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.set_env(VAR_NAME, "value");
    ///     assert!(std::env::var(VAR_NAME).is_ok());
    ///     Ok(())
    /// });
    ///
    /// assert!(std::env::var(VAR_NAME).is_err());
    /// ```
    pub fn set_env<K: AsRef<str>, V: Display>(&mut self, k: K, v: V) {
        let key = k.as_ref();
        if !self.saved_env_vars.contains_key(OsStr::new(key)) {
            self.saved_env_vars.insert(key.into(), std::env::var_os(key));
        }

        std::env::set_var(key, v.to_string());
    }
}

impl Drop for Jail {
    fn drop(&mut self) {
        for (key, value) in self.saved_env_vars.iter() {
            match value {
                Some(val) => std::env::set_var(key, val),
                None => std::env::remove_var(key)
            }
        }

        let _ = std::env::set_current_dir(&self.saved_cwd);
    }
}
