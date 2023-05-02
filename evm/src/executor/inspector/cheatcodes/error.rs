use crate::{
    error::SolError,
    executor::backend::{error::NoCheatcodeAccessError, DatabaseError},
};
use ethers::{
    abi::AbiEncode, prelude::k256::ecdsa::signature::Error as SignatureError, types::Bytes,
};
use foundry_common::errors::FsPathError;
use foundry_config::UnresolvedEnvVarError;
use std::{borrow::Cow, fmt::Arguments};

/// Type alias with a default Ok type of [`Bytes`], and default Err type of [`Error`].
pub type Result<T = Bytes, E = Error> = std::result::Result<T, E>;

macro_rules! err {
    ($msg:literal $(,)?) => {
        $crate::executor::inspector::cheatcodes::Error::fmt(::std::format_args!($msg))
    };
    ($err:expr $(,)?) => {
        <$crate::executor::inspector::cheatcodes::Error as ::std::convert::From<_>>::from($err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::executor::inspector::cheatcodes::Error::fmt(::std::format_args!($fmt, $($arg)*))
    };
}

macro_rules! bail {
    ($msg:literal $(,)?) => {
        return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::err!($msg))
    };
    ($err:expr $(,)?) => {
        return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::err!($err))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::err!($fmt, $($arg)*))
    };
}

macro_rules! ensure {
    ($cond:expr $(,)?) => {
        if !$cond {
            return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::Error::custom(
                ::std::concat!("Condition failed: `", ::std::stringify!($cond), "`")
            ));
        }
    };
    ($cond:expr, $msg:literal $(,)?) => {
        if !$cond {
            return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::err!($msg));
        }
    };
    ($cond:expr, $err:expr $(,)?) => {
        if !$cond {
            return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::err!($err));
        }
    };
    ($cond:expr, $fmt:expr, $($arg:tt)*) => {
        if !$cond {
            return ::std::result::Result::Err($crate::executor::inspector::cheatcodes::err!($fmt, $($arg)*));
        }
    };
}

pub(crate) use bail;
pub(crate) use ensure;
pub(crate) use err;

/// Errors that can happen when working with [`Cheacodes`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("You need to stop broadcasting before you can select forks.")]
    SelectForkDuringBroadcast,

    #[error(transparent)]
    Eyre(#[from] eyre::Report),

    #[error(transparent)]
    Signature(#[from] SignatureError),

    #[error(transparent)]
    Database(#[from] DatabaseError),

    #[error(transparent)]
    FsPath(#[from] FsPathError),

    #[error(transparent)]
    NoCheatcodeAccess(#[from] NoCheatcodeAccessError),

    #[error(transparent)]
    UnresolvedEnvVar(#[from] UnresolvedEnvVarError),

    #[error(transparent)]
    Abi(#[from] ethers::abi::Error),

    #[error(transparent)]
    Abi2(#[from] ethers::abi::AbiError),

    #[error(transparent)]
    Wallet(#[from] ethers::signers::WalletError),

    #[error(transparent)]
    EthersSignature(#[from] ethers::core::types::SignatureError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    JsonPath(#[from] jsonpath_lib::JsonPathError),

    #[error(transparent)]
    Hex(#[from] hex::FromHexError),

    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),

    #[error(transparent)]
    FromUtf8(#[from] std::string::FromUtf8Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    TryFromInt(#[from] std::num::TryFromIntError),

    /// Custom error.
    #[error("{0}")]
    Custom(Cow<'static, str>),

    /// Custom bytes. Will not get encoded with the error prefix.
    #[error("{}", hex::encode(_0))] // ignored in SolError implementation
    CustomBytes(Cow<'static, [u8]>),
}

impl Error {
    /// Creates a new error with a custom message.
    pub fn custom(msg: impl Into<Cow<'static, str>>) -> Self {
        Self::Custom(msg.into())
    }

    /// Creates a new error with a custom `fmt::Arguments` message.
    pub fn fmt(args: Arguments<'_>) -> Self {
        let cow = match args.as_str() {
            Some(s) => Cow::Borrowed(s),
            None => Cow::Owned(std::fmt::format(args)),
        };
        Self::Custom(cow)
    }

    /// Creates a new error with the given bytes.
    pub fn custom_bytes(bytes: impl Into<Cow<'static, [u8]>>) -> Self {
        Self::CustomBytes(bytes.into())
    }
}

impl From<Cow<'static, str>> for Error {
    fn from(value: Cow<'static, str>) -> Self {
        Self::Custom(value)
    }
}

impl From<String> for Error {
    fn from(value: String) -> Self {
        Self::Custom(value.into())
    }
}

impl From<&'static str> for Error {
    fn from(value: &'static str) -> Self {
        Self::Custom(value.into())
    }
}

impl SolError for Error {
    fn encode_error(&self) -> Bytes {
        match self {
            Self::CustomBytes(cow) => cow_to_bytes(cow),
            e => crate::error::encode_error(e),
        }
    }

    fn encode_string(&self) -> Bytes {
        match self {
            Self::CustomBytes(cow) => cow_to_bytes(cow),
            e => e.to_string().encode().into(),
        }
    }
}

fn cow_to_bytes(cow: &Cow<'static, [u8]>) -> Bytes {
    match cow {
        Cow::Borrowed(bytes) => Bytes::from_static(bytes),
        Cow::Owned(s) => Bytes(s.clone().into()),
    }
}
