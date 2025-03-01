use crate::{ident::identifier_parser, is_valid_identifier, new_input, Error, Input, Result};
use core::fmt;
use winnow::{combinator::trace, stream::Stream, ModalResult, Parser};

/// A root type, with no array suffixes. Corresponds to a single, non-sequence
/// type. This is the most basic type specifier.
///
/// Note that this type might modify the input string, so [`span()`](Self::span)
/// must not be assumed to be the same as the input string.
///
/// # Examples
///
/// ```
/// # use alloy_sol_type_parser::RootType;
/// let root_type = RootType::parse("uint256")?;
/// assert_eq!(root_type.span(), "uint256");
///
/// // Allows unknown types
/// assert_eq!(RootType::parse("MyStruct")?.span(), "MyStruct");
///
/// // No sequences
/// assert!(RootType::parse("uint256[2]").is_err());
///
/// // No tuples
/// assert!(RootType::parse("(uint256,uint256)").is_err());
///
/// // Input string might get modified
/// assert_eq!(RootType::parse("uint")?.span(), "uint256");
/// # Ok::<_, alloy_sol_type_parser::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootType<'a>(&'a str);

impl<'a> TryFrom<&'a str> for RootType<'a> {
    type Error = Error;

    #[inline]
    fn try_from(value: &'a str) -> Result<Self> {
        Self::parse(value)
    }
}

impl AsRef<str> for RootType<'_> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl fmt::Display for RootType<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl<'a> RootType<'a> {
    /// Create a new root type from a string without checking if it's valid.
    ///
    /// # Safety
    ///
    /// The string passed in must be a valid Solidity identifier. See
    /// [`is_valid_identifier`].
    pub const unsafe fn new_unchecked(s: &'a str) -> Self {
        debug_assert!(is_valid_identifier(s));
        Self(s)
    }

    /// Parse a root type from a string.
    #[inline]
    pub fn parse(input: &'a str) -> Result<Self> {
        Self::parser.parse(new_input(input)).map_err(Error::parser)
    }

    /// [`winnow`] parser for this type.
    pub(crate) fn parser(input: &mut Input<'a>) -> ModalResult<Self> {
        trace("RootType", |input: &mut Input<'a>| {
            identifier_parser(input).map(|ident| {
                // Workaround for enums in library function params or returns.
                // See: https://github.com/alloy-rs/core/pull/386
                // See ethabi workaround: https://github.com/rust-ethereum/ethabi/blob/b1710adc18f5b771d2d2519c87248b1ba9430778/ethabi/src/param_type/reader.rs#L162-L167
                if input.starts_with('.') {
                    let _ = input.next_token();
                    let _ = identifier_parser(input);
                    return Self("uint8");
                }

                // Normalize the `u?int` aliases to the canonical `u?int256`
                match ident {
                    "uint" => Self("uint256"),
                    "int" => Self("int256"),
                    _ => Self(ident),
                }
            })
        })
        .parse_next(input)
    }

    /// The string underlying this type. The type name.
    #[inline]
    pub const fn span(self) -> &'a str {
        self.0
    }

    /// Returns `Ok(())` if the type is a basic Solidity type.
    #[inline]
    pub fn try_basic_solidity(self) -> Result<()> {
        match self.0 {
            "address" | "bool" | "string" | "bytes" | "uint" | "int" | "function" => Ok(()),
            name => {
                if let Some(sz) = name.strip_prefix("bytes") {
                    if let Ok(sz) = sz.parse::<usize>() {
                        if sz != 0 && sz <= 32 {
                            return Ok(());
                        }
                    }
                    return Err(Error::invalid_size(name));
                }

                // fast path both integer types
                let s = name.strip_prefix('u').unwrap_or(name);

                if let Some(sz) = s.strip_prefix("int") {
                    if let Ok(sz) = sz.parse::<usize>() {
                        if sz != 0 && sz <= 256 && sz % 8 == 0 {
                            return Ok(());
                        }
                    }
                    return Err(Error::invalid_size(name));
                }

                Err(Error::invalid_type_string(name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_input() {
        assert_eq!(RootType::parse("Contract.Enum"), Ok(RootType("uint8")));

        assert_eq!(RootType::parse("int"), Ok(RootType("int256")));
        assert_eq!(RootType::parse("uint"), Ok(RootType("uint256")));
    }
}
