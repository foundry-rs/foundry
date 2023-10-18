use alloy_primitives::{Address, Bytes};
use alloy_sol_types::{Revert, SolError, SolValue};
use ethers::{core::k256::ecdsa::signature::Error as SignatureError, signers::WalletError};
use foundry_common::errors::FsPathError;
use foundry_config::UnresolvedEnvVarError;
use std::{borrow::Cow, fmt, ptr::NonNull};

/// Cheatcode result type.
///
/// Type alias with a default Ok type of [`Vec<u8>`], and default Err type of [`Error`].
pub type Result<T = Vec<u8>, E = Error> = std::result::Result<T, E>;

macro_rules! fmt_err {
    ($msg:literal $(,)?) => {
        $crate::impls::Error::fmt(::std::format_args!($msg))
    };
    ($err:expr $(,)?) => {
        <$crate::impls::Error as ::std::convert::From<_>>::from($err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::impls::Error::fmt(::std::format_args!($fmt, $($arg)*))
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
            return ::std::result::Result::Err($crate::impls::Error::custom(
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
            return Err($crate::impls::error::precompile_error(
                <Self as $crate::CheatcodeDef>::CHEATCODE.id,
                $address,
            ))
        }
    };
}

#[cold]
pub(crate) fn precompile_error(id: &'static str, address: &Address) -> Error {
    fmt_err!("cannot call {id} on precompile {address}")
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
    data: NonNull<[u8]>,
}

impl std::error::Error for Error {}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    /// A string error, encoded as `Error(string)`.
    String(&'a str),
    /// A bytes error, encoded directly as just `bytes`.
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
    /// Creates a new error and ABI encodes it.
    pub fn encode(error: impl Into<Self>) -> Bytes {
        error.into().abi_encode().into()
    }

    /// Creates a new error with a custom message.
    pub fn display(msg: impl fmt::Display) -> Self {
        Self::fmt(format_args!("{msg}"))
    }

    /// Creates a new error with a custom `fmt::Arguments` message.
    pub fn fmt(args: fmt::Arguments<'_>) -> Self {
        match args.as_str() {
            Some(s) => Self::new_str(s),
            None => Self::new_string(std::fmt::format(args)),
        }
    }

    /// ABI-encodes this error.
    pub fn abi_encode(&self) -> Vec<u8> {
        match self.kind() {
            ErrorKind::String(string) => Revert::from(string).abi_encode(),
            ErrorKind::Bytes(bytes) => bytes.abi_encode(),
        }
    }

    /// ABI-encodes this error as `bytes`.
    pub fn abi_encode_bytes(&self) -> Vec<u8> {
        self.data().abi_encode()
    }

    /// Returns the kind of this error.
    #[inline(always)]
    pub fn kind(&self) -> ErrorKind<'_> {
        let data = self.data();
        if self.is_str {
            ErrorKind::String(unsafe { std::str::from_utf8_unchecked(data) })
        } else {
            ErrorKind::Bytes(data)
        }
    }

    /// Returns the raw data of this error.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        unsafe { &*self.data.as_ptr() }
    }

    #[inline(always)]
    fn new_str(data: &'static str) -> Self {
        Self::new(true, false, data.as_bytes() as *const [u8] as *mut [u8])
    }

    #[inline(always)]
    fn new_string(data: String) -> Self {
        Self::new(true, true, Box::into_raw(data.into_boxed_str().into_boxed_bytes()))
    }

    #[inline(always)]
    fn new_bytes(data: &'static [u8]) -> Self {
        Self::new(false, false, data as *const [u8] as *mut [u8])
    }

    #[inline(always)]
    fn new_vec(data: Vec<u8>) -> Self {
        Self::new(false, true, Box::into_raw(data.into_boxed_slice()))
    }

    #[inline(always)]
    fn new(is_str: bool, drop: bool, data: *mut [u8]) -> Self {
        Self { is_str, drop, data: unsafe { NonNull::new_unchecked(data) } }
    }
}

impl Drop for Error {
    #[inline]
    fn drop(&mut self) {
        if self.drop {
            drop(unsafe { Box::from_raw(self.data.as_ptr()) });
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
    ethers::types::SignatureError,
    FsPathError,
    hex::FromHexError,
    eyre::Error,
    super::db::DatabaseError,
    jsonpath_lib::JsonPathError,
    serde_json::Error,
    SignatureError,
    std::io::Error,
    std::num::TryFromIntError,
    std::str::Utf8Error,
    std::string::FromUtf8Error,
    UnresolvedEnvVarError,
    WalletError,
);
