use std::{collections::HashMap, fmt, fs, hash, ops, path::PathBuf};

use tracing::{trace, warn};

/// Disk-serializable HashMap
///
/// How to read and save the contents of the type will be up to the user,
/// See [DiskMap::save()], [DiskMap::reload()]
#[derive(Debug)]
pub(crate) struct DiskMap<K: hash::Hash + Eq, V> {
    /// Where this file is stored
    path: PathBuf,
    /// Data it holds
    cache: HashMap<K, V>,
    /// Whether to write to disk
    transient: bool,
}

impl<K: hash::Hash + Eq, V> ops::Deref for DiskMap<K, V> {
    type Target = HashMap<K, V>;
    fn deref(&self) -> &Self::Target {
        &self.cache
    }
}

impl<K: hash::Hash + Eq, V> ops::DerefMut for DiskMap<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cache
    }
}

impl<K: hash::Hash + Eq, V> DiskMap<K, V> {
    /// Creates a new, empty `DiskMap`
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        trace!("path={:?}", path);
        DiskMap { path, cache: HashMap::new(), transient: false }
    }

    /// Reads the contents of the diskmap file and returns the read cache
    pub fn read<F, E>(path: impl Into<PathBuf>, read: F) -> Self
    where
        F: Fn(fs::File) -> Result<HashMap<K, V>, E>,
        E: fmt::Display,
    {
        let mut map = Self::new(path);
        trace!("reading diskmap path={:?}", map.path);
        map.reload(read);
        map
    }

    /// The path where the diskmap is (will be) stored
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Consumes the type and returns the underlying map
    pub fn into_inner(self) -> HashMap<K, V> {
        self.cache
    }

    /// Sets the `transient` to the given bool value
    pub fn set_transient(mut self, transient: bool) -> Self {
        self.transient = transient;
        self
    }

    /// Marks the cache as transient.
    ///
    /// A transient cache will never be written to disk
    pub fn transient(self) -> Self {
        self.set_transient(true)
    }

    /// Reloads the cache.
    ///
    /// This will replace the current cache with the contents of the file the diskmap points to,
    /// overwriting all changes
    fn reload<F, E>(&mut self, read: F)
    where
        F: Fn(fs::File) -> Result<HashMap<K, V>, E>,
        E: fmt::Display,
    {
        if self.transient {
            return
        }
        trace!("reverting diskmap {:?}", self.path);
        let _ = fs::File::open(self.path.clone())
            .map_err(|e| trace!("Failed to open disk map: {}", e))
            .and_then(|f| read(f).map_err(|e| warn!("Failed to read disk map: {}", e)))
            .and_then(|m| {
                self.cache = m;
                Ok(())
            });
    }

    /// Saves the diskmap to the file it points to
    ///
    /// The closure is expected to do the actual writing
    pub fn save<F, E>(&self, write: F)
    where
        F: Fn(&HashMap<K, V>, &mut fs::File) -> Result<(), E>,
        E: fmt::Display,
    {
        if self.transient {
            return
        }
        trace!("saving diskmap {:?}", self.path);
        let _ = fs::File::create(&self.path)
            .map_err(|e| warn!("Failed to open disk map for writing: {}", e))
            .and_then(|mut f| {
                write(&self.cache, &mut f).map_err(|e| warn!("Failed to write to disk map: {}", e))
            });
    }
}
