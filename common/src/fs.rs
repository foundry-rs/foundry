//! Contains various `std::fs` wrapper functions that also contain the target path in their errors
use crate::errors::FsPathError;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, FsPathError>;

/// Wrapper for [`File::create`].
pub fn create_file(path: impl AsRef<Path>) -> Result<fs::File> {
    let path = path.as_ref();
    File::create(path).map_err(|err| FsPathError::create_file(err, path))
}
/// Wrapper for [`std::fs::remove_file`].
pub fn remove_file(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|err| FsPathError::remove_file(err, path))
}

/// Wrapper for [`std::fs::read`].
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    fs::read(path).map_err(|err| FsPathError::read(err, path))
}

/// Wrapper for [`std::fs::read_link`].
pub fn read_link(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    fs::read_link(path).map_err(|err| FsPathError::read_link(err, path))
}

/// Wrapper for [`std::fs::read_to_string`].
pub fn read_to_string(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    fs::read_to_string(path).map_err(|err| FsPathError::read(err, path))
}

/// Reads the JSON file and deserialize it into the provided type.
pub fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    // read the file into a byte array first
    // https://github.com/serde-rs/json/issues/160
    let bytes = read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|source| FsPathError::ReadJson { source, path: path.to_path_buf() })
}

/// Writes the object as a JSON object.
pub fn write_json_file<T: Serialize>(path: &Path, obj: &T) -> Result<()> {
    let file = create_file(path)?;
    let file = std::io::BufWriter::new(file);
    serde_json::to_writer(file, obj)
        .map_err(|source| FsPathError::WriteJson { source, path: path.to_path_buf() })
}

/// Wrapper for `std::fs::write`
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    fs::write(path, contents).map_err(|err| FsPathError::write(err, path))
}

/// Wrapper for `std::fs::create_dir`
pub fn create_dir(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::create_dir(path).map_err(|err| FsPathError::create_dir(err, path))
}

/// Wrapper for `std::fs::create_dir_all`
pub fn create_dir_all(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::create_dir_all(path).map_err(|err| FsPathError::create_dir(err, path))
}

/// Wrapper for `std::fs::remove_dir`
pub fn remove_dir(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::remove_dir(path).map_err(|err| FsPathError::remove_dir(err, path))
}

/// Wrapper for `std::fs::remove_dir_all`
pub fn remove_dir_all(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::remove_dir_all(path).map_err(|err| FsPathError::remove_dir(err, path))
}

/// Wrapper for `std::fs::File::open`
pub fn open(path: impl AsRef<Path>) -> Result<fs::File> {
    let path = path.as_ref();
    fs::File::open(path).map_err(|err| FsPathError::open(err, path))
}

/// Normalize a path, removing things like `.` and `..`.
///
/// NOTE: This does not return symlinks and does not touch the filesystem at all (unlike
/// [`std::fs::canonicalize`])
///
/// ref: <https://github.com/rust-lang/cargo/blob/9ded34a558a900563b0acf3730e223c649cf859d/crates/cargo-util/src/paths.rs#L81>
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}

/// Returns all files with the given extension under the `root` dir
pub fn files_with_ext(root: impl AsRef<Path>, ext: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(walkdir::Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().map(|e| e == ext).unwrap_or_default())
        .map(|e| e.path().into())
        .collect()
}

/// Returns a list of absolute paths to all the json files under the root
pub fn json_files(root: impl AsRef<Path>) -> Vec<PathBuf> {
    files_with_ext(root, "json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        let p = Path::new("/a/../file.txt");
        let normalized = normalize_path(p);
        assert_eq!(normalized, PathBuf::from("/file.txt"));
    }
}
