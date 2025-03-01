use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

#[cfg(feature = "walkdir")]
mod find;

/// The solidity compiler can only reference files that exist locally on your computer.
/// So importing directly from GitHub (as an example) is not possible.
///
/// Let's imagine you want to use OpenZeppelin's amazing library of smart contracts,
/// `@openzeppelin/contracts-ethereum-package`:
///
/// ```ignore
/// pragma solidity 0.5.11;
///
/// import "@openzeppelin/contracts-ethereum-package/contracts/math/SafeMath.sol";
///
/// contract MyContract {
///     using SafeMath for uint256;
///     ...
/// }
/// ```
///
/// When using `solc`, you have to specify the following:
///
/// - A `prefix`: the path that's used in your smart contract, i.e.
///   `@openzeppelin/contracts-ethereum-package`
/// - A `target`: the absolute path of the downloaded contracts on your computer
///
/// The format looks like this: `solc prefix=target ./MyContract.sol`
///
/// For example:
///
/// ```text
/// solc --bin \
///     @openzeppelin/contracts-ethereum-package=/Your/Absolute/Path/To/@openzeppelin/contracts-ethereum-package \
///     ./MyContract.sol
/// ```
///
/// You can also specify a `context` which limits the scope of the remapping to a subset of your
/// project. This allows you to apply the remapping only to imports located in a specific library or
/// a specific file. Without a context a remapping is applied to every matching import in all files.
///
/// The format is: `solc context:prefix=target ./MyContract.sol`
///
/// [Source](https://ethereum.stackexchange.com/questions/74448/what-are-remappings-and-how-do-they-work-in-solidity)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Remapping {
    pub context: Option<String>,
    pub name: String,
    pub path: String,
}

impl Remapping {
    /// Convenience function for [`RelativeRemapping::new`]
    pub fn into_relative(self, root: &Path) -> RelativeRemapping {
        RelativeRemapping::new(self, root)
    }

    /// Removes the `base` path from the remapping
    pub fn strip_prefix(&mut self, base: &Path) -> &mut Self {
        if let Ok(stripped) = Path::new(&self.path).strip_prefix(base) {
            self.path = stripped.display().to_string();
        }
        self
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, thiserror::Error)]
pub enum RemappingError {
    #[error("invalid remapping format, found `{0}`, expected `<key>=<value>`")]
    InvalidRemapping(String),
    #[error("remapping key can't be empty, found `{0}`, expected `<key>=<value>`")]
    EmptyRemappingKey(String),
    #[error("remapping value must be a path, found `{0}`, expected `<key>=<value>`")]
    EmptyRemappingValue(String),
}

impl FromStr for Remapping {
    type Err = RemappingError;

    fn from_str(remapping: &str) -> Result<Self, Self::Err> {
        let (name, path) = remapping
            .split_once('=')
            .ok_or_else(|| RemappingError::InvalidRemapping(remapping.to_string()))?;
        let (context, name) = name
            .split_once(':')
            .map_or((None, name), |(context, name)| (Some(context.to_string()), name));
        if name.trim().is_empty() {
            return Err(RemappingError::EmptyRemappingKey(remapping.to_string()));
        }
        if path.trim().is_empty() {
            return Err(RemappingError::EmptyRemappingValue(remapping.to_string()));
        }
        // if the remapping just starts with : (no context name), treat it as global
        let context = context.filter(|c| !c.trim().is_empty());
        Ok(Self { context, name: name.to_string(), path: path.to_string() })
    }
}

impl Serialize for Remapping {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Remapping {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let remapping = String::deserialize(deserializer)?;
        Self::from_str(&remapping).map_err(serde::de::Error::custom)
    }
}

// Remappings are printed as `prefix=target`
impl fmt::Display for Remapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        if let Some(context) = self.context.as_ref() {
            #[cfg(target_os = "windows")]
            {
                // ensure we have `/` slashes on windows
                use path_slash::PathExt;
                s.push_str(&std::path::Path::new(context).to_slash_lossy());
            }
            #[cfg(not(target_os = "windows"))]
            {
                s.push_str(context);
            }
            s.push(':');
        }
        let name =
            if !self.name.ends_with('/') { format!("{}/", self.name) } else { self.name.clone() };
        s.push_str(&{
            #[cfg(target_os = "windows")]
            {
                // ensure we have `/` slashes on windows
                use path_slash::PathExt;
                format!("{}={}", name, std::path::Path::new(&self.path).to_slash_lossy())
            }
            #[cfg(not(target_os = "windows"))]
            {
                format!("{}={}", name, self.path)
            }
        });

        if !s.ends_with('/') {
            s.push('/');
        }
        f.write_str(&s)
    }
}

impl Remapping {
    /// Converts any `\\` separators in the `path` to `/`.
    pub fn slash_path(&mut self) {
        #[cfg(windows)]
        {
            use path_slash::PathExt;
            self.path = Path::new(&self.path).to_slash_lossy().to_string();
            if let Some(context) = self.context.as_mut() {
                *context = Path::new(&context).to_slash_lossy().to_string();
            }
        }
    }
}

/// A relative [`Remapping`] that's aware of the current location
///
/// See [`RelativeRemappingPathBuf`]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelativeRemapping {
    pub context: Option<String>,
    pub name: String,
    pub path: RelativeRemappingPathBuf,
}

impl RelativeRemapping {
    /// Creates a new `RelativeRemapping` starting prefixed with `root`
    pub fn new(remapping: Remapping, root: &Path) -> Self {
        Self {
            context: remapping.context.map(|c| {
                RelativeRemappingPathBuf::with_root(root, c).path.to_string_lossy().to_string()
            }),
            name: remapping.name,
            path: RelativeRemappingPathBuf::with_root(root, remapping.path),
        }
    }

    /// Converts this relative remapping into an absolute remapping
    ///
    /// This sets to root of the remapping to the given `root` path
    pub fn to_remapping(mut self, root: PathBuf) -> Remapping {
        self.path.parent = Some(root);
        self.into()
    }

    /// Converts this relative remapping into [`Remapping`] without the root path
    pub fn to_relative_remapping(mut self) -> Remapping {
        self.path.parent.take();
        self.into()
    }
}

// Remappings are printed as `prefix=target`
impl fmt::Display for RelativeRemapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        if let Some(context) = self.context.as_ref() {
            #[cfg(target_os = "windows")]
            {
                // ensure we have `/` slashes on windows
                use path_slash::PathExt;
                s.push_str(&std::path::Path::new(context).to_slash_lossy());
            }
            #[cfg(not(target_os = "windows"))]
            {
                s.push_str(context);
            }
            s.push(':');
        }
        s.push_str(&{
            #[cfg(target_os = "windows")]
            {
                // ensure we have `/` slashes on windows
                use path_slash::PathExt;
                format!("{}={}", self.name, self.path.original().to_slash_lossy())
            }
            #[cfg(not(target_os = "windows"))]
            {
                format!("{}={}", self.name, self.path.original().display())
            }
        });

        if !s.ends_with('/') {
            s.push('/');
        }
        f.write_str(&s)
    }
}

impl From<RelativeRemapping> for Remapping {
    fn from(r: RelativeRemapping) -> Self {
        let RelativeRemapping { context, mut name, path } = r;
        let mut path = path.relative().display().to_string();
        if !path.ends_with('/') {
            path.push('/');
        }
        if !name.ends_with('/') {
            name.push('/');
        }
        Self { context, name, path }
    }
}

impl From<Remapping> for RelativeRemapping {
    fn from(r: Remapping) -> Self {
        Self { context: r.context, name: r.name, path: r.path.into() }
    }
}

/// The path part of the [`Remapping`] that knows the path of the file it was configured in, if any.
///
/// A [`Remapping`] is intended to be absolute, but paths in configuration files are often desired
/// to be relative to the configuration file itself. For example, a path of
/// `weird-erc20/=lib/weird-erc20/src/` configured in a file `/var/foundry.toml` might be desired to
/// resolve as a `weird-erc20/=/var/lib/weird-erc20/src/` remapping.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelativeRemappingPathBuf {
    pub parent: Option<PathBuf>,
    pub path: PathBuf,
}

impl RelativeRemappingPathBuf {
    /// Creates a new `RelativeRemappingPathBuf` that checks if the `path` is a child path of
    /// `parent`.
    pub fn with_root(
        parent: impl AsRef<Path> + Into<PathBuf>,
        path: impl AsRef<Path> + Into<PathBuf>,
    ) -> Self {
        if let Ok(path) = path.as_ref().strip_prefix(parent.as_ref()) {
            Self { parent: Some(parent.into()), path: path.to_path_buf() }
        } else if path.as_ref().has_root() {
            Self { parent: None, path: path.into() }
        } else {
            Self { parent: Some(parent.into()), path: path.into() }
        }
    }

    /// Returns the path as it was declared, without modification.
    pub fn original(&self) -> &Path {
        &self.path
    }

    /// Returns this path relative to the file it was declared in, if any.
    /// Returns the original if this path was not declared in a file or if the
    /// path has a root.
    pub fn relative(&self) -> PathBuf {
        if self.original().has_root() {
            return self.original().into();
        }
        self.parent
            .as_ref()
            .map(|p| p.join(self.original()))
            .unwrap_or_else(|| self.original().into())
    }
}

impl<P: Into<PathBuf>> From<P> for RelativeRemappingPathBuf {
    fn from(path: P) -> Self {
        Self { parent: None, path: path.into() }
    }
}

impl Serialize for RelativeRemapping {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RelativeRemapping {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let remapping = String::deserialize(deserializer)?;
        let remapping = Remapping::from_str(&remapping).map_err(serde::de::Error::custom)?;
        Ok(Self { context: remapping.context, name: remapping.name, path: remapping.path.into() })
    }
}

#[cfg(test)]
mod tests {
    pub use super::*;
    pub use similar_asserts::assert_eq;

    #[test]
    fn relative_remapping() {
        let remapping = "oz=a/b/c/d";
        let remapping = Remapping::from_str(remapping).unwrap();

        let relative = RelativeRemapping::new(remapping.clone(), Path::new("a/b/c"));
        assert_eq!(relative.path.relative(), Path::new(&remapping.path));
        assert_eq!(relative.path.original(), Path::new("d"));

        let relative = RelativeRemapping::new(remapping.clone(), Path::new("x/y"));
        assert_eq!(relative.path.relative(), Path::new("x/y/a/b/c/d"));
        assert_eq!(relative.path.original(), Path::new(&remapping.path));

        let remapping = "oz=/a/b/c/d";
        let remapping = Remapping::from_str(remapping).unwrap();
        let relative = RelativeRemapping::new(remapping.clone(), Path::new("a/b"));
        assert_eq!(relative.path.relative(), Path::new(&remapping.path));
        assert_eq!(relative.path.original(), Path::new(&remapping.path));
        assert!(relative.path.parent.is_none());

        let relative = RelativeRemapping::new(remapping, Path::new("/a/b"));
        assert_eq!(relative.to_relative_remapping(), Remapping::from_str("oz/=c/d/").unwrap());
    }

    #[test]
    fn remapping_errors() {
        let remapping = "oz=../b/c/d";
        let remapping = Remapping::from_str(remapping).unwrap();
        assert_eq!(remapping.name, "oz".to_string());
        assert_eq!(remapping.path, "../b/c/d".to_string());

        let err = Remapping::from_str("").unwrap_err();
        matches!(err, RemappingError::InvalidRemapping(_));

        let err = Remapping::from_str("oz=").unwrap_err();
        matches!(err, RemappingError::EmptyRemappingValue(_));
    }

    #[test]
    fn can_resolve_contexts() {
        let remapping = "context:oz=a/b/c/d";
        let remapping = Remapping::from_str(remapping).unwrap();

        assert_eq!(
            remapping,
            Remapping {
                context: Some("context".to_string()),
                name: "oz".to_string(),
                path: "a/b/c/d".to_string(),
            }
        );
        assert_eq!(remapping.to_string(), "context:oz/=a/b/c/d/".to_string());

        let remapping = "context:foo=C:/bar/src/";
        let remapping = Remapping::from_str(remapping).unwrap();

        assert_eq!(
            remapping,
            Remapping {
                context: Some("context".to_string()),
                name: "foo".to_string(),
                path: "C:/bar/src/".to_string()
            }
        );
    }

    #[test]
    fn can_resolve_global_contexts() {
        let remapping = ":oz=a/b/c/d/";
        let remapping = Remapping::from_str(remapping).unwrap();

        assert_eq!(
            remapping,
            Remapping { context: None, name: "oz".to_string(), path: "a/b/c/d/".to_string() }
        );
        assert_eq!(remapping.to_string(), "oz/=a/b/c/d/".to_string());
    }
}
