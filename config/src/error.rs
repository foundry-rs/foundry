//! error handling and solc error codes
use figment::providers::{Format, Toml};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashSet, error::Error, fmt};

/// Represents a failed attempt to extract `Config` from a `Figment`
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractConfigError {
    /// error thrown when extracting the `Config`
    pub(crate) error: figment::Error,
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
        for err in unique_errors {
            writeln!(f, "{}", err)?;
        }
        f.write_str("failed to extract foundry config")
    }
}

impl Error for ExtractConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Error::source(&self.error)
    }
}

/// Represents an error that can occur when constructing the `Config`
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FoundryConfigError {
    /// An error thrown during toml parsing
    #[error("foundry.toml error: {0}")]
    Toml(figment::Error),
    /// Any other error thrown when constructing the config's figment
    #[error("foundry config error: {0}")]
    Other(figment::Error),
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
    /// All other error codes
    Other(u64),
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
            SolidityErrorCode::Other(code) => code,
        }
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
            other => SolidityErrorCode::Other(other),
        }
    }
}

impl Serialize for SolidityErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64((*self).into())
    }
}

impl<'de> Deserialize<'de> for SolidityErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        u64::deserialize(deserializer).map(Into::into)
    }
}
