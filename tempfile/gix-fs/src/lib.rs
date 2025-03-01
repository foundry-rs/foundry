//! A crate with file-system specific utilities.
#![deny(rust_2018_idioms, missing_docs)]
#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Common knowledge about the worktree that is needed across most interactions with the work tree
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(PartialEq, Eq, Debug, Hash, Ord, PartialOrd, Clone, Copy)]
pub struct Capabilities {
    /// If `true`, the filesystem will consider the precomposed umlaut `ä` similar to its decomposed form `"a\u{308}"` and consider them the same.
    /// If `false`, the filesystem will only see bytes which means that the above example could live side-by-side.
    ///
    /// Even though a filesystem that treats both forms the same will still reproduce the exact same byte sequence during traversal for instance,
    /// this might also mean that we see paths in their decomposed form (this happens when creating directory `ä` in MacOS Finder for example).
    ///
    /// If Git would store such decomposed paths in the repository, which only sees bytes, on linux this might mean the path will look strange
    /// at best, which is why it prefers to store precomposed unicode on systems where it matters, like MacOS and Windows.
    ///
    /// For best compatibility, and with this value being `true`, we will turn decomposed paths and input like command-line arguments into their
    /// precomposed forms, so no decomposed byte sequences should end up in storage.
    pub precompose_unicode: bool,
    /// If true, the filesystem ignores the case of input, which makes `A` the same file as `a`.
    /// This is also called case-folding.
    pub ignore_case: bool,
    /// If true, we assume the executable bit is honored as part of the files mode. If false, we assume the file system
    /// ignores the executable bit, hence it will be reported as 'off' even though we just tried to set it to be on.
    pub executable_bit: bool,
    /// If true, the file system supports symbolic links and we should try to create them. Otherwise symbolic links will be checked
    /// out as files which contain the link as text.
    pub symlink: bool,
}
mod capabilities;

mod snapshot;
pub use snapshot::{FileSnapshot, SharedFileSnapshot, SharedFileSnapshotMut};

///
pub mod symlink;

///
pub mod read_dir;
pub use read_dir::function::read_dir;

///
pub mod dir;

/// Like [`std::env::current_dir()`], but it will `precompose_unicode` if that value is true, if the current directory
/// is valid unicode and if there are decomposed unicode codepoints.
///
/// Thus, it will turn `"a\u{308}"` into `ä` if `true`.
/// Keeping it `false` will not alter the output.
///
/// Note that `precompose_unicode` most be set using the `core.precomposeUnicode` git configuration.
pub fn current_dir(precompose_unicode: bool) -> std::io::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    Ok(if precompose_unicode {
        gix_utils::str::precompose_path(cwd.into()).into_owned()
    } else {
        cwd
    })
}

/// A stack of path components with the delegation of side-effects as the currently set path changes, component by component.
#[derive(Clone)]
pub struct Stack {
    /// The prefix/root for all paths we handle.
    root: PathBuf,
    /// the most recent known cached that we know is valid.
    current: PathBuf,
    /// The relative portion of `valid` that was added previously.
    current_relative: PathBuf,
    /// The amount of path components of 'current' beyond the roots components.
    valid_components: usize,
    /// If set, we assume the `current` element is a directory to affect calls to `(push|pop)_directory()`.
    current_is_directory: bool,
}

#[cfg(unix)]
/// Returns whether a a file has the executable permission set.
pub fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (metadata.mode() & 0o100) != 0
}

/// Classifiers for IO-errors.
pub mod io_err {
    use std::io::ErrorKind;

    /// Return `true` if `err` indicates that the entry doesn't exist on disk. `raw` is used as well
    /// for additional checks while the variants are outside the MSRV.
    pub fn is_not_found(err: ErrorKind, raw_err: Option<i32>) -> bool {
        // TODO: use variant once MSRV is 1.83
        err == ErrorKind::NotFound || raw_err == Some(20)
    }
}

#[cfg(not(unix))]
/// Returns whether a a file has the executable permission set.
pub fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

///
pub mod stack;
