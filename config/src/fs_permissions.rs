//! Support for controlling fs access

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

/// Configures file system access
///
/// E.g. for cheat codes (`vm.writeFile`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FsPermissions {
    /// what kind of access is allowed
    pub permission: FsAccessPermission,
    /// All paths that are allowed to access
    pub allowed_paths: Vec<PathBuf>,
}

// === impl FsPermissions ===

impl FsPermissions {
    /// Updates all `allowed_paths` and joins ([`Path::join`]) the `root` with all entries
    pub fn join_all(&mut self, root: impl AsRef<Path>) {
        let root = root.as_ref();
        self.allowed_paths.iter_mut().for_each(|p| {
            *p = root.join(&*p);
        })
    }
}

/// Determines the status of file system access
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FsAccessPermission {
    /// FS access is allowed, this includes `read` + `write`
    Enabled,
    /// FS access is _not_ allowed
    Disabled,
    /// Only reading is allowed
    Read,
    /// Only writing is allowed
    Write,
}

impl Default for FsAccessPermission {
    fn default() -> Self {
        FsAccessPermission::Disabled
    }
}

impl FromStr for FsAccessPermission {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "true" | "read-write" => Ok(FsAccessPermission::Enabled),
            "false" | "none" => Ok(FsAccessPermission::Disabled),
            "read" => Ok(FsAccessPermission::Read),
            "write" => Ok(FsAccessPermission::Write),
            _ => Err(format!("Unknown variant {}", s)),
        }
    }
}

impl Serialize for FsAccessPermission {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            FsAccessPermission::Enabled => serializer.serialize_bool(true),
            FsAccessPermission::Disabled => serializer.serialize_bool(false),
            FsAccessPermission::Read => serializer.serialize_str("read"),
            FsAccessPermission::Write => serializer.serialize_str("write"),
        }
    }
}

impl<'de> Deserialize<'de> for FsAccessPermission {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Status {
            Bool(bool),
            String(String),
        }
        match Status::deserialize(deserializer)? {
            Status::Bool(enabled) => {
                let status = if enabled {
                    FsAccessPermission::Enabled
                } else {
                    FsAccessPermission::Disabled
                };
                Ok(status)
            }
            Status::String(val) => val.parse().map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_permission() {
        assert_eq!(FsAccessPermission::Enabled, "true".parse().unwrap());
        assert_eq!(FsAccessPermission::Disabled, "false".parse().unwrap());
        assert_eq!(FsAccessPermission::Disabled, "none".parse().unwrap());
        assert_eq!(FsAccessPermission::Read, "read".parse().unwrap());
        assert_eq!(FsAccessPermission::Write, "write".parse().unwrap());
    }
}
