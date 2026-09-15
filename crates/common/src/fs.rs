//! Contains various `std::fs` wrapper functions that also contain the target path in their errors.

use crate::errors::FsPathError;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// The [`fs`](self) result type.
pub type Result<T> = std::result::Result<T, FsPathError>;

/// Controls whether atomic publication may replace an existing artifact.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishMode {
    /// Atomically replace the destination if it exists.
    Replace,
    /// Publish only if the destination does not exist.
    CreateNew,
}

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
    let s = read_to_string(path)?;
    serde_json::from_str(&s).map_err(|source| FsPathError::ReadJson { source, path: path.into() })
}

/// Reads and decodes the json gzip file, then deserialize it into the provided type.
pub fn read_json_gzip_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = open(path)?;
    let reader = BufReader::new(file);
    let decoder = GzDecoder::new(reader);
    serde_json::from_reader(decoder)
        .map_err(|source| FsPathError::ReadJson { source, path: path.into() })
}

/// Reads the entire contents of a locked shared file into a string.
pub fn locked_read_to_string(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let contents = locked_read(path)?;
    String::from_utf8(contents).map_err(|err| FsPathError::read(std::io::Error::other(err), path))
}

/// Reads the entire contents of a locked shared file into a bytes vector.
pub fn locked_read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let mut file =
        fs::OpenOptions::new().read(true).open(path).map_err(|err| FsPathError::open(err, path))?;
    file.lock_shared().map_err(|err| FsPathError::lock(err, path))?;
    let contents = read_inner(path, &mut file)?;
    file.unlock().map_err(|err| FsPathError::unlock(err, path))?;
    Ok(contents)
}

fn read_inner(path: &Path, file: &mut File) -> Result<Vec<u8>> {
    let file_len = file.metadata().map_err(|err| FsPathError::open(err, path))?.len() as usize;
    let mut buffer = Vec::with_capacity(file_len);
    file.read_to_end(&mut buffer).map_err(|err| FsPathError::read(err, path))?;
    Ok(buffer)
}

/// Writes the object as a JSON object.
pub fn write_json_file<T: Serialize>(path: &Path, obj: &T) -> Result<()> {
    let file = create_file(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, obj)
        .map_err(|source| FsPathError::WriteJson { source, path: path.into() })?;
    writer.flush().map_err(|e| FsPathError::write(e, path))
}

/// Atomically publishes an object as compact JSON.
///
/// The destination's parent directory must already exist. Concurrent replacement writers are
/// last-publication-wins, and readers observe either the complete old file or the complete new
/// file when all writers use atomic publication. This does not provide read-modify-write locking
/// or power-loss durability. Publication replaces the destination directory entry rather than
/// preserving its inode or following an existing symlink; newly published files use the temporary
/// file's owner-only permissions on Unix.
pub fn write_json_file_atomic<T: Serialize + ?Sized>(
    path: &Path,
    obj: &T,
    mode: PublishMode,
) -> Result<()> {
    write_atomic_with(path, mode, |file| {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, obj)
            .map_err(|source| FsPathError::WriteJson { source, path: path.into() })?;
        writer.flush().map_err(|err| FsPathError::write(err, path))
    })
}

/// Writes the object as a pretty JSON object.
pub fn write_pretty_json_file<T: Serialize>(path: &Path, obj: &T) -> Result<()> {
    write_pretty_json(path, obj, create_file(path)?)
}

/// Atomically publishes an object as pretty JSON.
///
/// See [`write_json_file_atomic`] for publication guarantees.
pub fn write_pretty_json_file_atomic<T: Serialize + ?Sized>(
    path: &Path,
    obj: &T,
    mode: PublishMode,
) -> Result<()> {
    write_atomic_with(path, mode, |file| {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, obj)
            .map_err(|source| FsPathError::WriteJson { source, path: path.into() })?;
        writer.flush().map_err(|err| FsPathError::write(err, path))
    })
}

/// Writes an object as pretty JSON with owner-only permissions on Unix.
pub fn write_sensitive_json_file<T: Serialize>(path: &Path, obj: &T) -> Result<()> {
    let mut options = File::options();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);

    let file = options.open(path).map_err(|err| FsPathError::create_file(err, path))?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|err| FsPathError::write(err, path))?;

    write_pretty_json(path, obj, file)
}

fn write_pretty_json<T: Serialize>(path: &Path, obj: &T, file: File) -> Result<()> {
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, obj)
        .map_err(|source| FsPathError::WriteJson { source, path: path.into() })?;
    writer.flush().map_err(|e| FsPathError::write(e, path))
}

/// Writes the object as a gzip compressed file.
pub fn write_json_gzip_file<T: Serialize>(path: &Path, obj: &T) -> Result<()> {
    let file = create_file(path)?;
    let writer = BufWriter::new(file);
    let mut encoder = GzEncoder::new(writer, Compression::default());
    serde_json::to_writer(&mut encoder, obj)
        .map_err(|source| FsPathError::WriteJson { source, path: path.into() })?;
    // Ensure we surface any I/O errors on final gzip write and buffer flush.
    let mut inner_writer = encoder.finish().map_err(|e| FsPathError::write(e, path))?;
    inner_writer.flush().map_err(|e| FsPathError::write(e, path))?;
    Ok(())
}

/// Atomically publishes an object as gzip-compressed JSON.
///
/// See [`write_json_file_atomic`] for publication guarantees.
pub fn write_json_gzip_file_atomic<T: Serialize + ?Sized>(
    path: &Path,
    obj: &T,
    mode: PublishMode,
) -> Result<()> {
    write_atomic_with(path, mode, |file| {
        let writer = BufWriter::new(file);
        let mut encoder = GzEncoder::new(writer, Compression::default());
        serde_json::to_writer(&mut encoder, obj)
            .map_err(|source| FsPathError::WriteJson { source, path: path.into() })?;
        let mut writer = encoder.finish().map_err(|err| FsPathError::write(err, path))?;
        writer.flush().map_err(|err| FsPathError::write(err, path))
    })
}

/// Wrapper for `std::fs::write`
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    fs::write(path, contents).map_err(|err| FsPathError::write(err, path))
}

/// Atomically publishes raw bytes.
///
/// See [`write_json_file_atomic`] for publication guarantees.
pub fn write_atomic(path: &Path, contents: &[u8], mode: PublishMode) -> Result<()> {
    write_atomic_with(path, mode, |file| {
        file.write_all(contents).map_err(|err| FsPathError::write(err, path))?;
        file.flush().map_err(|err| FsPathError::write(err, path))
    })
}

fn write_atomic_with(
    path: &Path,
    mode: PublishMode,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp =
        NamedTempFile::new_in(parent).map_err(|err| FsPathError::create_file(err, path))?;
    write(temp.as_file_mut())?;

    let published = match mode {
        PublishMode::Replace => temp.persist(path),
        PublishMode::CreateNew => temp.persist_noclobber(path),
    };
    published.map(|_| ()).map_err(|err| FsPathError::write(err.error, path))
}

/// Writes all content in an exclusive locked file.
pub fn locked_write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|err| FsPathError::open(err, path))?;
    file.lock().map_err(|err| FsPathError::lock(err, path))?;
    file.write_all(contents.as_ref()).map_err(|err| FsPathError::write(err, path))?;
    file.unlock().map_err(|err| FsPathError::unlock(err, path))
}

/// Writes a line in an exclusive locked file.
pub fn locked_write_line(path: impl AsRef<Path>, line: &str) -> Result<()> {
    let path = path.as_ref();
    if cfg!(windows) {
        return locked_write_line_windows(path, line);
    }

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(|err| FsPathError::open(err, path))?;

    file.lock().map_err(|err| FsPathError::lock(err, path))?;
    writeln!(file, "{line}").map_err(|err| FsPathError::write(err, path))?;
    file.unlock().map_err(|err| FsPathError::unlock(err, path))
}

// Locking fails on Windows if the file is opened in append mode.
fn locked_write_line_windows(path: &Path, line: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(false)
        .create(true)
        .open(path)
        .map_err(|err| FsPathError::open(err, path))?;
    file.lock().map_err(|err| FsPathError::lock(err, path))?;

    file.seek(SeekFrom::End(0)).map_err(|err| FsPathError::write(err, path))?;
    writeln!(file, "{line}").map_err(|err| FsPathError::write(err, path))?;

    file.unlock().map_err(|err| FsPathError::unlock(err, path))
}

/// Wrapper for `std::fs::copy`
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::copy(from, to).map_err(|err| FsPathError::copy(err, from, to))
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
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().copied() {
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

/// Returns an iterator over all files with the given extension under the `root` dir.
pub fn files_with_ext<'a>(root: &Path, ext: &'a str) -> impl Iterator<Item = PathBuf> + 'a {
    walkdir::WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(walkdir::Result::ok)
        .filter(|e| e.file_type().is_file() && e.path().extension() == Some(ext.as_ref()))
        .map(walkdir::DirEntry::into_path)
}

/// Returns an iterator over all JSON files under the `root` dir.
pub fn json_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    files_with_ext(root, "json")
}

/// Canonicalize a path, returning an error if the path does not exist.
///
/// Mainly useful to apply canonicalization to paths obtained from project files but still error
/// properly instead of flattening the errors.
pub fn canonicalize_path(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::SerializeSeq;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element("partial")?;
            Err(serde::ser::Error::custom("injected serialization failure"))
        }
    }

    #[test]
    fn atomic_writers_replace_complete_files() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw");
        let compact = dir.path().join("compact.json");
        let pretty = dir.path().join("pretty.json");
        let gzip = dir.path().join("gzip.json.gz");

        write_atomic(&raw, b"old", PublishMode::Replace).unwrap();
        write_atomic(&raw, b"new", PublishMode::Replace).unwrap();
        write_json_file_atomic(&compact, &vec![1, 2], PublishMode::Replace).unwrap();
        write_pretty_json_file_atomic(&pretty, &vec![3, 4], PublishMode::Replace).unwrap();
        write_json_gzip_file_atomic(&gzip, &vec![5, 6], PublishMode::Replace).unwrap();

        assert_eq!(read(&raw).unwrap(), b"new");
        assert_eq!(read_json_file::<Vec<u8>>(&compact).unwrap(), [1, 2]);
        assert!(read_to_string(&pretty).unwrap().contains("\n  3,"));
        assert_eq!(read_json_gzip_file::<Vec<u8>>(&gzip).unwrap(), [5, 6]);
    }

    #[test]
    fn atomic_serialization_failure_preserves_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact.json");
        std::fs::write(&path, "previous").unwrap();

        assert!(write_json_file_atomic(&path, &FailingSerialize, PublishMode::Replace).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_create_new_does_not_replace_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        write_atomic(&path, b"winner", PublishMode::CreateNew).unwrap();

        let error = write_atomic(&path, b"loser", PublishMode::CreateNew).unwrap_err();

        assert_eq!(std::fs::read(&path).unwrap(), b"winner");
        assert_eq!(std::io::Error::from(error).kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn concurrent_readers_only_observe_complete_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("artifact.json"));
        let writing = Arc::new(AtomicBool::new(true));
        write_json_file_atomic(&path, &vec![0_u8; 4096], PublishMode::Replace).unwrap();

        let writer_path = Arc::clone(&path);
        let writer_running = Arc::clone(&writing);
        let writer = std::thread::spawn(move || {
            for value in 1..=100_u8 {
                write_json_file_atomic(&writer_path, &vec![value; 4096], PublishMode::Replace)
                    .unwrap();
            }
            writer_running.store(false, Ordering::Release);
        });

        while writing.load(Ordering::Acquire) {
            let value = read_json_file::<Vec<u8>>(&path).unwrap();
            assert_eq!(value.len(), 4096);
            assert!(value.iter().all(|byte| *byte == value[0]));
        }
        writer.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_write_sensitive_json_file_permissions() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["new", "existing"] {
            let path = dir.path().join(name);
            if name == "existing" {
                fs::write(&path, []).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            }

            write_sensitive_json_file(&path, &()).unwrap();
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_normalize_path() {
        let p = Path::new("/a/../file.txt");
        let normalized = normalize_path(p);
        assert_eq!(normalized, PathBuf::from("/file.txt"));
    }
}
