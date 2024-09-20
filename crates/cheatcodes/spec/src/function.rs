use serde::{Deserialize, Serialize};
use std::fmt;

/// Solidity function.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Function<'a> {
    /// The function's unique identifier. This is the function name, optionally appended with an
    /// index if it is overloaded.
    pub id: &'a str,
    /// The description of the function.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
    /// The Solidity function declaration, including full type and parameter names, visibility,
    /// etc.
    pub declaration: &'a str,
    /// The Solidity function visibility attribute. This is currently always `external`, but this
    /// may change in the future.
    pub visibility: Visibility,
    /// The Solidity function state mutability attribute.
    pub mutability: Mutability,
    /// The standard function signature used to calculate `selector`.
    /// See the [Solidity docs] for more information.
    ///
    /// [Solidity docs]: https://docs.soliditylang.org/en/latest/abi-spec.html#function-selector
    pub signature: &'a str,
    /// The hex-encoded, "0x"-prefixed 4-byte function selector,
    /// which is the Keccak-256 hash of `signature`.
    pub selector: &'a str,
    /// The 4-byte function selector as a byte array.
    pub selector_bytes: [u8; 4],
}

impl fmt::Display for Function<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.declaration)
    }
}

/// Solidity function visibility attribute. See the [Solidity docs] for more information.
///
/// [Solidity docs]: https://docs.soliditylang.org/en/latest/contracts.html#function-visibility
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    /// The function is only visible externally.
    External,
    /// Visible externally and internally.
    Public,
    /// Only visible internally.
    Internal,
    /// Only visible in the current contract
    Private,
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Visibility {
    /// Returns the string representation of the visibility.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Private => "private",
        }
    }
}

/// Solidity function state mutability attribute. See the [Solidity docs] for more information.
///
/// [Solidity docs]: https://docs.soliditylang.org/en/latest/contracts.html#state-mutability
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Mutability {
    /// Disallows modification or access of state.
    Pure,
    /// Disallows modification of state.
    View,
    /// Allows modification of state.
    #[serde(rename = "")]
    None,
}

impl fmt::Display for Mutability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Mutability {
    /// Returns the string representation of the mutability.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::View => "view",
            Self::None => "",
        }
    }
}
