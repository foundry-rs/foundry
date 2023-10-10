use super::{Cheatcodes, Result};
use crate::abi::hevm::{DirEntry, FsMetadata, HEVMCalls};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Bytes, U256};
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use foundry_utils::types::ToAlloy;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

fn project_root(state: &Cheatcodes) -> Result {
    let root = state.config.root.display().to_string();
    Ok(DynSolValue::String(root).abi_encode().into())
}

fn read_file(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let data = fs::read_to_string(path)?;
    Ok(DynSolValue::String(data).abi_encode().into())
}

fn read_file_binary(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let data = fs::read(path)?;
    Ok(DynSolValue::Bytes(data).abi_encode().into())
}

fn read_line(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    // Get reader for previously opened file to continue reading OR initialize new reader
    let reader = state
        .context
        .opened_read_files
        .entry(path.clone())
        .or_insert(BufReader::new(fs::open(path)?));

    let mut line: String = String::new();
    reader.read_line(&mut line)?;

    // Remove trailing newline character, preserving others for cases where it may be important
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }

    Ok(DynSolValue::String(line).abi_encode().into())
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
) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
    // write access to foundry.toml is not allowed
    state.config.ensure_not_foundry_toml(&path)?;

    if state.fs_commit {
        fs::write(path, content.as_ref())?;
    }

    Ok(Bytes::new())
}

/// Writes a single line to the file.
///
/// This will create a file if it does not exist, and append the `line` if it does.
fn write_line(state: &Cheatcodes, path: impl AsRef<Path>, line: &str) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
    state.config.ensure_not_foundry_toml(&path)?;

    if state.fs_commit {
        let mut file = std::fs::OpenOptions::new().append(true).create(true).open(path)?;

        writeln!(file, "{line}")?;
    }

    Ok(Bytes::new())
}

fn copy_file(state: &Cheatcodes, from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result {
    let from = state.config.ensure_path_allowed(from, FsAccessKind::Read)?;
    let to = state.config.ensure_path_allowed(to, FsAccessKind::Write)?;
    state.config.ensure_not_foundry_toml(&to)?;

    let n = fs::copy(from, to)?;
    Ok(DynSolValue::Uint(U256::from(n), 256).abi_encode().into())
}

fn close_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    state.context.opened_read_files.remove(&path);

    Ok(Bytes::new())
}

/// Removes a file from the filesystem.
///
/// Only files inside `<root>/` can be removed, `foundry.toml` excluded.
///
/// This will return an error if the path points to a directory, or the file does not exist
fn remove_file(state: &mut Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
    state.config.ensure_not_foundry_toml(&path)?;

    // also remove from the set if opened previously
    state.context.opened_read_files.remove(&path);

    if state.fs_commit {
        fs::remove_file(&path)?;
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
fn create_dir(state: &Cheatcodes, path: impl AsRef<Path>, recursive: bool) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
    if recursive { fs::create_dir_all(path) } else { fs::create_dir(path) }?;
    Ok(Bytes::new())
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
fn remove_dir(state: &Cheatcodes, path: impl AsRef<Path>, recursive: bool) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Write)?;
    if recursive { fs::remove_dir_all(path) } else { fs::remove_dir(path) }?;
    Ok(Bytes::new())
}

/// Reads the directory at the given path recursively, up to `max_depth`.
///
/// Follows symbolic links if `follow_links` is true.
fn read_dir(
    state: &Cheatcodes,
    path: impl AsRef<Path>,
    max_depth: u64,
    follow_links: bool,
) -> Result {
    let root = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
    let paths: Vec<DynSolValue> = WalkDir::new(root)
        .min_depth(1)
        .max_depth(max_depth.try_into()?)
        .follow_links(follow_links)
        .contents_first(false)
        .same_file_system(true)
        .sort_by_file_name()
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
            DynSolValue::Tuple(vec![
                DynSolValue::String(entry.error_message),
                DynSolValue::String(entry.path),
                DynSolValue::Uint(U256::from(entry.depth), 8),
                DynSolValue::Bool(entry.is_dir),
                DynSolValue::Bool(entry.is_symlink),
            ])
        })
        .collect();
    Ok(DynSolValue::Array(paths).abi_encode().into())
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
fn read_link(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    let target = fs::read_link(path)?;

    Ok(DynSolValue::String(target.display().to_string()).abi_encode().into())
}

/// Gets the metadata of a file/directory
///
/// This will return an error if no file/directory is found, or if the target path isn't allowed
fn fs_metadata(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    let metadata = path.metadata()?;

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
    Ok(DynSolValue::Tuple(vec![
        DynSolValue::Bool(metadata.is_dir),
        DynSolValue::Bool(metadata.is_symlink),
        DynSolValue::Uint(U256::from(metadata.length.to_alloy()), 256),
        DynSolValue::Bool(metadata.read_only),
        DynSolValue::Uint(U256::from(metadata.modified.to_alloy()), 256),
        DynSolValue::Uint(U256::from(metadata.accessed.to_alloy()), 256),
        DynSolValue::Uint(U256::from(metadata.created.to_alloy()), 256),
    ])
    .abi_encode()
    .into())
}

/// Verifies if a given path points to a valid entity
///
/// This function will return `true` if `path` points to a valid filesystem entity, otherwise it
/// will return `false`
///
/// Note: This function does not verify if a user has necessary permissions to access the path,
/// only that such a path exists
fn exists(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    Ok(DynSolValue::Bool(path.exists()).abi_encode().into())
}

/// Verifies if a given path exists on disk and points at a regular file
///
/// This function will return `true` if `path` points to a regular file that exists on the disk,
/// otherwise it will return `false`
///
/// Note: This function does not verify if a user has necessary permissions to access the file,
/// only that such a file exists on disk
fn is_file(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    Ok(DynSolValue::Bool(path.is_file()).abi_encode().into())
}

/// Verifies if a given path exists on disk and points at a directory
///
/// This function will return `true` if `path` points to a directory that exists on the disk,
/// otherwise it will return `false`
///
/// Note: This function does not verify if a user has necessary permissions to access the directory,
/// only that such a directory exists
fn is_dir(state: &Cheatcodes, path: impl AsRef<Path>) -> Result {
    let path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;

    Ok(DynSolValue::Bool(path.is_dir()).abi_encode().into())
}

#[instrument(level = "error", name = "fs", target = "evm::cheatcodes", skip_all)]
pub fn apply(state: &mut Cheatcodes, call: &HEVMCalls) -> Option<Result> {
    let res = match call {
        HEVMCalls::ProjectRoot(_) => project_root(state),
        HEVMCalls::ReadFile(inner) => read_file(state, &inner.0),
        HEVMCalls::ReadFileBinary(inner) => read_file_binary(state, &inner.0),
        HEVMCalls::ReadLine(inner) => read_line(state, &inner.0),
        HEVMCalls::WriteFile(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteFileBinary(inner) => write_file(state, &inner.0, &inner.1),
        HEVMCalls::WriteLine(inner) => write_line(state, &inner.0, &inner.1),
        HEVMCalls::CopyFile(inner) => copy_file(state, &inner.0, &inner.1),
        HEVMCalls::CloseFile(inner) => close_file(state, &inner.0),
        HEVMCalls::RemoveFile(inner) => remove_file(state, &inner.0),
        HEVMCalls::FsMetadata(inner) => fs_metadata(state, &inner.0),
        HEVMCalls::ReadLink(inner) => read_link(state, &inner.0),
        HEVMCalls::CreateDir(inner) => create_dir(state, &inner.0, inner.1),
        HEVMCalls::RemoveDir(inner) => remove_dir(state, &inner.0, inner.1),
        HEVMCalls::ReadDir0(inner) => read_dir(state, &inner.0, 1, false),
        HEVMCalls::ReadDir1(inner) => read_dir(state, &inner.0, inner.1, false),
        HEVMCalls::ReadDir2(inner) => read_dir(state, &inner.0, inner.1, inner.2),
        HEVMCalls::Exists(inner) => exists(state, &inner.0),
        HEVMCalls::IsFile(inner) => is_file(state, &inner.0),
        HEVMCalls::IsDir(inner) => is_dir(state, &inner.0),

        _ => return None,
    };
    Some(res)
}
