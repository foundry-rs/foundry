//! error handling and support

use alloy_primitives::Bytes;
use alloy_sol_types::{SolError, SolValue};

/// Solidity revert prefix.
///
/// `keccak256("Error(string)")[..4] == 0x08c379a0`
pub const REVERT_PREFIX: [u8; 4] = [8, 195, 121, 160];

/// Custom Cheatcode error prefix.
///
/// `keccak256("CheatcodeError(string)")[..4] == 0xeeaa9e6f`
pub const ERROR_PREFIX: [u8; 4] = [238, 170, 158, 111];

/// An extension trait for `std::error::Error` that can ABI-encode itself.
pub trait ErrorExt: std::error::Error {
    /// ABI-encodes the error using `Revert(string)`.
    fn encode_error(&self) -> Bytes;

    /// ABI-encodes the error as a string.
    fn encode_string(&self) -> Bytes;
}

impl<T: std::error::Error> ErrorExt for T {
    fn encode_error(&self) -> Bytes {
        alloy_sol_types::Revert::from(self.to_string()).abi_encode().into()
    }

    fn encode_string(&self) -> Bytes {
        self.to_string().abi_encode().into()
    }
}
