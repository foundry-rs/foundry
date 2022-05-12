//! common utilities used in foundryup
use crate::{
    errors::FoundryupError,
    process::{with_default, Processor},
};
use eyre::WrapErr;
use std::{
    env, fs,
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// Returns the storage directory used by foundryup
///
/// It returns one of the following values, in this order of
/// preference:
///
/// - The value of the `FOUNDRY_HOME` environment variable, if it is an absolute path.
/// - The value of the current working directory joined with the value of the `FOUNDRY_HOME`
///   environment variable, if `FOUNDRY_HOME` is a relative directory.
/// - The `.foundry` directory in the user's home directory, as reported by the `home_dir` function.
///
/// # Errors
///
/// This function fails if it fails to retrieve the current directory,
/// or if the home directory cannot be determined.
pub fn foundry_home() -> io::Result<PathBuf> {
    with_default(|env| foundry_home_from(&**env))
}

pub fn foundry_home_from(env: &dyn Processor) -> io::Result<PathBuf> {
    let cwd = env.current_dir()?;
    foundry_home_with_cwd_from(env, &cwd)
}

pub fn foundry_home_with_cwd_from(env: &dyn Processor, cwd: &Path) -> io::Result<PathBuf> {
    if let Some(home) = env.var_os("FOUNDRY_HOME").filter(|v| !v.is_empty()) {
        {
            let home = PathBuf::from(home);
            return if home.is_absolute() { Ok(home) } else { Ok(cwd.join(&home)) }
        }
    }
    env.home_dir()
        .map(|d| d.join(".foundry"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "could not find foundry home dir"))
}

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
