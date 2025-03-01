use alloc::{boxed::Box, string::String};
use core::fmt;

/// Parser result
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Parser error.
#[derive(Clone, PartialEq, Eq)]
pub struct Error(Repr);

impl core::error::Error for Error {}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Error").field(&self.0 .0).finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error {
    /// Instantiate a new error.
    pub fn new(s: impl fmt::Display) -> Self {
        Self::_new("", &s)
    }

    /// Instantiate a new parser error.
    pub fn parser(e: impl fmt::Display) -> Self {
        Self::_new(if cfg!(feature = "std") { "parser error:\n" } else { "parser error: " }, &e)
    }

    /// Instantiate an invalid type string error. Invalid type string errors are
    /// for type strings that are not valid type strings. E.g. "uint256))))[".
    pub fn invalid_type_string(ty: impl fmt::Display) -> Self {
        Self::_new("invalid type string: ", &ty)
    }

    /// Instantiate an invalid identifier string error. Invalid identifier string errors are for
    /// identifier strings that do not follow the format described in
    /// <https://docs.soliditylang.org/en/latest/grammar.html#a4.SolidityLexer.Identifier>.
    pub fn invalid_identifier_string(identifier: impl fmt::Display) -> Self {
        Self::_new("invalid identifier string: ", &identifier)
    }

    /// Instantiate an invalid size error. Invalid size errors are for valid
    /// primitive types with invalid sizes. E.g. `"uint7"` or `"bytes1337"` or
    /// `"string[aaaaaa]"`.
    pub fn invalid_size(ty: impl fmt::Display) -> Self {
        Self::_new("invalid size for type: ", &ty)
    }

    // Not public API.
    #[doc(hidden)]
    #[inline(never)]
    #[cold]
    pub fn _new(s: &str, e: &dyn fmt::Display) -> Self {
        Self(Repr(Box::new(format!("{s}{e}"))))
    }
}

#[derive(Clone, PartialEq, Eq)]
#[allow(clippy::box_collection)] // `Box<String>` is smaller than `String` or `Box<str>`.
struct Repr(Box<String>);

impl fmt::Display for Repr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
