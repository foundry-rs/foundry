use crate::{
    abi::{DirEntry, FsMetadata, HEVMCalls},
    error,
    executor::inspector::Cheatcodes,
};
use bytes::Bytes;
use ethers::abi::{self, AbiEncode, Token, Tokenize};
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

fn project_root(state: &Cheatcodes) -> Result<Bytes, Bytes> {
    let root = state.config.root.display().to_string();

    Ok(abi::encode(&[Token::String(root)]).into())
}

fn read_file(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read_to_string(path).map_err(error::encode_error)?;

    Ok(abi::encode(&[Token::String(data)]).into())
}

fn read_file_binary(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    let data = fs::read(path).map_err(error::encode_error)?;

    Ok(abi::encode(&[Token::Bytes(data)]).into())
}

fn read_line(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    // Get reader for previously opened file to continue reading OR initialize new reader
    let reader = state
        .context
        .opened_read_files
        .entry(path.clone())
        .or_insert(BufReader::new(fs::open(path).map_err(error::encode_error)?));

    let mut line: String = String::new();
    reader.read_line(&mut line).map_err(error::encode_error)?;

    // Remove trailing newline character, preserving others for cases where it may be important
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    Ok(abi::encode(&[Token::String(line)]).into())
}

/// Writes `content` to `path`.
///
/// This function will create a file if it does not exist, and will entirely replace its contents if
/// it does.
///
/// Caution: writing files is only allowed if the targeted path is allowed, (inside `<root>/` by
/// default)
pub(super) fn write_file(
    state: &Cheatcodes,
    path: impl AsRef<Path>,
    content: impl AsRef<[u8]>,
) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Write).map_err(error::encode_error)?;
    // write access to foundry.toml is not allowed
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    if state.fs_commit {
        fs::write(path, content.as_ref()).map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

/// Writes a single line to the file.
///
/// This will create a file if it does not exist, and append the `line` if it does.
fn write_line(state: &Cheatcodes, path: impl AsRef<Path>, line: &str) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Write).map_err(error::encode_error)?;
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    if state.fs_commit {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(error::encode_error)?;

        writeln!(file, "{line}").map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

fn close_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    state.context.opened_read_files.remove(&path);

    Ok(Bytes::new())
}

/// Removes a file from the filesystem.
///
/// Only files inside `<root>/` can be removed, `foundry.toml` excluded.
///
/// This will return an error if the path points to a directory, or the file does not exist
fn remove_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Write).map_err(error::encode_error)?;
    state.config.ensure_not_foundry_toml(&path).map_err(error::encode_error)?;

    // also remove from the set if opened previously
    state.context.opened_read_files.remove(&path);

    if state.fs_commit {
        fs::remove_file(&path).map_err(error::encode_error)?;
    }

    Ok(Bytes::new())
}

/// Creates a new, empty directory at the provided path.
///
/// If `recursive` is true, it will also create all the parent directories if they don't exist.
///
/// # Errors
///
/// This function will return an error in the following situations, but is not limited to just these
/// cases:
///
/// - User lacks permissions to modify `path`.
/// - A parent of the given path doesn't exist and `recursive` is false.
/// - `path` already exists and `recursive` is false.
fn create_dir(state: &Cheatcodes, path: impl AsRef<Path>, recursive: bool) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Write).map_err(error::encode_error)?;
    if recursive { fs::create_dir_all(path) } else { fs::create_dir(path) }
        .map(|()| Bytes::new())
        .map_err(error::encode_error)
}

/// Removes a directory at the provided path.
///
/// This will also remove all the directory's contents recursively if `recursive` is true.
///
/// # Errors
///
/// This function will return an error in the following situations, but is not limited to just these
/// cases:
///
/// - `path` doesn't exist.
/// - `path` isn't a directory.
/// - User lacks permissions to modify `path`.
/// - The directory is not empty and `recursive` is false.
fn remove_dir(state: &Cheatcodes, path: impl AsRef<Path>, recursive: bool) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Write).map_err(error::encode_error)?;
    if recursive { fs::remove_dir_all(path) } else { fs::remove_dir(path) }
        .map(|()| Bytes::new())
        .map_err(error::encode_error)
}

/// Reads the directory at the given path recursively, up to `max_depth`.
///
/// Follows symbolic links if `follow_links` is true.
fn read_dir(
    state: &Cheatcodes,
    path: impl AsRef<Path>,
    max_depth: u64,
    follow_links: bool,
) -> Result<Bytes, Bytes> {
    let root =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;
    let paths: Vec<Token> = WalkDir::new(root)
        .min_depth(1)
        .max_depth(max_depth.try_into().map_err(error::encode_error)?)
        .follow_links(follow_links)
        .into_iter()
        .map(|entry| {
            let entry = match entry {
                Ok(entry) => DirEntry {
                    error_message: String::new(),
                    path: entry.path().display().to_string(),
                    depth: entry.depth() as u64,
                    is_dir: entry.file_type().is_dir(),
                    is_symlink: entry.path_is_symlink(),
                },
                Err(e) => DirEntry {
                    error_message: e.to_string(),
                    path: e.path().map(|p| p.display().to_string()).unwrap_or_default(),
                    depth: e.depth() as u64,
                    is_dir: false,
                    is_symlink: false,
                },
            };
            Token::Tuple(entry.into_tokens())
        })
        .collect();
    Ok(abi::encode(&[Token::Array(paths)]).into())
}

/// Reads a symbolic link, returning the path that the link points to.
///
/// # Errors
///
/// This function will return an error in the following situations, but is not limited to just these
/// cases:
///
/// - `path` is not a symbolic link.
/// - `path` does not exist.
fn read_link(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    let target = fs::read_link(path).map_err(error::encode_error)?;

    Ok(abi::encode(&[Token::String(target.display().to_string())]).into())
}

/// Gets the metadata of a file/directory
///
/// This will return an error if no file/directory is found, or if the target path isn't allowed
fn fs_metadata(state: &Cheatcodes, path: impl AsRef<Path>) -> Result<Bytes, Bytes> {
    let path =
        state.config.ensure_path_allowed(path, FsAccessKind::Read).map_err(error::encode_error)?;

    let metadata = path.metadata().map_err(error::encode_error)?;

    // These fields not available on all platforms; default to 0
    let [modified, accessed, created] =
        [metadata.modified(), metadata.accessed(), metadata.created()].map(|time| {
            time.unwrap_or(UNIX_EPOCH).duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
        });

    let metadata = FsMetadata {
        is_dir: metadata.is_dir(),
        is_symlink: metadata.is_symlink(),
        length: metadata.len().into(),
        read_only: metadata.permissions().readonly(),
        modified: modified.into(),
        accessed: accessed.into(),
        created: created.into(),
    };
    Ok(metadata.encode().into())
}

pub fn apply(state: &mut Cheatcodes, call: &HEVMCalls) -> Option<Result<Bytes, Bytes>> {
    let res = match call {
        HEVMCalls::ProjectRoot(_) => project_root(state),
        HEVMCalls::ReadFile(inner) => read_file(state, &inner.0),
        HEVMCalls::ReadFileBinary(inner) => read_file_binary(state, &inner.0),
        HEVMCalls::ReadLine(inner) => read_line(state, &inner.0),
        HEVMCalls::WriteFile(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteFileBinary(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteLine(inner) => write_line(state, &inner.0, &inner.1),
        HEVMCalls::CloseFile(inner) => close_file(state, &inner.0),
        HEVMCalls::RemoveFile(inner) => remove_file(state, &inner.0),
        HEVMCalls::FsMetadata(inner) => fs_metadata(state, &inner.0),
        HEVMCalls::ReadLink(inner) => read_link(state, &inner.0),
        HEVMCalls::CreateDir(inner) => create_dir(state, &inner.0, inner.1),
        HEVMCalls::RemoveDir(inner) => remove_dir(state, &inner.0, inner.1),
        HEVMCalls::ReadDir0(inner) => read_dir(state, &inner.0, 1, false),
        HEVMCalls::ReadDir1(inner) => read_dir(state, &inner.0, inner.1, false),
        HEVMCalls::ReadDir2(inner) => read_dir(state, &inner.0, inner.1, inner.2),

        _ => return None,
    };
    Some(res)
}
