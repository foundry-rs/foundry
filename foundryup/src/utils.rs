//! common utilities used in foundryup
use crate::errors::FoundryupError;
use eyre::WrapErr;
use std::{
    env, fs,
    fs::File,
    io,
    path::{Path, PathBuf},
};

// pub fn current_dir() -> eyre::Result<PathBuf> {
//     process().current_dir().wrap_err(RustupError::LocatingWorkingDir)
// }

pub fn current_exe() -> eyre::Result<PathBuf> {
    env::current_exe().context(FoundryupError::LocatingWorkingDir)
}

pub(crate) fn open_file(name: &str, path: &Path) -> eyre::Result<File> {
    File::open(path).with_context(|| FoundryupError::ReadingFile {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

pub fn read_file(name: &str, path: &Path) -> eyre::Result<String> {
    fs::read_to_string(path).with_context(|| FoundryupError::ReadingFile {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

pub fn write_file(name: &str, path: &Path, contents: impl AsRef<[u8]>) -> eyre::Result<()> {
    write_new_file(path, contents).with_context(|| FoundryupError::WritingFile {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

/// Writes the `contents` to the given path
///
/// The file will be truncated and created
pub fn write_new_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).truncate(true).create(true).open(path)?;
    io::Write::write_all(&mut file, contents.as_ref())?;
    file.sync_data()?;

    Ok(())
}
