use std::path::PathBuf;
use thiserror::Error;
use url::Url;

/// Main error type
#[derive(Error, Debug)]
pub enum FoundryupError {
    #[error("Unable to proceed. Could not locate working directory.")]
    LocatingWorkingDir,
    #[error("could not read {name} directory: '{}'", .path.display())]
    ReadingDirectory { name: String, path: PathBuf },
    #[error("could not read {name} file: '{}'", .path.display())]
    ReadingFile { name: String, path: PathBuf },
    #[error("could not remove '{}' directory: '{}'", .name, .path.display())]
    RemovingDirectory { name: String, path: PathBuf },
    #[error("could not create {name} directory: '{}'", .path.display())]
    CreatingDirectory { name: String, path: PathBuf },
    #[error("could not remove '{name}' file: '{}'", .path.display())]
    RemovingFile { name: String, path: PathBuf },
    #[error("could not write {name} file: '{}'", .path.display())]
    WritingFile { name: String, path: PathBuf },
    #[error("couldn't determine self executable name")]
    NoExeName,
    #[error("foundryup is not installed at '{}'", .p.display())]
    FoundryupNotInstalled { p: PathBuf },
    #[error("could not download file from '{url}' to '{}'", .path.display())]
    DownloadingFile { url: Url, path: PathBuf },
    #[error("current platform is not supported by foundry: os='{os}' arch='{arch}'")]
    UnsupportedPlatform { os: &'static str, arch: &'static str },
    #[error("failed to set permissions for '{}'", .p.display())]
    SettingPermissions { p: PathBuf, source: std::io::Error },
    #[error("could not create link from '{}' to '{}'", .src.display(), .dest.display())]
    LinkingFile { src: PathBuf, dest: PathBuf },
    #[error("failure during windows uninstall")]
    WindowsUninstallMadness,
}
