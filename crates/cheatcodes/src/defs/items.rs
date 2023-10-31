use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt};

/// A Solidity custom error.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Error<'a> {
    /// The name of the error.
    pub name: &'a str,
    /// The description of the error.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
    /// The Solidity error declaration, including full type, parameter names, etc.
    pub declaration: &'a str,
}

impl fmt::Display for Error<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.declaration)
    }
}

/// A Solidity event.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Event<'a> {
    /// The name of the event.
    pub name: &'a str,
    /// The description of the event.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
    /// The Solidity event declaration, including full type, parameter names, etc.
    pub declaration: &'a str,
}

impl fmt::Display for Event<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.declaration)
    }
}

/// A Solidity enumeration.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Enum<'a> {
    /// The name of the enum.
    pub name: &'a str,
    /// The description of the enum.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
    /// The variants of the enum.
    #[serde(borrow)]
    pub variants: Cow<'a, [EnumVariant<'a>]>,
}

impl fmt::Display for Enum<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "enum {} {{ ", self.name)?;
        for (i, variant) in self.variants.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(variant.name)?;
        }
        f.write_str(" }")
    }
}

/// A variant of an [`Enum`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct EnumVariant<'a> {
    /// The name of the variant.
    pub name: &'a str,
    /// The description of the variant.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
}

/// A Solidity struct.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Struct<'a> {
    /// The name of the struct.
    pub name: &'a str,
    /// The description of the struct.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
    /// The fields of the struct.
    #[serde(borrow)]
    pub fields: Cow<'a, [StructField<'a>]>,
}

impl fmt::Display for Struct<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct {} {{ ", self.name)?;
        for field in self.fields.iter() {
            write!(f, "{} {}; ", field.ty, field.name)?;
        }
        f.write_str("}")
    }
}

/// A [`Struct`] field.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct StructField<'a> {
    /// The name of the field.
    pub name: &'a str,
    /// The type of the field.
    pub ty: &'a str,
    /// The description of the field.
    /// This is a markdown string derived from the NatSpec documentation.
    pub description: &'a str,
}
