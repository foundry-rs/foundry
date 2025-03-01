use crate::{Error, Input, Result, RootType, TupleSpecifier};
use winnow::{combinator::trace, ModalResult, Parser};

/// A stem of a Solidity array type. It is either a root type, or a tuple type.
///
/// # Examples
///
/// ```
/// # use alloy_sol_type_parser::{TypeStem, RootType, TupleSpecifier};
/// let stem = TypeStem::parse("uint256")?;
/// assert_eq!(stem.span(), "uint256");
/// assert!(matches!(stem, TypeStem::Root(_)));
/// assert_eq!(stem.as_root(), Some(&RootType::parse("uint256").unwrap()));
///
/// let stem = TypeStem::parse("(uint256,bool)")?;
/// assert_eq!(stem.span(), "(uint256,bool)");
/// assert!(matches!(stem, TypeStem::Tuple(_)));
/// assert_eq!(stem.as_tuple(), Some(&TupleSpecifier::parse("(uint256,bool)").unwrap()));
/// # Ok::<_, alloy_sol_type_parser::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeStem<'a> {
    /// Root type.
    Root(RootType<'a>),
    /// Tuple type.
    Tuple(TupleSpecifier<'a>),
}

impl<'a> TryFrom<&'a str> for TypeStem<'a> {
    type Error = Error;

    #[inline]
    fn try_from(value: &'a str) -> Result<Self> {
        Self::parse(value)
    }
}

impl AsRef<str> for TypeStem<'_> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.span()
    }
}

impl<'a> TypeStem<'a> {
    /// Parse a type stem from a string.
    #[inline]
    pub fn parse(input: &'a str) -> Result<Self> {
        if input.starts_with('(') || input.starts_with("tuple(") {
            input.try_into().map(Self::Tuple)
        } else {
            input.try_into().map(Self::Root)
        }
    }

    /// [`winnow`] parser for this type.
    pub(crate) fn parser(input: &mut Input<'a>) -> ModalResult<Self> {
        let name = "TypeStem";
        if input.starts_with('(') || input.starts_with("tuple(") {
            trace(name, TupleSpecifier::parser).parse_next(input).map(Self::Tuple)
        } else {
            trace(name, RootType::parser).parse_next(input).map(Self::Root)
        }
    }

    /// Fallible conversion to a root type
    #[inline]
    pub const fn as_root(&self) -> Option<&RootType<'a>> {
        match self {
            Self::Root(root) => Some(root),
            Self::Tuple(_) => None,
        }
    }

    /// Fallible conversion to a tuple type
    #[inline]
    pub const fn as_tuple(&self) -> Option<&TupleSpecifier<'a>> {
        match self {
            Self::Root(_) => None,
            Self::Tuple(tuple) => Some(tuple),
        }
    }

    /// Returns the type stem as a string.
    #[inline]
    pub const fn span(&self) -> &'a str {
        match self {
            Self::Root(root) => root.span(),
            Self::Tuple(tuple) => tuple.span(),
        }
    }

    /// Returns true if the type is a basic Solidity type.
    #[inline]
    pub fn try_basic_solidity(&self) -> Result<()> {
        match self {
            Self::Root(root) => root.try_basic_solidity(),
            Self::Tuple(tuple) => tuple.try_basic_solidity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple() {
        // empty tuple
        assert_eq!(
            TypeStem::parse("()"),
            Ok(TypeStem::Tuple(TupleSpecifier { span: "()", types: vec![] }))
        );
        TypeStem::parse("tuple(").unwrap_err();
        assert_eq!(
            TypeStem::parse("tuple()"),
            Ok(TypeStem::Tuple(TupleSpecifier { span: "tuple()", types: vec![] }))
        );

        // type named tuple
        assert_eq!(TypeStem::parse("tuple"), Ok(TypeStem::Root(RootType::parse("tuple").unwrap())))
    }
}
