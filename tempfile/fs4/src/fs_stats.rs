/// `FsStats` contains some common stats about a file system.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FsStats {
    pub(crate) free_space: u64,
    pub(crate) available_space: u64,
    pub(crate) total_space: u64,
    pub(crate) allocation_granularity: u64,
}

impl FsStats {
    /// Returns the number of free bytes in the file system containing the provided
    /// path.
    pub fn free_space(&self) -> u64 {
        self.free_space
    }

    /// Returns the available space in bytes to non-priveleged users in the file
    /// system containing the provided path.
    pub fn available_space(&self) -> u64 {
        self.available_space
    }

    /// Returns the total space in bytes in the file system containing the provided
    /// path.
    pub fn total_space(&self) -> u64 {
        self.total_space
    }

    /// Returns the filesystem's disk space allocation granularity in bytes.
    /// The provided path may be for any file in the filesystem.
    ///
    /// On Posix, this is equivalent to the filesystem's block size.
    /// On Windows, this is equivalent to the filesystem's cluster size.
    pub fn allocation_granularity(&self) -> u64 {
        self.allocation_granularity
    }
}
