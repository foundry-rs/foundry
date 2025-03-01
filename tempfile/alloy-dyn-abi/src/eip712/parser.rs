//! EIP-712 specific parsing structures.

// TODO: move to `sol-type-parser`

use crate::{
    eip712::resolver::{PropertyDef, TypeDef},
    Error,
};
use alloc::vec::Vec;
use parser::{Error as TypeParserError, TypeSpecifier};

/// A property is a type and a name. Of the form `type name`. E.g.
/// `uint256 foo` or `(MyStruct[23],bool) bar`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropDef<'a> {
    /// The prop type specifier.
    pub ty: TypeSpecifier<'a>,
    /// The prop name.
    pub name: &'a str,
}

impl PropDef<'_> {
    /// Convert to an owned `PropertyDef`
    pub fn to_owned(&self) -> PropertyDef {
        PropertyDef::new(self.ty.span, self.name).unwrap()
    }
}

impl<'a> TryFrom<&'a str> for PropDef<'a> {
    type Error = Error;

    #[inline]
    fn try_from(input: &'a str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

impl<'a> PropDef<'a> {
    /// Parse a string into property definition.
    pub fn parse(input: &'a str) -> Result<Self, Error> {
        let (ty, name) =
            input.rsplit_once(' ').ok_or_else(|| Error::invalid_property_def(input))?;
        Ok(PropDef { ty: ty.trim().try_into()?, name: name.trim() })
    }
}

/// Represents a single component type in an EIP-712 `encodeType` type string.
///
/// <https://eips.ethereum.org/EIPS/eip-712#definition-of-encodetype>
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentType<'a> {
    /// The span.
    pub span: &'a str,
    /// The name of the component type.
    pub type_name: &'a str,
    /// Properties of the component type.
    pub props: Vec<PropDef<'a>>,
}

impl<'a> TryFrom<&'a str> for ComponentType<'a> {
    type Error = Error;

    #[inline]
    fn try_from(input: &'a str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

impl<'a> ComponentType<'a> {
    /// Parse a string into a component type.
    pub fn parse(input: &'a str) -> Result<Self, Error> {
        let (name, props_str) = input
            .split_once('(')
            .ok_or_else(|| Error::TypeParser(TypeParserError::invalid_type_string(input)))?;

        let mut props = vec![];
        let mut depth = 1; // 1 to account for the ( in the split above
        let mut last = 0;

        for (i, c) in props_str.chars().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let candidate = &props_str[last..i];
                        if !candidate.is_empty() {
                            props.push(candidate.try_into()?);
                        }
                        last = i + 1;
                        break;
                    }
                }
                ',' => {
                    if depth == 1 {
                        props.push(props_str[last..i].try_into()?);
                        last = i + 1;
                    }
                }
                _ => {}
            }
        }

        Ok(Self { span: &input[..last + name.len() + 1], type_name: name, props })
    }

    /// Convert to an owned TypeDef.
    pub fn to_owned(&self) -> TypeDef {
        TypeDef::new(self.type_name, self.props.iter().map(|p| p.to_owned()).collect()).unwrap()
    }
}

/// Represents a list of component types in an EIP-712 `encodeType` type string.
#[derive(Debug, PartialEq, Eq)]
pub struct EncodeType<'a> {
    /// The list of component types.
    pub types: Vec<ComponentType<'a>>,
}

impl<'a> TryFrom<&'a str> for EncodeType<'a> {
    type Error = Error;

    #[inline]
    fn try_from(input: &'a str) -> Result<Self, Self::Error> {
        Self::parse(input)
    }
}

impl<'a> EncodeType<'a> {
    /// Parse a string into a list of component types.
    pub fn parse(input: &'a str) -> Result<Self, Error> {
        let mut types = vec![];
        let mut remaining = input;

        while let Ok(t) = ComponentType::parse(remaining) {
            remaining = &remaining[t.span.len()..];
            types.push(t);
        }

        Ok(Self { types })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "Transaction(Person from,Person to,Asset tx)Asset(address token,uint256 amount)Person(address wallet,string name)";

    #[test]
    fn empty_type() {
        let empty_domain_type =
            ComponentType { span: "EIP712Domain()", type_name: "EIP712Domain", props: vec![] };
        assert_eq!(ComponentType::parse("EIP712Domain()"), Ok(empty_domain_type.clone()));

        assert_eq!(
            EncodeType::try_from("EIP712Domain()"),
            Ok(EncodeType { types: vec![empty_domain_type] })
        );
    }

    #[test]
    fn test_component_type() {
        assert_eq!(
            ComponentType::parse("Transaction(Person from,Person to,Asset tx)"),
            Ok(ComponentType {
                span: "Transaction(Person from,Person to,Asset tx)",
                type_name: "Transaction",
                props: vec![
                    "Person from".try_into().unwrap(),
                    "Person to".try_into().unwrap(),
                    "Asset tx".try_into().unwrap(),
                ],
            })
        );
    }

    #[test]
    fn test_encode_type() {
        assert_eq!(
            EncodeType::parse(EXAMPLE),
            Ok(EncodeType {
                types: vec![
                    "Transaction(Person from,Person to,Asset tx)".try_into().unwrap(),
                    "Asset(address token,uint256 amount)".try_into().unwrap(),
                    "Person(address wallet,string name)".try_into().unwrap(),
                ]
            })
        );
    }
}
