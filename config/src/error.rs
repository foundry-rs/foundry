use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
