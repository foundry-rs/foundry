//! Commonly used identifiers for contracts in the compiled output.

use std::{borrow::Cow, fmt, path::Path, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ParseContractInfoError(String);

/// Represents the common contract argument pattern for `<path>:<contractname>` where `<path>:` is
/// optional.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContractInfo {
    /// Location of the contract
    pub path: Option<String>,
    /// Name of the contract
    pub name: String,
}

// === impl ContractInfo ===

impl ContractInfo {
    /// Creates a new `ContractInfo` from the `info` str.
    ///
    /// This will attempt `ContractInfo::from_str`, if `info` matches the `<path>:<name>` format,
    /// the `ContractInfo`'s `path` will be set.
    ///
    /// otherwise the `name` of the new object will be `info`.
    ///
    /// # Examples
    ///
    /// ```
    /// use foundry_compilers::info::ContractInfo;
    ///
    /// let info = ContractInfo::new("src/Greeter.sol:Greeter");
    /// assert_eq!(
    ///     info,
    ///     ContractInfo { path: Some("src/Greeter.sol".to_string()), name: "Greeter".to_string() }
    /// );
    /// ```
    pub fn new(info: &str) -> Self {
        info.parse().unwrap_or_else(|_| Self { path: None, name: info.to_string() })
    }

    /// Returns the path to the contract source file if provided.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref().map(Path::new)
    }
}

impl fmt::Display for ContractInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path}:")?;
        }
        f.write_str(&self.name)
    }
}

impl FromStr for ContractInfo {
    type Err = ParseContractInfoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || {
            ParseContractInfoError(
                "contract source info format must be `<path>:<contractname>` or `<contractname>`"
                    .to_string(),
            )
        };
        let mut iter = s.rsplit(':');
        let name = iter.next().ok_or_else(err)?.trim().to_string();
        let path = iter.next().map(str::to_string);

        if name.ends_with(".sol") || name.contains('/') {
            return Err(err());
        }

        Ok(Self { path, name })
    }
}

impl From<FullContractInfo> for ContractInfo {
    fn from(info: FullContractInfo) -> Self {
        let FullContractInfo { path, name } = info;
        Self { path: Some(path), name }
    }
}

/// The reference type for `ContractInfo`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContractInfoRef<'a> {
    pub path: Option<Cow<'a, str>>,
    pub name: Cow<'a, str>,
}

impl From<ContractInfo> for ContractInfoRef<'_> {
    fn from(info: ContractInfo) -> Self {
        ContractInfoRef { path: info.path.map(Into::into), name: info.name.into() }
    }
}

impl<'a> From<&'a ContractInfo> for ContractInfoRef<'a> {
    fn from(info: &'a ContractInfo) -> Self {
        ContractInfoRef {
            path: info.path.as_deref().map(Into::into),
            name: info.name.as_str().into(),
        }
    }
}
impl From<FullContractInfo> for ContractInfoRef<'_> {
    fn from(info: FullContractInfo) -> Self {
        ContractInfoRef { path: Some(info.path.into()), name: info.name.into() }
    }
}

impl<'a> From<&'a FullContractInfo> for ContractInfoRef<'a> {
    fn from(info: &'a FullContractInfo) -> Self {
        ContractInfoRef { path: Some(info.path.as_str().into()), name: info.name.as_str().into() }
    }
}

/// Represents the common contract argument pattern `<path>:<contractname>`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FullContractInfo {
    /// Location of the contract
    pub path: String,
    /// Name of the contract
    pub name: String,
}

impl fmt::Display for FullContractInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path, self.name)
    }
}

impl FromStr for FullContractInfo {
    type Err = ParseContractInfoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (path, name) = s.split_once(':').ok_or_else(|| {
            ParseContractInfoError("Expected `<path>:<contractname>`, got `{s}`".to_string())
        })?;
        Ok(Self { path: path.to_string(), name: name.trim().to_string() })
    }
}

impl TryFrom<ContractInfo> for FullContractInfo {
    type Error = ParseContractInfoError;

    fn try_from(value: ContractInfo) -> Result<Self, Self::Error> {
        let ContractInfo { path, name } = value;
        Ok(Self {
            path: path.ok_or_else(|| {
                ParseContractInfoError("path to contract must be present".to_string())
            })?,
            name,
        })
    }
}
