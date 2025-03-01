use std::{io, io::ErrorKind::AlreadyExists, path::Path};

/// Create a new symlink at `link` which points to `original`.
///
/// Note that `original` doesn't have to exist.
#[cfg(not(windows))]
pub fn create(original: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

/// Remove a symlink.
///
/// Note that on only on windows this is special.
#[cfg(not(windows))]
pub fn remove(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

// TODO: use the `symlink` crate once it can delete directory symlinks
/// Remove a symlink.
#[cfg(windows)]
pub fn remove(path: &Path) -> io::Result<()> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.is_file() {
            std::fs::remove_file(path) // this removes the link itself
        } else {
            std::fs::remove_dir(path) // however, this sees the destination directory, which isn't the right thing actually
        }
    } else {
        std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path))
    }
}

/// Create a new symlink at `link` which points to `original`.
///
/// Note that if a symlink target (the `original`) isn't present on disk, it's assumed to be a
/// file, creating a dangling file symlink. This is similar to a dangling symlink on Unix,
/// which doesn't have to care about the target type though.
#[cfg(windows)]
pub fn create(original: &Path, link: &Path) -> io::Result<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    // TODO: figure out if links to links count as files or whatever they point at
    let orig_abs = link.parent().expect("dir for link").join(original);
    if orig_abs.is_dir() {
        symlink_dir(original, link)
    } else {
        symlink_file(original, link)
    }
}

/// Return true if `err` indicates that a file collision happened, i.e. a symlink couldn't be created as the `link`
/// already exists as filesystem object.
#[cfg(not(windows))]
pub fn is_collision_error(err: &std::io::Error) -> bool {
    // TODO: use ::IsDirectory as well when stabilized instead of raw_os_error(), and ::FileSystemLoop respectively
    err.kind() == AlreadyExists
            || err.raw_os_error() == Some(21)
            || err.raw_os_error() == Some(62) // no-follow on symlnk on mac-os
            || err.raw_os_error() == Some(40) // no-follow on symlnk on ubuntu
}

/// Return true if `err` indicates that a file collision happened, i.e. a symlink couldn't be created as the `link`
/// already exists as filesystem object.
#[cfg(windows)]
pub fn is_collision_error(err: &std::io::Error) -> bool {
    err.kind() == AlreadyExists || err.kind() == std::io::ErrorKind::PermissionDenied
}
