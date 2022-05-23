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
    io::Write,
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

/// Returns the home directory
pub fn home_dir() -> Option<PathBuf> {
    get_process().home_dir()
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

pub(crate) fn append_file(name: &str, path: &Path, line: &str) -> eyre::Result<()> {
    append_file_(path, line).with_context(|| FoundryupError::WritingFile {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

fn append_file_(dest: &Path, line: &str) -> io::Result<()> {
    let mut dest_file = fs::OpenOptions::new().write(true).append(true).create(true).open(dest)?;

    writeln!(dest_file, "{}", line)?;

    dest_file.sync_data()?;

    Ok(())
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

        let metadata = fs::metadata(path).map_err(|e| FoundryupError::SettingPermissions {
            p: PathBuf::from(path),
            source: e,
        })?;
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

pub(crate) fn remove_dir(name: &str, path: &Path) -> eyre::Result<()> {
    remove_dir_(path).with_context(|| FoundryupError::RemovingDirectory {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

fn remove_dir_(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        if cfg!(windows) {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
    } else {
        // remove_dir all doesn't delete write-only files on windows
        remove_dir_all::remove_dir_all(path)
    }
}

pub fn ensure_dir_exists(name: &str, path: &Path) -> eyre::Result<bool> {
    ensure_dir_exists_(path).with_context(|| FoundryupError::CreatingDirectory {
        name: name.to_string(),
        path: PathBuf::from(path),
    })
}

fn ensure_dir_exists_(path: impl AsRef<Path>) -> io::Result<bool> {
    if !is_directory(path.as_ref()) {
        fs::create_dir_all(path.as_ref()).map(|()| true)
    } else {
        Ok(false)
    }
}

pub(crate) fn is_directory<P: AsRef<Path>>(path: P) -> bool {
    fs::metadata(path).ok().as_ref().map(fs::Metadata::is_dir) == Some(true)
}

pub fn is_file<P: AsRef<Path>>(path: P) -> bool {
    fs::metadata(path).ok().as_ref().map(fs::Metadata::is_file) == Some(true)
}

pub(crate) fn copy_file(src: &Path, dest: &Path) -> eyre::Result<()> {
    let metadata = fs::symlink_metadata(src).with_context(|| FoundryupError::ReadingFile {
        name: "metadata for".to_string(),
        path: PathBuf::from(src),
    })?;
    if metadata.file_type().is_symlink() {
        symlink_file(src, dest).map(|_| ())
    } else {
        fs::copy(src, dest)
            .with_context(|| {
                format!("could not copy file from '{}' to '{}'", src.display(), dest.display())
            })
            .map(|_| ())
    }
}

pub fn hardlink_file(src: &Path, dest: &Path) -> eyre::Result<()> {
    let _ = fs::remove_file(dest);
    fs::hard_link(src, dest).with_context(|| FoundryupError::LinkingFile {
        src: PathBuf::from(src),
        dest: PathBuf::from(dest),
    })
}

#[cfg(unix)]
fn symlink_file(src: &Path, dest: &Path) -> eyre::Result<()> {
    std::os::unix::fs::symlink(src, dest).with_context(|| FoundryupError::LinkingFile {
        src: PathBuf::from(src),
        dest: PathBuf::from(dest),
    })
}

#[cfg(not(windows))]
fn has_cmd(cmd: &str) -> bool {
    let cmd = format!("{}{}", cmd, env::consts::EXE_SUFFIX);
    let path = get_process().var_os("PATH").unwrap_or_default();
    env::split_paths(&path).map(|p| p.join(&cmd)).any(|p| p.exists())
}

#[cfg(not(windows))]
pub(crate) fn find_cmd<'a>(cmds: &[&'a str]) -> Option<&'a str> {
    cmds.iter().cloned().find(|&s| has_cmd(s))
}
