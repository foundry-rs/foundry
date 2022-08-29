//! Support for controlling fs access

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fmt,
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
    /// Creates anew instance with the given `permission` and `allowed_paths`
    pub fn new(
        permission: FsAccessPermission,
        allowed_paths: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        Self { permission, allowed_paths: allowed_paths.into_iter().map(Into::into).collect() }
    }

    /// Returns true if access to the specified path is allowed with the specified.
    ///
    /// This first checks permission, and only if it is granted, whether the path is allowed.
    ///
    /// We only allow paths that are inside  allowed paths.
    ///
    /// Caution: This should be called with normalized paths if the `allowed_paths` are also
    /// normalized.
    pub fn is_path_allowed(&self, path: &Path, kind: FsAccessKind) -> bool {
        if !self.permission.is_granted(kind) {
            return false
        }
        self.allowed_paths.iter().any(|allowed_path| path.starts_with(allowed_path))
    }

    /// Updates all `allowed_paths` and joins ([`Path::join`]) the `root` with all entries
    pub fn join_all(&mut self, root: impl AsRef<Path>) {
        let root = root.as_ref();
        self.allowed_paths.iter_mut().for_each(|p| {
            *p = root.join(&*p);
        })
    }

    /// Same as [`Self::join_all`] but consumes the type
    pub fn joined(mut self, root: impl AsRef<Path>) -> Self {
        self.join_all(root);
        self
    }
}

/// Represents the operation on the fs
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FsAccessKind {
    /// read from fs (`vm.readFile`)
    Read,
    /// write to fs (`vm.writeFile`)
    Write,
}

impl fmt::Display for FsAccessKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsAccessKind::Read => f.write_str("read"),
            FsAccessKind::Write => f.write_str("write"),
        }
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

// === impl FsAccessPermission ===

impl FsAccessPermission {
    /// Returns true if the access is allowed
    pub fn is_granted(&self, kind: FsAccessKind) -> bool {
        match (self, kind) {
            (FsAccessPermission::Enabled, _) => true,
            (FsAccessPermission::Disabled, _) => false,
            (FsAccessPermission::Read, FsAccessKind::Read) => true,
            (FsAccessPermission::Write, FsAccessKind::Write) => true,
            _ => false,
        }
    }
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

impl fmt::Display for FsAccessPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsAccessPermission::Enabled => f.write_str("true"),
            FsAccessPermission::Disabled => f.write_str("false"),
            FsAccessPermission::Read => f.write_str("read"),
            FsAccessPermission::Write => f.write_str("write"),
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
