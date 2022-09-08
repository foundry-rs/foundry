//! error handling and support

use bytes::Bytes;
use ethers::{
    abi::{self, Abi, AbiDecode, AbiEncode, AbiError, ParamType},
    prelude::U256,
    utils::keccak256,
};
use foundry_common::SELECTOR_LEN;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

/// The selector of `keccak(Error(string))` used by Solidity string reverts
pub const SOLIDITY_REVERT_SELECTOR: [u8; 4] = [8, 195, 121, 160];

/// The selector of a Solidity panic
pub const SOLIDITY_PANIC_SELECTOR: [u8; 4] = [78, 72, 123, 113];

/// The selector of a cheatcode-related error (`CheatCodeError(string error, string[] hints)`)
pub static CHEATCODE_ERROR_SELECTOR: Lazy<[u8; 32]> =
    Lazy::new(|| keccak256("CheatCodeError(string,string[])"));

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

/// Encodes the given message as a Solidity custom error
pub fn encode_error(reason: impl Display) -> Bytes {
    encode_error_with_hints(reason, Vec::new())
}

/// Encodes the given message and hints as a Solidity custom error
pub fn encode_error_with_hints(reason: impl Display, hints: Vec<String>) -> Bytes {
    [
        CHEATCODE_ERROR_SELECTOR.as_slice(),
        reason.to_string().encode().as_slice(),
        hints.encode().as_slice(),
    ]
    .concat()
    .into()
}

/// A Solidity panic.
#[derive(Debug)]
pub enum SolidityPanic {
    /// 0x00: Generic compiler inserted panics.
    Generic,
    /// 0x01: If you call `assert` with an argument that evaluates to false
    Assert,
    /// 0x11: If an arithmetic operation results in underflow or overflow outside of an `unchecked
    /// { ... }` block.
    OverUnderFlow,
    /// 0x12: Divide by zero.
    DivideByZero,
    /// 0x21: If you convert a value that is too big or negative into an enum type.
    InvalidEnumCast,
    /// 0x22: If you access a storage byte array that is incorrectly encoded.
    InvalidStorageByteArray,
    /// 0x31: If you call `.pop()` on an empty array
    PopOnEmptyArray,
    /// 0x32: If you access an array, `bytesN` or an array slice at an out-of-bounds or negative
    /// index.
    IndexOutOfBounds,
    /// 0x41: If you allocate too much memory or create an array that is too large.
    Alloc,
    /// 0x51: If you call a zero-initialized variable of internal function type.
    InvalidPointer,
    /// An unknown Solidity panic.
    Unknown(usize),
}

impl AbiDecode for SolidityPanic {
    fn decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiError> {
        if !bytes.as_ref().starts_with(&SOLIDITY_PANIC_SELECTOR) {
            Err(AbiError::WrongSelector)
        } else {
            let code = U256::decode(&bytes.as_ref()[SELECTOR_LEN..])?.as_usize();

            Ok(match code {
                0x00 => SolidityPanic::Generic,
                0x01 => SolidityPanic::Assert,
                0x11 => SolidityPanic::OverUnderFlow,
                0x12 => SolidityPanic::DivideByZero,
                0x21 => SolidityPanic::InvalidEnumCast,
                0x22 => SolidityPanic::InvalidStorageByteArray,
                0x31 => SolidityPanic::PopOnEmptyArray,
                0x32 => SolidityPanic::IndexOutOfBounds,
                0x41 => SolidityPanic::Alloc,
                0x51 => SolidityPanic::InvalidPointer,
                code => SolidityPanic::Unknown(code),
            })
        }
    }
}

impl Display for SolidityPanic {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                SolidityPanic::Generic => "Generic compiler panic encountered",
                SolidityPanic::Assert => "Assertion violated",
                SolidityPanic::OverUnderFlow => "Arithmetic over/underflow",
                SolidityPanic::DivideByZero => "Division or modulo by zero",
                SolidityPanic::InvalidEnumCast => "Conversion into non-existent enum variant",
                SolidityPanic::InvalidStorageByteArray => "Incorrectly encoded byte storage array",
                SolidityPanic::PopOnEmptyArray => "Called pop on an empty array",
                SolidityPanic::IndexOutOfBounds => "Index out of bounds",
                SolidityPanic::Alloc => "Memory allocation overflow",
                SolidityPanic::InvalidPointer =>
                    "Called a zero-initialized variable of type internal function",
                SolidityPanic::Unknown(_) => "Unknown Solidity panic encountered",
            }
        )
    }
}

/// A Solidity string revert (e.g. from `require(cond, msg)`)
#[derive(Debug)]
pub struct StringRevert(pub String);

impl AbiDecode for StringRevert {
    fn decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiError> {
        if !bytes.as_ref().starts_with(&SOLIDITY_REVERT_SELECTOR) {
            Err(AbiError::WrongSelector)
        } else {
            Ok(StringRevert(String::decode(&bytes.as_ref()[SELECTOR_LEN..])?))
        }
    }
}

impl Display for StringRevert {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A wrapper around a decoded error with potential hints.
///
/// Using the [AbiDecode] trait it is possible to decode:
///
/// - Solidity panic codes
/// - Solidity string reverts
/// - Forge-specific cheatcode errors
///
/// It is also possible to decode custom user-defined errors using [DecodedError::decode_with_abi].
// TODO: Does this name kind of suck?
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct DecodedError {
    pub message: String,
    pub hints: Vec<String>,
}

impl AbiDecode for DecodedError {
    fn decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiError> {
        SolidityPanic::decode(&bytes)
            .map(Into::into)
            .or_else(|_| StringRevert::decode(&bytes).map(Into::into))
            .or_else(|_| String::decode(&bytes).map(Into::into))
            .or_else(|_| {
                // Try decoding cheatcode errors
                if !bytes.as_ref().starts_with(CHEATCODE_ERROR_SELECTOR.as_ref()) {
                    Err(AbiError::WrongSelector)
                } else {
                    let mut tokens = abi::decode(
                        &[ParamType::String, ParamType::Array(Box::new(ParamType::String))],
                        bytes.as_ref(),
                    )?
                    .into_iter();

                    let message = tokens
                        .next()
                        .and_then(|token| token.into_string())
                        .ok_or(AbiError::DecodingError(ethers::abi::Error::InvalidData))?;
                    let hints = tokens
                        .next()
                        .and_then(|token| token.into_array())
                        .ok_or(AbiError::DecodingError(ethers::abi::Error::InvalidData))?
                        .into_iter()
                        .map(|hint| {
                            hint.into_string()
                                .ok_or(AbiError::DecodingError(ethers::abi::Error::InvalidData))
                        })
                        .collect::<Result<Vec<String>, AbiError>>()?;
                    Ok(DecodedError { message, hints })
                }
            })
            .or_else(|_| {
                // Try decoding unknown errors
                if bytes.as_ref().len() < SELECTOR_LEN {
                    return Err(AbiError::WrongSelector)
                }

                Ok(DecodedError::from(format!(
                    "{}:{}",
                    hex::encode(&bytes.as_ref()[..SELECTOR_LEN]),
                    String::decode(&bytes.as_ref()[SELECTOR_LEN..])?
                )))
            })
    }
}

impl<T> From<T> for DecodedError
where
    T: Display,
{
    fn from(message: T) -> Self {
        DecodedError { message: message.to_string(), hints: Vec::new() }
    }
}

impl DecodedError {
    /// Decode an error from some return data, accounting for custom errors defined in an ABI
    pub fn decode_with_abi(data: &[u8], abi: &Abi) -> Result<Self, AbiError> {
        if data.len() < SELECTOR_LEN {
            return Err(AbiError::WrongSelector)
        }

        let error =
            abi.errors().find(|error| error.signature()[..SELECTOR_LEN] == data[..SELECTOR_LEN]);
        if let Some(error) = error {
            if let Ok(decoded) = error.decode(&data[SELECTOR_LEN..]) {
                let inputs =
                    decoded.iter().map(foundry_utils::format_token).collect::<Vec<_>>().join(", ");
                return Ok(DecodedError::from(format!("{}({})", error.name, inputs)))
            }
        }

        DecodedError::decode(data)
    }
}
