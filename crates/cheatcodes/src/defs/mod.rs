//! Cheatcode definitions.

use alloy_sol_types::SolCall;
use serde::{Deserialize, Serialize};

mod vm;
pub use vm::Vm;

/// Cheatcode definition trait. Implemented by all [`Vm`] functions.
pub trait CheatcodeDef: std::fmt::Debug + Clone + SolCall {
    /// The static cheatcode definition.
    const CHEATCODE: &'static Cheatcode<'static>;
}

/// Specification of a single cheatcode.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[non_exhaustive]
pub struct Cheatcode<'a> {
    // Automatically-generated fields.
    /// The cheatcode's unique identifier. This is the function name, optionally appended with an
    /// index if it is overloaded.
    pub id: &'a str,
    /// The Solidity function declaration string, including full type and parameter names,
    /// visibility, etc.
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
    /// The description of the cheatcode.
    /// This is a markdown string derived from the documentation of the function declaration.
    pub description: &'a str,

    // Manually-specified fields.
    /// The group that the cheatcode belongs to.
    pub group: Group,
    /// The current status of the cheatcode. E.g. whether it is stable or experimental, etc.
    pub status: Status,
    /// Whether the cheatcode is safe to use inside of scripts. E.g. it does not change state in an
    /// unexpected way.
    pub safety: Safety,
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

/// The status of a cheatcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Status {
    /// The cheatcode and its API is currently stable.
    Stable,
    /// The cheatcode is unstable, meaning it may contain bugs and may break its API on any
    /// release.
    ///
    /// Use of experimental cheatcodes will result in a warning.
    Experimental,
    /// The cheatcode has been deprecated, meaning it will be removed in a future release.
    ///
    /// Use of deprecated cheatcodes is discouraged and will result in a warning.
    Deprecated,
    /// The cheatcode has been removed and is no longer available for use.
    ///
    /// Use of removed cheatcodes will result in a hard error.
    Removed,
}

/// Cheatcode groups.
/// Initially derived and modified from inline comments in [`forge-std`'s `Vm.sol`][vmsol].
///
/// [vmsol]: https://github.com/foundry-rs/forge-std/blob/dcb0d52bc4399d37a6545848e3b8f9d03c77b98d/src/Vm.sol
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Group {
    /// Cheatcodes that read from, or write to the current EVM execution state.
    ///
    /// Examples: any of the `record` cheatcodes, `chainId`, `coinbase`.
    ///
    /// Safety: ambiguous, depends on whether the cheatcode is read-only or not.
    Evm,
    /// Cheatcodes that interact with how a test is run.
    ///
    /// Examples: `assume`, `skip`, `expectRevert`.
    ///
    /// Safety: ambiguous, depends on whether the cheatcode is read-only or not.
    Testing,
    /// Cheatcodes that interact with how a script is run.
    ///
    /// Examples: `broadcast`, `startBroadcast`, `stopBroadcast`.
    ///
    /// Safety: safe.
    Scripting,
    /// Cheatcodes that interact with the OS or filesystem.
    ///
    /// Examples: `ffi`, `projectRoot`, `writeFile`.
    ///
    /// Safety: safe.
    Filesystem,
    /// Cheatcodes that interact with the program's environment variables.
    ///
    /// Examples: `setEnv`, `envBool`, `envOr`.
    ///
    /// Safety: safe.
    Environment,
    /// Utility cheatcodes that deal with string parsing and manipulation.
    ///
    /// Examples: `toString`. `parseBytes`.
    ///
    /// Safety: safe.
    String,
    /// Utility cheatcodes that deal with parsing values from and converting values to JSON.
    ///
    /// Examples: `serializeJson`, `parseJsonUint`, `writeJson`.
    ///
    /// Safety: safe.
    Json,
    /// Generic, uncategorized utilities.
    ///
    /// Examples: `toString`, `parse*`, `serialize*`.
    ///
    /// Safety: safe.
    Utilities,
}

impl Group {
    /// Returns the safety of this cheatcode group.
    ///
    /// Some groups are inherently safe or unsafe, while others are ambiguous and will return
    /// `None`.
    #[inline]
    pub const fn safety(self) -> Option<Safety> {
        match self {
            Self::Evm | Self::Testing => None,
            Self::Scripting |
            Self::Filesystem |
            Self::Environment |
            Self::String |
            Self::Json |
            Self::Utilities => Some(Safety::Safe),
        }
    }

    /// Returns this value as a string.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Evm => "evm",
            Self::Testing => "testing",
            Self::Scripting => "scripting",
            Self::Filesystem => "filesystem",
            Self::Environment => "environment",
            Self::String => "string",
            Self::Json => "json",
            Self::Utilities => "utilities",
        }
    }
}

// TODO: Find a better name for this
/// Cheatcode safety.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Safety {
    /// The cheatcode is not safe to use in scripts.
    Unsafe,
    /// The cheatcode is safe to use in scripts.
    #[default]
    Safe,
}

impl Safety {
    /// Returns this value as a string.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Unsafe => "unsafe",
        }
    }

    /// Returns whether this value is safe.
    #[inline]
    pub const fn is_safe(self) -> bool {
        matches!(self, Self::Safe)
    }
}
