//! common utilities used in foundryup
use crate::{
    errors::FoundryupError,
    process::{get_process, with_default, Processor},
};
use eyre::WrapErr;
use std::{
    env, fs,
    fs::File,
    io,
    path::{Path, PathBuf},
};
use url::Url;

/// The version message for the current program, like
/// `foundryup 0.1.0 (f01b232bc 2022-01-22T23:28:39.493201+00:00)`
pub(crate) const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA_SHORT"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

/// New type for a process exit code
pub struct ExitCode(pub i32);

impl From<i32> for ExitCode {
    fn from(code: i32) -> Self {
        ExitCode(code)
    }
}

/// Returns the storage directory used by foundryup
pub fn foundryup_dir() -> io::Result<PathBuf> {
    Ok(foundry_home()?.join("foundryup"))
}

/// Returns the home directory used by foundry
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

/// Remove the given file
pub fn remove_file(name: &str, path: &Path) -> eyre::Result<()> {
    fs::remove_file(path).with_context(|| FoundryupError::RemovingFile {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

pub(crate) fn parse_url(url: &str) -> eyre::Result<Url> {
    Url::parse(url).with_context(|| format!("failed to parse url: {}", url))
}

/// Downloads the file from the `url` to the given `path`
pub async fn download_file(url: &Url, path: &Path) -> eyre::Result<()> {
    download_file_(url, path).await.with_context(|| FoundryupError::DownloadingFile {
        url: url.clone(),
        path: path.to_path_buf(),
    })
}

async fn download_file_(_url: &Url, _path: &Path) -> eyre::Result<()> {
    let _process = get_process();

    Ok(())
}

pub(crate) fn make_executable(path: &Path) -> eyre::Result<()> {
    #[allow(clippy::unnecessary_wraps)]
    #[cfg(windows)]
    fn inner(_: &Path) -> Result<()> {
        Ok(())
    }
    #[cfg(not(windows))]
    fn inner(path: &Path) -> eyre::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .map_err(|e| FoundryupError::SettingPermissions { p: PathBuf::from(path), source: e })?;
        let mut perms = metadata.permissions();
        let mode = perms.mode();
        let new_mode = (mode & !0o777) | 0o755;

        // Check if permissions are ok already - #1638
        if mode == new_mode {
            return Ok(())
        }

        perms.set_mode(new_mode);
        set_permissions(path, perms)
    }

    inner(path)
}

#[cfg(not(windows))]
fn set_permissions(path: &Path, perms: fs::Permissions) -> eyre::Result<()> {
    fs::set_permissions(path, perms).map_err(|e| {
        FoundryupError::SettingPermissions { p: PathBuf::from(path), source: e }.into()
    })
}
