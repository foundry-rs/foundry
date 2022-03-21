use std::{collections::BTreeMap, fmt, fs, hash, ops, path::PathBuf};

use tracing::{trace, warn};

/// Disk-serializable BTreeMap
///
/// How to read and save the contents of the type will be up to the user,
/// See [DiskMap::save()], [DiskMap::reload()]
#[derive(Debug)]
pub(crate) struct DiskMap<K: hash::Hash + Eq, V> {
    /// Where this file is stored
    file_path: PathBuf,
    /// Data it holds
    cache: BTreeMap<K, V>,
    /// Whether to write to disk
    transient: bool,
}

impl<K: hash::Hash + Eq, V> ops::Deref for DiskMap<K, V> {
    type Target = BTreeMap<K, V>;
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
    pub fn new(file_path: impl Into<PathBuf>) -> Self {
        Self::with_data(file_path, Default::default())
    }

    /// Creates a new, `DiskMap`
    pub fn with_data(file_path: impl Into<PathBuf>, cache: BTreeMap<K, V>) -> Self {
        let path = file_path.into();
        DiskMap { file_path: path, cache, transient: false }
    }

    /// Reads the contents of the diskmap file and returns the read cache
    pub fn read<F, E>(path: impl Into<PathBuf>, read: F) -> Self
    where
        F: Fn(fs::File) -> Result<BTreeMap<K, V>, E>,
        E: fmt::Display,
    {
        let mut map = Self::new(path);
        trace!("reading diskmap path={:?}", map.file_path);
        map.reload(read);
        map
    }

    /// The path where the diskmap is (will be) stored
    pub fn path(&self) -> &PathBuf {
        &self.file_path
    }

    /// Consumes the type and returns the underlying map
    pub fn into_inner(self) -> BTreeMap<K, V> {
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
        F: Fn(fs::File) -> Result<BTreeMap<K, V>, E>,
        E: fmt::Display,
    {
        if self.transient {
            return
        }
        trace!("reloading diskmap {:?}", self.file_path);
        let _ = fs::File::open(self.file_path.clone())
            .map_err(|e| trace!("Failed to open disk map: {}", e))
            .and_then(|f| read(f).map_err(|e| warn!("Failed to read disk map: {}", e)))
            .map(|m| {
                self.cache = m;
            });
    }

    /// Saves the diskmap to the file it points to
    ///
    /// The closure is expected to do the actual writing
    pub fn save<F, E>(&self, write: F)
    where
        F: Fn(&BTreeMap<K, V>, &mut fs::File) -> Result<(), E>,
        E: fmt::Display,
    {
        if self.transient {
            return
        }
        trace!("saving diskmap {:?}", self.file_path);
        if let Some(parent) = self.file_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::File::create(&self.file_path)
            .map_err(|e| warn!("Failed to open disk map for writing: {}", e))
            .and_then(|mut f| {
                write(&self.cache, &mut f).map_err(|e| warn!("Failed to write to disk map: {}", e))
            });
    }
}
