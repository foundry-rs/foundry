use alloc::string::{String, ToString};
use core::fmt;
use parser::TypeSpecifier;
use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};

/// The contract internal type. This could be a regular Solidity type, a
/// user-defined type, an enum, a struct, a contract, or an address payable.
///
/// The internal type represents the Solidity definition of the type, stripped
/// of the memory or storage keywords. It is used to convey the application dev
/// and user-facing type, while the json param "type" field is used to convey
/// the underlying ABI type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InternalType {
    /// Address payable.
    AddressPayable(String),
    /// Contract.
    Contract(String),
    /// Enum. Possibly of the form `contract.enum`.
    Enum {
        /// Contract qualifier, if any
        contract: Option<String>,
        /// Enum name
        ty: String,
    },
    /// Struct. Possibly of the form `contract.struct`.
    Struct {
        /// Contract qualifier, if any
        contract: Option<String>,
        /// Struct name
        ty: String,
    },
    /// Other. Possible of the form `contract.other`.
    Other {
        /// Contract qualifier, if any
        contract: Option<String>,
        /// Struct name
        ty: String,
    },
}

impl fmt::Display for InternalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_borrowed().fmt(f)
    }
}

impl Serialize for InternalType {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_borrowed().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InternalType {
    #[inline]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(ItVisitor)
    }
}

impl InternalType {
    /// Parse a string into an instance, taking ownership of data
    #[inline]
    pub fn parse(s: &str) -> Option<Self> {
        BorrowedInternalType::parse(s).map(BorrowedInternalType::into_owned)
    }

    /// True if the instance is a `struct` variant.
    #[inline]
    pub const fn is_struct(&self) -> bool {
        matches!(self, Self::Struct { .. })
    }

    /// True if the instance is a `enum` variant.
    #[inline]
    pub const fn is_enum(&self) -> bool {
        matches!(self, Self::Enum { .. })
    }

    /// True if the instance is a `contract` variant.
    #[inline]
    pub const fn is_contract(&self) -> bool {
        matches!(self, Self::Contract(_))
    }

    /// True if the instance is a `address payable` variant.
    #[inline]
    pub const fn is_address_payable(&self) -> bool {
        matches!(self, Self::AddressPayable(_))
    }

    /// True if the instance is a `other` variant.
    #[inline]
    pub const fn is_other(&self) -> bool {
        matches!(self, Self::Other { .. })
    }

    /// Fallible conversion to a variant.
    #[inline]
    pub fn as_struct(&self) -> Option<(Option<&str>, &str)> {
        match self {
            Self::Struct { contract, ty } => Some((contract.as_deref(), ty)),
            _ => None,
        }
    }

    /// Fallible conversion to a variant.
    #[inline]
    pub fn as_enum(&self) -> Option<(Option<&str>, &str)> {
        match self {
            Self::Enum { contract, ty } => Some((contract.as_deref(), ty)),
            _ => None,
        }
    }

    /// Fallible conversion to a variant.
    #[inline]
    pub fn as_contract(&self) -> Option<&str> {
        match self {
            Self::Contract(s) => Some(s),
            _ => None,
        }
    }

    /// Fallible conversion to a variant.
    #[inline]
    pub fn as_other(&self) -> Option<(Option<&str>, &str)> {
        match self {
            Self::Other { contract, ty } => Some((contract.as_deref(), ty)),
            _ => None,
        }
    }

    /// Return a [`TypeSpecifier`] describing the struct if this type is a
    /// struct.
    #[inline]
    pub fn struct_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.as_struct().and_then(|s| TypeSpecifier::parse(s.1).ok())
    }

    /// Return a [`TypeSpecifier`] describing the enum if this type is an enum.
    #[inline]
    pub fn enum_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.as_enum().and_then(|s| TypeSpecifier::parse(s.1).ok())
    }

    /// Return a [`TypeSpecifier`] describing the contract if this type is a
    /// contract.
    #[inline]
    pub fn contract_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.as_contract().and_then(|s| TypeSpecifier::parse(s).ok())
    }

    /// Return a [`TypeSpecifier`] describing the other if this type is an
    /// other. An "other" specifier indicates EITHER a regular Solidity type OR
    /// a user-defined type. It is not possible to distinguish between the two
    /// without additional context.
    #[inline]
    pub fn other_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.as_other().and_then(|s| TypeSpecifier::parse(s.1).ok())
    }

    #[inline]
    pub(crate) fn as_borrowed(&self) -> BorrowedInternalType<'_> {
        match self {
            Self::AddressPayable(s) => BorrowedInternalType::AddressPayable(s),
            Self::Contract(s) => BorrowedInternalType::Contract(s),
            Self::Enum { contract, ty } => {
                BorrowedInternalType::Enum { contract: contract.as_deref(), ty }
            }
            Self::Struct { contract, ty } => {
                BorrowedInternalType::Struct { contract: contract.as_deref(), ty }
            }
            Self::Other { contract, ty } => {
                BorrowedInternalType::Other { contract: contract.as_deref(), ty }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum BorrowedInternalType<'a> {
    AddressPayable(&'a str),
    Contract(&'a str),
    Enum { contract: Option<&'a str>, ty: &'a str },
    Struct { contract: Option<&'a str>, ty: &'a str },
    Other { contract: Option<&'a str>, ty: &'a str },
}

impl fmt::Display for BorrowedInternalType<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::AddressPayable(s) => f.write_str(s),
            Self::Contract(s) => {
                f.write_str("contract ")?;
                f.write_str(s)
            }
            Self::Enum { contract, ty }
            | Self::Struct { contract, ty }
            | Self::Other { contract, ty } => {
                match self {
                    Self::Enum { .. } => f.write_str("enum ")?,
                    Self::Struct { .. } => f.write_str("struct ")?,
                    Self::Other { .. } => {}
                    _ => unreachable!(),
                }
                if let Some(c) = contract {
                    f.write_str(c)?;
                    f.write_str(".")?;
                }
                f.write_str(ty)
            }
        }
    }
}

impl Serialize for BorrowedInternalType<'_> {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for BorrowedInternalType<'a> {
    #[inline]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(BorrowedItVisitor)
    }
}

impl<'a> BorrowedInternalType<'a> {
    /// Instantiate a borrowed internal type by parsing a string.
    fn parse(v: &'a str) -> Option<Self> {
        if v.starts_with("address payable") {
            return Some(Self::AddressPayable(v));
        }
        if let Some(body) = v.strip_prefix("enum ") {
            if let Some((contract, ty)) = body.split_once('.') {
                Some(Self::Enum { contract: Some(contract), ty })
            } else {
                Some(Self::Enum { contract: None, ty: body })
            }
        } else if let Some(body) = v.strip_prefix("struct ") {
            if let Some((contract, ty)) = body.split_once('.') {
                Some(Self::Struct { contract: Some(contract), ty })
            } else {
                Some(Self::Struct { contract: None, ty: body })
            }
        } else if let Some(body) = v.strip_prefix("contract ") {
            Some(Self::Contract(body))
        } else if let Some((contract, ty)) = v.split_once('.') {
            Some(Self::Other { contract: Some(contract), ty })
        } else {
            Some(Self::Other { contract: None, ty: v })
        }
    }

    pub(crate) fn into_owned(self) -> InternalType {
        match self {
            Self::AddressPayable(s) => InternalType::AddressPayable(s.to_string()),
            Self::Contract(s) => InternalType::Contract(s.to_string()),
            Self::Enum { contract, ty } => {
                InternalType::Enum { contract: contract.map(String::from), ty: ty.to_string() }
            }
            Self::Struct { contract, ty } => {
                InternalType::Struct { contract: contract.map(String::from), ty: ty.to_string() }
            }
            Self::Other { contract, ty } => {
                InternalType::Other { contract: contract.map(String::from), ty: ty.to_string() }
            }
        }
    }
}

const VISITOR_EXPECTED: &str = "a valid internal type";

pub(crate) struct ItVisitor;

impl Visitor<'_> for ItVisitor {
    type Value = InternalType;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(VISITOR_EXPECTED)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        BorrowedInternalType::parse(v)
            .map(BorrowedInternalType::into_owned)
            .ok_or_else(|| E::invalid_value(serde::de::Unexpected::Str(v), &VISITOR_EXPECTED))
    }
}

const BORROWED_VISITOR_EXPECTED: &str = "a valid borrowed internal type";

pub(crate) struct BorrowedItVisitor;

impl<'de> Visitor<'de> for BorrowedItVisitor {
    type Value = BorrowedInternalType<'de>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(BORROWED_VISITOR_EXPECTED)
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
        BorrowedInternalType::parse(v).ok_or_else(|| {
            E::invalid_value(serde::de::Unexpected::Str(v), &BORROWED_VISITOR_EXPECTED)
        })
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Err(E::invalid_value(serde::de::Unexpected::Str(v), &BORROWED_VISITOR_EXPECTED))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    macro_rules! parser_test {
        ($test_str:expr, $expected:expr) => {
            assert_eq!(InternalType::parse($test_str).unwrap(), $expected);
        };
    }

    #[test]
    fn parse_simple_internal_types() {
        parser_test!(
            "struct SpentItem[]",
            InternalType::Struct { contract: None, ty: "SpentItem[]".into() }
        );
        parser_test!(
            "struct Contract.Item",
            InternalType::Struct { contract: Some("Contract".into()), ty: "Item".into() }
        );
        parser_test!(
            "enum ItemType[32]",
            InternalType::Enum { contract: None, ty: "ItemType[32]".into() }
        );
        parser_test!(
            "enum Contract.Item",
            InternalType::Enum { contract: Some("Contract".into()), ty: "Item".into() }
        );

        parser_test!("contract Item", InternalType::Contract("Item".into()));
        parser_test!("contract Item[]", InternalType::Contract("Item[]".into()));
        parser_test!("contract Item[][2]", InternalType::Contract("Item[][2]".into()));
        parser_test!("contract Item[][2][]", InternalType::Contract("Item[][2][]".into()));

        parser_test!(
            "address payable",
            InternalType::AddressPayable("address payable".to_string())
        );
        parser_test!(
            "address payable[][][][][]",
            InternalType::AddressPayable("address payable[][][][][]".into())
        );
        parser_test!("Item", InternalType::Other { contract: None, ty: "Item".into() });
        parser_test!(
            "Contract.Item[][33]",
            InternalType::Other { contract: Some("Contract".into()), ty: "Item[][33]".into() }
        );
    }
}
