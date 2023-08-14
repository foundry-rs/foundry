//! error handling and support

use ethers_core::{abi::AbiEncode, types::Bytes};
use std::fmt::Display;

/// Solidity revert prefix.
///
/// `keccak256("Error(String)")[..4] == 0x08c379a0`
pub const REVERT_PREFIX: [u8; 4] = [8, 195, 121, 160];

/// Custom Cheatcode error prefix.
///
/// `keccak256("CheatCodeError")[..4] == 0x0bc44503`
pub const ERROR_PREFIX: [u8; 4] = [11, 196, 69, 3];

/// An extension trait for `std::error::Error` that can abi-encode itself
pub trait SolError: std::error::Error {
    /// Returns the abi-encoded custom error
    ///
    /// Same as `encode_string` but prefixed with `ERROR_PREFIX`
    fn encode_error(&self) -> Bytes {
        encode_error(self)
    }

    /// Returns the error as abi-encoded String
    ///
    /// See also [`AbiEncode`](ethers::abi::AbiEncode)
    fn encode_string(&self) -> Bytes {
        self.to_string().encode().into()
    }
}

/// Encodes the given messages as solidity custom error
pub fn encode_error(reason: impl Display) -> Bytes {
    [ERROR_PREFIX.as_slice(), reason.to_string().encode().as_slice()].concat().into()
}
