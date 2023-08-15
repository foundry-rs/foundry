use figment::Profile;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

/// Warnings emitted during loading or managing Configuration
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
#[serde(tag = "type")]
pub enum Warning {
    /// An unknown section was encountered in a TOML file
    UnknownSection {
        /// The unknown key
        unknown_section: Profile,
        /// The source where the key was found
        source: Option<String>,
    },
    /// No local TOML file found, with location tried
    NoLocalToml(PathBuf),
    /// Could not read TOML
    CouldNotReadToml {
        /// The path of the TOML file
        path: PathBuf,
        /// The error message that occurred
        err: String,
    },
    /// Could not write TOML
    CouldNotWriteToml {
        /// The path of the TOML file
        path: PathBuf,
        /// The error message that occurred
        err: String,
    },
    /// Invalid profile. Profile should be a table
    CouldNotFixProfile {
        /// The path of the TOML file
        path: PathBuf,
        /// The profile to be fixed
        profile: String,
        /// The error message that occurred
        err: String,
    },
    /// Deprecated key.
    DeprecatedKey {
        /// The key being deprecated
        old: String,
        /// The new key replacing the deprecated one if not empty, otherwise, meaning the old one
        /// is being removed completely without replacement
        new: String,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSection { unknown_section, source } => {
                let source = source.as_ref().map(|src| format!(" in {src}")).unwrap_or_default();
                f.write_fmt(format_args!("Unknown section [{unknown_section}] found{source}. This notation for profiles has been deprecated and may result in the profile not being registered in future versions. Please use [profile.{unknown_section}] instead or run `forge config --fix`."))
            }
            Self::NoLocalToml(tried) => {
                let path = tried.display();
                f.write_fmt(format_args!("No local TOML found to fix at {path}. Change the current directory to a project path or set the foundry.toml path with the FOUNDRY_CONFIG environment variable"))
            }
            Self::CouldNotReadToml { path, err } => {
                f.write_fmt(format_args!("Could not read TOML at {}: {err}", path.display()))
            }
            Self::CouldNotWriteToml { path, err } => {
                f.write_fmt(format_args!("Could not write TOML to {}: {err}", path.display()))
            }
            Self::CouldNotFixProfile { path, profile, err } => f.write_fmt(format_args!(
                "Could not fix [{}] in TOML at {}: {}",
                profile,
                path.display(),
                err
            )),
            Self::DeprecatedKey { old, new } if new.is_empty() => f.write_fmt(format_args!(
                "Key `{old}` is being deprecated and will be removed in future versions.",
            )),
            Self::DeprecatedKey { old, new } => f.write_fmt(format_args!(
                "Key `{old}` is being deprecated in favor of `{new}`. It will be removed in future versions.",
            )),
        }
    }
}

impl std::error::Error for Warning {}
