//! error handling and support

use bytes::Bytes;
use ethers::{abi::AbiEncode, utils::keccak256};
use once_cell::sync::Lazy;
use std::fmt::Display;

// keccak(Error(string))
pub static REVERT_PREFIX: [u8; 4] = [8, 195, 121, 160];

/// Custom error prefiix
pub static ERROR_PREFIX: Lazy<[u8; 32]> = Lazy::new(|| keccak256("CheatCodeError"));

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
