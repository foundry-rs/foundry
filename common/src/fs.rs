//! Contains various `std::fs` wrapper functions that also contain the target path in their errors
use crate::errors::FsPathError;
use serde::{de::DeserializeOwned, Serialize};
use std::{fs, path::Path};

type Result<T> = std::result::Result<T, FsPathError>;

/// Wrapper for `std::fs::File::create`
pub fn create_file(path: impl AsRef<Path>) -> Result<fs::File> {
    let path = path.as_ref();
    fs::File::create(path).map_err(|err| FsPathError::create_file(err, path))
}
/// Wrapper for `std::fs::remove_file`
pub fn remove_file(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|err| FsPathError::remove_file(err, path))
}

/// Wrapper for `std::fs::read`
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    fs::read(path).map_err(|err| FsPathError::read(err, path))
}

/// Wrapper for `std::fs::read_to_string`
pub fn read_to_string(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    fs::read_to_string(path).map_err(|err| FsPathError::read(err, path))
}

/// Reads the json file and deserialize it into the provided type
pub fn read_json_file<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let file = open(path)?;
    let file = std::io::BufReader::new(file);
    serde_json::from_reader(file)
        .map_err(|source| FsPathError::ReadJson { source, path: path.to_path_buf() })
}

/// Writes the object as a json object
pub fn write_json_file<T: Serialize>(path: impl AsRef<Path>, obj: &T) -> Result<()> {
    let path = path.as_ref();
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
