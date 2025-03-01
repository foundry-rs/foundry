use crate::{
    new_input,
    utils::{opt_ws_ident, spanned, tuple_parser},
    Error, Input, Result, TypeSpecifier,
};
use alloc::vec::Vec;
use core::fmt;
use winnow::{combinator::trace, ModalResult, Parser};

// TODO: Parse visibility and state mutability

/// Represents a function parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterSpecifier<'a> {
    /// The full span of the specifier.
    pub span: &'a str,
    /// The type of the parameter.
    pub ty: TypeSpecifier<'a>,
    /// The storage specifier.
    pub storage: Option<Storage>,
    /// Whether the parameter indexed.
    pub indexed: bool,
    /// The name of the parameter.
    pub name: Option<&'a str>,
}

impl<'a> TryFrom<&'a str> for ParameterSpecifier<'a> {
    type Error = Error;

    #[inline]
    fn try_from(value: &'a str) -> Result<Self> {
        Self::parse(value)
    }
}

impl<'a> ParameterSpecifier<'a> {
    /// Parse a parameter from a string.
    #[inline]
    pub fn parse(input: &'a str) -> Result<Self> {
        Self::parser.parse(new_input(input)).map_err(Error::parser)
    }

    /// [`winnow`] parser for this type.
    pub(crate) fn parser(input: &mut Input<'a>) -> ModalResult<Self> {
        trace(
            "ParameterSpecifier",
            spanned(|input: &mut Input<'a>| {
                let ty = TypeSpecifier::parser(input)?;
                let mut name = opt_ws_ident(input)?;

                let mut storage = None;
                if let Some(kw @ ("storage" | "memory" | "calldata")) = name {
                    storage = match kw {
                        "storage" => Some(Storage::Storage),
                        "memory" => Some(Storage::Memory),
                        "calldata" => Some(Storage::Calldata),
                        _ => unreachable!(),
                    };
                    name = opt_ws_ident(input)?;
                }

                let mut indexed = false;
                if let Some("indexed") = name {
                    indexed = true;
                    name = opt_ws_ident(input)?;
                }
                Ok((ty, storage, indexed, name))
            }),
        )
        .parse_next(input)
        .map(|(span, (ty, storage, indexed, name))| Self {
            span,
            ty,
            storage,
            indexed,
            name,
        })
    }
}

/// Represents a list of function parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parameters<'a> {
    /// The full span of the specifier.
    pub span: &'a str,
    /// The parameters.
    pub params: Vec<ParameterSpecifier<'a>>,
}

impl<'a> TryFrom<&'a str> for Parameters<'a> {
    type Error = Error;

    #[inline]
    fn try_from(value: &'a str) -> Result<Self> {
        Self::parse(value)
    }
}

impl<'a> Parameters<'a> {
    /// Parse a parameter list from a string.
    #[inline]
    pub fn parse(input: &'a str) -> Result<Self> {
        Self::parser.parse(new_input(input)).map_err(Error::parser)
    }

    /// [`winnow`] parser for this type.
    pub(crate) fn parser(input: &mut Input<'a>) -> ModalResult<Self> {
        trace("Parameters", spanned(tuple_parser(ParameterSpecifier::parser)))
            .parse_next(input)
            .map(|(span, params)| Self { span, params })
    }
}

/// Storage specifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Storage {
    /// `memory`
    Memory,
    /// `storage`
    Storage,
    /// `calldata`
    Calldata,
}

impl core::str::FromStr for Storage {
    type Err = Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl fmt::Display for Storage {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Storage {
    /// Parse a string storage specifier.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "memory" => Ok(Self::Memory),
            "storage" => Ok(Self::Storage),
            "calldata" => Ok(Self::Calldata),
            s => Err(Error::_new("invalid storage specifier: ", &s)),
        }
    }

    /// Returns a string representation of the storage specifier.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Storage => "storage",
            Self::Calldata => "calldata",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_param() {
        assert_eq!(
            ParameterSpecifier::parse("bool name"),
            Ok(ParameterSpecifier {
                span: "bool name",
                ty: TypeSpecifier::parse("bool").unwrap(),
                storage: None,
                indexed: false,
                name: Some("name"),
            })
        );

        assert_eq!(
            ParameterSpecifier::parse("bool indexed name"),
            Ok(ParameterSpecifier {
                span: "bool indexed name",
                ty: TypeSpecifier::parse("bool").unwrap(),
                storage: None,
                indexed: true,
                name: Some("name"),
            })
        );

        assert_eq!(
            ParameterSpecifier::parse("bool2    indexed \t name"),
            Ok(ParameterSpecifier {
                span: "bool2    indexed \t name",
                ty: TypeSpecifier::parse("bool2").unwrap(),
                storage: None,
                indexed: true,
                name: Some("name"),
            })
        );

        ParameterSpecifier::parse("a b ").unwrap_err();
        ParameterSpecifier::parse(" a b ").unwrap_err();
        ParameterSpecifier::parse(" a b").unwrap_err();
    }

    #[test]
    fn parse_params() {
        assert_eq!(Parameters::parse("()"), Ok(Parameters { span: "()", params: vec![] }));
        assert_eq!(Parameters::parse("( )"), Ok(Parameters { span: "( )", params: vec![] }));
        assert_eq!(Parameters::parse("(  )"), Ok(Parameters { span: "(  )", params: vec![] }));
        assert_eq!(Parameters::parse("(   )"), Ok(Parameters { span: "(   )", params: vec![] }));

        assert_eq!(
            Parameters::parse("(\tuint256   , \t)"),
            Ok(Parameters {
                span: "(\tuint256   , \t)",
                params: vec![ParameterSpecifier {
                    span: "uint256   ",
                    ty: TypeSpecifier::parse("uint256").unwrap(),
                    storage: None,
                    indexed: false,
                    name: None,
                }]
            })
        );
        assert_eq!(
            Parameters::parse("( \t uint256 \ta,\t bool b, \t)"),
            Ok(Parameters {
                span: "( \t uint256 \ta,\t bool b, \t)",
                params: vec![
                    ParameterSpecifier {
                        span: "uint256 \ta",
                        ty: TypeSpecifier::parse("uint256").unwrap(),
                        storage: None,
                        indexed: false,
                        name: Some("a"),
                    },
                    ParameterSpecifier {
                        span: "bool b",
                        ty: TypeSpecifier::parse("bool").unwrap(),
                        storage: None,
                        indexed: false,
                        name: Some("b"),
                    }
                ]
            })
        );
    }

    #[test]
    fn parse_storage() {
        assert_eq!(
            ParameterSpecifier::parse("foo storag"),
            Ok(ParameterSpecifier {
                span: "foo storag",
                ty: TypeSpecifier::parse("foo").unwrap(),
                storage: None,
                indexed: false,
                name: Some("storag")
            })
        );
        assert_eq!(
            ParameterSpecifier::parse("foo storage"),
            Ok(ParameterSpecifier {
                span: "foo storage",
                ty: TypeSpecifier::parse("foo").unwrap(),
                storage: Some(Storage::Storage),
                indexed: false,
                name: None
            })
        );
        assert_eq!(
            ParameterSpecifier::parse("foo storage bar"),
            Ok(ParameterSpecifier {
                span: "foo storage bar",
                ty: TypeSpecifier::parse("foo").unwrap(),
                storage: Some(Storage::Storage),
                indexed: false,
                name: "bar".into()
            })
        );
        assert_eq!(
            ParameterSpecifier::parse("foo memory bar"),
            Ok(ParameterSpecifier {
                span: "foo memory bar",
                ty: TypeSpecifier::parse("foo").unwrap(),
                storage: Some(Storage::Memory),
                indexed: false,
                name: "bar".into()
            })
        );
        assert_eq!(
            ParameterSpecifier::parse("foo calldata bar"),
            Ok(ParameterSpecifier {
                span: "foo calldata bar",
                ty: TypeSpecifier::parse("foo").unwrap(),
                storage: Some(Storage::Calldata),
                indexed: false,
                name: "bar".into()
            })
        );
        ParameterSpecifier::parse("foo storag bar").unwrap_err();
    }
}
