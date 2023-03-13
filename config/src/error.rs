//! error handling and solc error codes
use figment::providers::{Format, Toml};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashSet, error::Error, fmt, str::FromStr};

/// The message shown upon panic if the config could not be extracted from the figment
pub const FAILED_TO_EXTRACT_CONFIG_PANIC_MSG: &str = "failed to extract foundry config:";

/// Represents a failed attempt to extract `Config` from a `Figment`
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractConfigError {
    /// error thrown when extracting the `Config`
    pub(crate) error: figment::Error,
}

impl ExtractConfigError {
    /// Wraps the figment error
    pub fn new(error: figment::Error) -> Self {
        Self { error }
    }
}

impl fmt::Display for ExtractConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut unique_errors = Vec::with_capacity(self.error.count());
        let mut unique = HashSet::with_capacity(self.error.count());
        for err in self.error.clone().into_iter() {
            let err = if err
                .metadata
                .as_ref()
                .map(|meta| meta.name.contains(Toml::NAME))
                .unwrap_or_default()
            {
                FoundryConfigError::Toml(err)
            } else {
                FoundryConfigError::Other(err)
            };

            if unique.insert(err.to_string()) {
                unique_errors.push(err);
            }
        }
        writeln!(f, "{FAILED_TO_EXTRACT_CONFIG_PANIC_MSG}")?;
        for err in unique_errors {
            writeln!(f, "{err}")?;
        }
        Ok(())
    }
}

impl Error for ExtractConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Error::source(&self.error)
    }
}

/// Represents an error that can occur when constructing the `Config`
#[derive(Debug, Clone, PartialEq)]
pub enum FoundryConfigError {
    /// An error thrown during toml parsing
    Toml(figment::Error),
    /// Any other error thrown when constructing the config's figment
    Other(figment::Error),
}

impl fmt::Display for FoundryConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fmt_err = |err: &figment::Error, f: &mut fmt::Formatter<'_>| {
            write!(f, "{err}")?;
            if !err.path.is_empty() {
                // the path will contain the setting value like `["etherscan_api_key"]`
                write!(f, " for setting `{}`", err.path.join("."))?;
            }
            Ok(())
        };

        match self {
            FoundryConfigError::Toml(err) => {
                f.write_str("foundry.toml error: ")?;
                fmt_err(err, f)
            }
            FoundryConfigError::Other(err) => {
                f.write_str("foundry config error: ")?;
                fmt_err(err, f)
            }
        }
    }
}

impl Error for FoundryConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FoundryConfigError::Other(error) | FoundryConfigError::Toml(error) => {
                Error::source(error)
            }
        }
    }
}

/// A non-exhaustive list of solidity error codes
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SolidityErrorCode {
    /// Warning that SPDX license identifier not provided in source file
    SpdxLicenseNotProvided,
    /// Warning that contract code size exceeds 24576 bytes (a limit introduced in Spurious
    /// Dragon).
    ContractExceeds24576Bytes,
    /// Warning that Function state mutability can be restricted to [view,pure]
    FunctionStateMutabilityCanBeRestricted,
    /// Warning: Unused local variable
    UnusedLocalVariable,
    /// Warning: Unused function parameter. Remove or comment out the variable name to silence this
    /// warning.
    UnusedFunctionParameter,
    /// Warning: Return value of low-level calls not used.
    ReturnValueOfCallsNotUsed,
    ///  Warning: Interface functions are implicitly "virtual"
    InterfacesExplicitlyVirtual,
    /// Warning: This contract has a payable fallback function, but no receive ether function.
    /// Consider adding a receive ether function.
    PayableNoReceiveEther,
    ///  Warning: This declaration shadows an existing declaration.
    ShadowsExistingDeclaration,
    /// This declaration has the same name as another declaration.
    DeclarationSameNameAsAnother,
    /// Unnamed return variable can remain unassigned
    UnnamedReturnVariable,
    /// Unreachable code
    Unreachable,
    /// Missing pragma solidity
    PragmaSolidity,
    /// All other error codes
    Other(u64),
}

// === impl SolidityErrorCode ===

impl SolidityErrorCode {
    /// The textual identifier for this error
    ///
    /// Returns `Err(code)` if unknown error
    pub fn as_str(&self) -> Result<&'static str, u64> {
        let s = match self {
            SolidityErrorCode::SpdxLicenseNotProvided => "license",
            SolidityErrorCode::ContractExceeds24576Bytes => "code-size",
            SolidityErrorCode::FunctionStateMutabilityCanBeRestricted => "func-mutability",
            SolidityErrorCode::UnusedLocalVariable => "unused-var",
            SolidityErrorCode::UnusedFunctionParameter => "unused-param",
            SolidityErrorCode::ReturnValueOfCallsNotUsed => "unused-return",
            SolidityErrorCode::InterfacesExplicitlyVirtual => "virtual-interfaces",
            SolidityErrorCode::PayableNoReceiveEther => "missing-receive-ether",
            SolidityErrorCode::ShadowsExistingDeclaration => "shadowing",
            SolidityErrorCode::DeclarationSameNameAsAnother => "same-varname",
            SolidityErrorCode::UnnamedReturnVariable => "unnamed-return",
            SolidityErrorCode::Unreachable => "unreachable",
            SolidityErrorCode::PragmaSolidity => "pragma-solidity",
            SolidityErrorCode::Other(code) => return Err(*code),
        };
        Ok(s)
    }
}

impl From<SolidityErrorCode> for u64 {
    fn from(code: SolidityErrorCode) -> u64 {
        match code {
            SolidityErrorCode::SpdxLicenseNotProvided => 1878,
            SolidityErrorCode::ContractExceeds24576Bytes => 5574,
            SolidityErrorCode::FunctionStateMutabilityCanBeRestricted => 2018,
            SolidityErrorCode::UnusedLocalVariable => 2072,
            SolidityErrorCode::UnusedFunctionParameter => 5667,
            SolidityErrorCode::ReturnValueOfCallsNotUsed => 9302,
            SolidityErrorCode::InterfacesExplicitlyVirtual => 5815,
            SolidityErrorCode::PayableNoReceiveEther => 3628,
            SolidityErrorCode::ShadowsExistingDeclaration => 2519,
            SolidityErrorCode::DeclarationSameNameAsAnother => 8760,
            SolidityErrorCode::UnnamedReturnVariable => 6321,
            SolidityErrorCode::Unreachable => 5740,
            SolidityErrorCode::PragmaSolidity => 3420,
            SolidityErrorCode::Other(code) => code,
        }
    }
}

impl fmt::Display for SolidityErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Ok(name) => name.fmt(f),
            Err(code) => code.fmt(f),
        }
    }
}

impl FromStr for SolidityErrorCode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let code = match s {
            "unreachable" => SolidityErrorCode::Unreachable,
            "unused-return" => SolidityErrorCode::UnnamedReturnVariable,
            "unused-param" => SolidityErrorCode::UnusedFunctionParameter,
            "unused-var" => SolidityErrorCode::UnusedLocalVariable,
            "code-size" => SolidityErrorCode::ContractExceeds24576Bytes,
            "shadowing" => SolidityErrorCode::ShadowsExistingDeclaration,
            "func-mutability" => SolidityErrorCode::FunctionStateMutabilityCanBeRestricted,
            "license" => SolidityErrorCode::SpdxLicenseNotProvided,
            "pragma-solidity" => SolidityErrorCode::PragmaSolidity,
            "virtual-interfaces" => SolidityErrorCode::InterfacesExplicitlyVirtual,
            "missing-receive-ether" => SolidityErrorCode::PayableNoReceiveEther,
            "same-varname" => SolidityErrorCode::DeclarationSameNameAsAnother,
            _ => return Err(format!("Unknown variant {s}")),
        };

        Ok(code)
    }
}

impl From<u64> for SolidityErrorCode {
    fn from(code: u64) -> Self {
        match code {
            1878 => SolidityErrorCode::SpdxLicenseNotProvided,
            5574 => SolidityErrorCode::ContractExceeds24576Bytes,
            2018 => SolidityErrorCode::FunctionStateMutabilityCanBeRestricted,
            2072 => SolidityErrorCode::UnusedLocalVariable,
            5667 => SolidityErrorCode::UnusedFunctionParameter,
            9302 => SolidityErrorCode::ReturnValueOfCallsNotUsed,
            5815 => SolidityErrorCode::InterfacesExplicitlyVirtual,
            3628 => SolidityErrorCode::PayableNoReceiveEther,
            2519 => SolidityErrorCode::ShadowsExistingDeclaration,
            8760 => SolidityErrorCode::DeclarationSameNameAsAnother,
            6321 => SolidityErrorCode::UnnamedReturnVariable,
            3420 => SolidityErrorCode::PragmaSolidity,
            5740 => SolidityErrorCode::Unreachable,
            other => SolidityErrorCode::Other(other),
        }
    }
}

impl Serialize for SolidityErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.as_str() {
            Ok(alias) => serializer.serialize_str(alias),
            Err(code) => serializer.serialize_u64(code),
        }
    }
}

impl<'de> Deserialize<'de> for SolidityErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        /// Helper deserializer for error codes as names and codes
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum SolCode {
            Name(String),
            Code(u64),
        }

        match SolCode::deserialize(deserializer)? {
            SolCode::Code(code) => Ok(code.into()),
            SolCode::Name(name) => name.parse().map_err(serde::de::Error::custom),
        }
    }
}
