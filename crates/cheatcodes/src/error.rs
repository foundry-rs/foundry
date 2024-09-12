use crate::Vm;
use alloy_primitives::{hex, Address, Bytes};
use alloy_signer::Error as SignerError;
use alloy_signer_local::LocalSignerError;
use alloy_sol_types::SolError;
use foundry_common::errors::FsPathError;
use foundry_config::UnresolvedEnvVarError;
use foundry_evm_core::backend::{BackendError, DatabaseError};
use foundry_wallets::error::WalletSignerError;
use k256::ecdsa::signature::Error as SignatureError;
use revm::primitives::EVMError;
use std::{borrow::Cow, fmt};

/// Cheatcode result type.
///
/// Type alias with a default Ok type of [`Vec<u8>`], and default Err type of [`Error`].
pub type Result<T = Vec<u8>, E = Error> = std::result::Result<T, E>;

macro_rules! fmt_err {
    ($msg:literal $(,)?) => {
        $crate::Error::fmt(::std::format_args!($msg))
    };
    ($err:expr $(,)?) => {
        <$crate::Error as ::std::convert::From<_>>::from($err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::Error::fmt(::std::format_args!($fmt, $($arg)*))
    };
}

macro_rules! bail {
    ($msg:literal $(,)?) => {
        return ::std::result::Result::Err(fmt_err!($msg))
    };
    ($err:expr $(,)?) => {
        return ::std::result::Result::Err(fmt_err!($err))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return ::std::result::Result::Err(fmt_err!($fmt, $($arg)*))
    };
}

macro_rules! ensure {
    ($cond:expr $(,)?) => {
        if !$cond {
            return ::std::result::Result::Err($crate::Error::custom(
                ::std::concat!("Condition failed: `", ::std::stringify!($cond), "`")
            ));
        }
    };
    ($cond:expr, $msg:literal $(,)?) => {
        if !$cond {
            return ::std::result::Result::Err(fmt_err!($msg));
        }
    };
    ($cond:expr, $err:expr $(,)?) => {
        if !$cond {
            return ::std::result::Result::Err(fmt_err!($err));
        }
    };
    ($cond:expr, $fmt:expr, $($arg:tt)*) => {
        if !$cond {
            return ::std::result::Result::Err(fmt_err!($fmt, $($arg)*));
        }
    };
}

macro_rules! ensure_not_precompile {
    ($address:expr, $ctxt:expr) => {
        if $ctxt.is_precompile($address) {
            return Err($crate::error::precompile_error($address));
        }
    };
}

#[cold]
pub(crate) fn precompile_error(address: &Address) -> Error {
    fmt_err!("cannot use precompile {address} as an argument")
}

/// Error thrown by cheatcodes.
// This uses a custom repr to minimize the size of the error.
// The repr is basically `enum { Cow<'static, str>, Cow<'static, [u8]> }`
pub struct Error {
    /// If true, encode `data` as `Error(string)`, otherwise encode it directly as `bytes`.
    is_str: bool,
    /// Whether this was constructed from an owned byte vec, which means we have to drop the data
    /// in `impl Drop`.
    drop: bool,
    /// The error data. Always a valid pointer, and never modified.
    data: *const [u8],
}

impl std::error::Error for Error {}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Error::")?;
        self.kind().fmt(f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

/// Kind of cheatcode errors.
///
/// Constructed by [`Error::kind`].
#[derive(Debug)]
pub enum ErrorKind<'a> {
    /// A string error, ABI-encoded as `CheatcodeError(string)`.
    String(&'a str),
    /// A raw bytes error. Does not get encoded.
    Bytes(&'a [u8]),
}

impl fmt::Display for ErrorKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::String(ss) => f.write_str(ss),
            Self::Bytes(b) => f.write_str(&hex::encode_prefixed(b)),
        }
    }
}

impl Error {
    /// Creates a new error and ABI encodes it as `CheatcodeError(string)`.
    pub fn encode(error: impl Into<Self>) -> Bytes {
        error.into().abi_encode().into()
    }

    /// Creates a new error with a custom message.
    pub fn display(msg: impl fmt::Display) -> Self {
        Self::fmt(format_args!("{msg}"))
    }

    /// Creates a new error with a custom [`fmt::Arguments`] message.
    pub fn fmt(args: fmt::Arguments<'_>) -> Self {
        match args.as_str() {
            Some(s) => Self::new_str(s),
            None => Self::new_string(std::fmt::format(args)),
        }
    }

    /// ABI-encodes this error as `CheatcodeError(string)` if the inner message is a string,
    /// otherwise returns the raw bytes.
    pub fn abi_encode(&self) -> Vec<u8> {
        match self.kind() {
            ErrorKind::String(string) => Vm::CheatcodeError { message: string.into() }.abi_encode(),
            ErrorKind::Bytes(bytes) => bytes.into(),
        }
    }

    /// Returns the kind of this error.
    #[inline]
    pub fn kind(&self) -> ErrorKind<'_> {
        let data = self.data();
        if self.is_str {
            debug_assert!(std::str::from_utf8(data).is_ok());
            ErrorKind::String(unsafe { std::str::from_utf8_unchecked(data) })
        } else {
            ErrorKind::Bytes(data)
        }
    }

    /// Returns the raw data of this error.
    #[inline]
    pub fn data(&self) -> &[u8] {
        unsafe { &*self.data }
    }

    /// Returns `true` if this error is a human-readable string.
    #[inline]
    pub fn is_str(&self) -> bool {
        self.is_str
    }

    #[inline]
    fn new_str(data: &'static str) -> Self {
        Self::_new(true, false, data.as_bytes())
    }

    #[inline]
    fn new_string(data: String) -> Self {
        Self::_new(true, true, Box::into_raw(data.into_boxed_str().into_boxed_bytes()))
    }

    #[inline]
    fn new_bytes(data: &'static [u8]) -> Self {
        Self::_new(false, false, data)
    }

    #[inline]
    fn new_vec(data: Vec<u8>) -> Self {
        Self::_new(false, true, Box::into_raw(data.into_boxed_slice()))
    }

    #[inline]
    fn _new(is_str: bool, drop: bool, data: *const [u8]) -> Self {
        debug_assert!(!data.is_null());
        Self { is_str, drop, data }
    }
}

impl Drop for Error {
    #[inline]
    fn drop(&mut self) {
        if self.drop {
            drop(unsafe { Box::<[u8]>::from_raw(self.data.cast_mut()) });
        }
    }
}

impl From<Cow<'static, str>> for Error {
    fn from(value: Cow<'static, str>) -> Self {
        match value {
            Cow::Borrowed(str) => Self::new_str(str),
            Cow::Owned(string) => Self::new_string(string),
        }
    }
}

impl From<String> for Error {
    #[inline]
    fn from(value: String) -> Self {
        Self::new_string(value)
    }
}

impl From<&'static str> for Error {
    #[inline]
    fn from(value: &'static str) -> Self {
        Self::new_str(value)
    }
}

impl From<Cow<'static, [u8]>> for Error {
    #[inline]
    fn from(value: Cow<'static, [u8]>) -> Self {
        match value {
            Cow::Borrowed(bytes) => Self::new_bytes(bytes),
            Cow::Owned(vec) => Self::new_vec(vec),
        }
    }
}

impl From<&'static [u8]> for Error {
    #[inline]
    fn from(value: &'static [u8]) -> Self {
        Self::new_bytes(value)
    }
}

impl<const N: usize> From<&'static [u8; N]> for Error {
    #[inline]
    fn from(value: &'static [u8; N]) -> Self {
        Self::new_bytes(value)
    }
}

impl From<Vec<u8>> for Error {
    #[inline]
    fn from(value: Vec<u8>) -> Self {
        Self::new_vec(value)
    }
}

impl From<Bytes> for Error {
    #[inline]
    fn from(value: Bytes) -> Self {
        Self::new_vec(value.into())
    }
}

// So we can use `?` on `Result<_, Error>`.
macro_rules! impl_from {
    ($($t:ty),* $(,)?) => {$(
        impl From<$t> for Error {
            #[inline]
            fn from(value: $t) -> Self {
                Self::display(value)
            }
        }
    )*};
}

impl_from!(
    alloy_sol_types::Error,
    alloy_dyn_abi::Error,
    alloy_primitives::SignatureError,
    FsPathError,
    hex::FromHexError,
    eyre::Error,
    BackendError,
    DatabaseError,
    jsonpath_lib::JsonPathError,
    serde_json::Error,
    SignatureError,
    std::io::Error,
    std::num::TryFromIntError,
    std::str::Utf8Error,
    std::string::FromUtf8Error,
    UnresolvedEnvVarError,
    LocalSignerError,
    SignerError,
    WalletSignerError,
);

impl<T: Into<BackendError>> From<EVMError<T>> for Error {
    #[inline]
    fn from(err: EVMError<T>) -> Self {
        Self::display(BackendError::from(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode() {
        let error = Vm::CheatcodeError { message: "hello".into() }.abi_encode();
        assert_eq!(Error::from("hello").abi_encode(), error);
        assert_eq!(Error::encode("hello"), error);

        assert_eq!(Error::from(b"hello").abi_encode(), b"hello");
        assert_eq!(Error::encode(b"hello"), b"hello"[..]);
    }
}
