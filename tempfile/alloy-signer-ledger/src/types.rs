//! Helpers for interacting with the Ethereum Ledger App.
//!
//! [Official Docs](https://github.com/LedgerHQ/app-ethereum/blob/master/doc/ethapp.adoc)

#![allow(clippy::upper_case_acronyms)]

use alloy_primitives::hex;
use std::fmt;
use thiserror::Error;

#[derive(Clone, Debug)]
/// Ledger wallet type
pub enum DerivationType {
    /// Ledger Live-generated HD path
    LedgerLive(usize),
    /// Legacy generated HD Path
    Legacy(usize),
    /// Any other path
    Other(String),
}

impl fmt::Display for DerivationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::Legacy(index) => write!(f, "m/44'/60'/0'/{index}"),
            Self::LedgerLive(index) => write!(f, "m/44'/60'/{index}'/0/0"),
            Self::Other(inner) => f.write_str(inner),
        }
    }
}

/// Error when using the Ledger transport.
#[derive(Debug, Error)]
pub enum LedgerError {
    /// Underlying Ledger transport error.
    #[error(transparent)]
    LedgerError(#[from] coins_ledger::errors::LedgerError),
    /// Device response was unexpectedly empty.
    #[error("received an unexpected empty response")]
    UnexpectedNullResponse,
    /// [`hex`](mod@hex) error.
    #[error(transparent)]
    HexError(#[from] hex::FromHexError),
    /// [`semver`] error.
    #[error(transparent)]
    SemVerError(#[from] semver::Error),
    /// Signature Error
    #[error(transparent)]
    SignatureError(#[from] alloy_primitives::SignatureError),
    /// Thrown when trying to sign using EIP-712 with an incompatible Ledger Ethereum app.
    #[error("Ledger Ethereum app requires at least version {0}")]
    UnsupportedAppVersion(&'static str),
    /// Got a response, but it didn't contain as much data as expected
    #[error("bad response; got {got} bytes, expected {expected}")]
    ShortResponse {
        /// Number of bytes received.
        got: usize,
        /// Number of bytes expected.
        expected: usize,
    },
}

pub(crate) const P1_FIRST: u8 = 0x00;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types, dead_code)]
pub(crate) enum INS {
    GET_PUBLIC_KEY = 0x02,
    SIGN = 0x04,
    GET_APP_CONFIGURATION = 0x06,
    SIGN_PERSONAL_MESSAGE = 0x08,
    SIGN_ETH_EIP_712 = 0x0C,
}

impl fmt::Display for INS {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GET_PUBLIC_KEY => write!(f, "GET_PUBLIC_KEY"),
            Self::SIGN => write!(f, "SIGN"),
            Self::GET_APP_CONFIGURATION => write!(f, "GET_APP_CONFIGURATION"),
            Self::SIGN_PERSONAL_MESSAGE => write!(f, "SIGN_PERSONAL_MESSAGE"),
            Self::SIGN_ETH_EIP_712 => write!(f, "SIGN_ETH_EIP_712"),
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub(crate) enum P1 {
    NON_CONFIRM = 0x00,
    MORE = 0x80,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub(crate) enum P2 {
    NO_CHAINCODE = 0x00,
}
