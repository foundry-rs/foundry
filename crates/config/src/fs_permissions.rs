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
#[serde(transparent)]
pub struct FsPermissions {
    /// what kind of access is allowed
    pub permissions: Vec<PathPermission>,
}

// === impl FsPermissions ===

impl FsPermissions {
    /// Creates anew instance with the given `permissions`
    pub fn new(permissions: impl IntoIterator<Item = PathPermission>) -> Self {
        Self { permissions: permissions.into_iter().collect() }
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
        self.find_permission(path).map(|perm| perm.is_granted(kind)).unwrap_or_default()
    }

    /// Returns the permission for the matching path
    pub fn find_permission(&self, path: &Path) -> Option<FsAccessPermission> {
        self.permissions.iter().find(|perm| path.starts_with(&perm.path)).map(|perm| perm.access)
    }

    /// Updates all `allowed_paths` and joins ([`Path::join`]) the `root` with all entries
    pub fn join_all(&mut self, root: impl AsRef<Path>) {
        let root = root.as_ref();
        self.permissions.iter_mut().for_each(|perm| {
            perm.path = root.join(&perm.path);
        })
    }

    /// Same as [`Self::join_all`] but consumes the type
    pub fn joined(mut self, root: impl AsRef<Path>) -> Self {
        self.join_all(root);
        self
    }

    /// Removes all existing permissions for the given path
    pub fn remove(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        self.permissions.retain(|permission| permission.path != path)
    }

    /// Returns true if no permissions are configured
    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty()
    }

    /// Returns the number of configured permissions
    pub fn len(&self) -> usize {
        self.permissions.len()
    }
}

/// Represents an access permission to a single path
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PathPermission {
    /// Permission level to access the `path`
    pub access: FsAccessPermission,
    /// The targeted path guarded by the permission
    pub path: PathBuf,
}

// === impl PathPermission ===

impl PathPermission {
    /// Returns a new permission for the path and the given access
    pub fn new(path: impl Into<PathBuf>, access: FsAccessPermission) -> Self {
        Self { path: path.into(), access }
    }

    /// Returns a new read-only permission for the path
    pub fn read(path: impl Into<PathBuf>) -> Self {
        Self::new(path, FsAccessPermission::Read)
    }

    /// Returns a new read-write permission for the path
    pub fn read_write(path: impl Into<PathBuf>) -> Self {
        Self::new(path, FsAccessPermission::ReadWrite)
    }

    /// Returns a new write-only permission for the path
    pub fn write(path: impl Into<PathBuf>) -> Self {
        Self::new(path, FsAccessPermission::Write)
    }

    /// Returns a non permission for the path
    pub fn none(path: impl Into<PathBuf>) -> Self {
        Self::new(path, FsAccessPermission::None)
    }

    /// Returns true if the access is allowed
    pub fn is_granted(&self, kind: FsAccessKind) -> bool {
        self.access.is_granted(kind)
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
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum FsAccessPermission {
    /// FS access is _not_ allowed
    #[default]
    None,
    /// FS access is allowed, this includes `read` + `write`
    ReadWrite,
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
            (FsAccessPermission::ReadWrite, _) => true,
            (FsAccessPermission::None, _) => false,
            (FsAccessPermission::Read, FsAccessKind::Read) => true,
            (FsAccessPermission::Write, FsAccessKind::Write) => true,
            _ => false,
        }
    }
}

impl FromStr for FsAccessPermission {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "true" | "read-write" | "readwrite" => Ok(FsAccessPermission::ReadWrite),
            "false" | "none" => Ok(FsAccessPermission::None),
            "read" => Ok(FsAccessPermission::Read),
            "write" => Ok(FsAccessPermission::Write),
            _ => Err(format!("Unknown variant {s}")),
        }
    }
}

impl fmt::Display for FsAccessPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsAccessPermission::ReadWrite => f.write_str("read-write"),
            FsAccessPermission::None => f.write_str("none"),
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
            FsAccessPermission::ReadWrite => serializer.serialize_bool(true),
            FsAccessPermission::None => serializer.serialize_bool(false),
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
                let status =
                    if enabled { FsAccessPermission::ReadWrite } else { FsAccessPermission::None };
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
        assert_eq!(FsAccessPermission::ReadWrite, "true".parse().unwrap());
        assert_eq!(FsAccessPermission::ReadWrite, "readwrite".parse().unwrap());
        assert_eq!(FsAccessPermission::ReadWrite, "read-write".parse().unwrap());
        assert_eq!(FsAccessPermission::None, "false".parse().unwrap());
        assert_eq!(FsAccessPermission::None, "none".parse().unwrap());
        assert_eq!(FsAccessPermission::Read, "read".parse().unwrap());
        assert_eq!(FsAccessPermission::Write, "write".parse().unwrap());
    }
}
