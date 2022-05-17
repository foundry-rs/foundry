use std::path::PathBuf;
use thiserror::Error;

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
    #[error("could not remove '{name}' file: '{}'", .path.display())]
    RemovingFile { name: String, path: PathBuf },
    #[error("could not write {name} file: '{}'", .path.display())]
    WritingFile { name: String, path: PathBuf },
    #[error("couldn't determine self executable name")]
    NoExeName,
}
