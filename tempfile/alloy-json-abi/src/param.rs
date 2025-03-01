use crate::{
    internal_type::BorrowedInternalType,
    utils::{mk_eparam, mk_param, validate_identifier},
    InternalType,
};
use alloc::{borrow::Cow, string::String, vec::Vec};
use core::{fmt, str::FromStr};
use parser::{Error, ParameterSpecifier, TypeSpecifier};
use serde::{de::Unexpected, Deserialize, Deserializer, Serialize, Serializer};

/// JSON specification of a parameter.
///
/// Parameters are the inputs and outputs of [Function]s, and the fields of
/// [Error]s.
///
/// [Function]: crate::Function
/// [Error]: crate::Error
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Param {
    /// The canonical Solidity type of the parameter, using the word "tuple" to
    /// represent complex types. E.g. `uint256` or `bytes[2]` or `tuple` or
    /// `tuple[2]`.
    ///
    /// Generally, this is a valid [`TypeSpecifier`], but in very rare
    /// circumstances, such as when a function in a library contains an enum
    /// in its parameters or return types, this will be `Contract.EnumName`
    /// instead of the actual type (`uint8`).
    /// Visible for macros, functions inside the crate, and doc tests. It is not recommended to
    /// instantiate directly. Use Param::new instead.
    #[doc(hidden)]
    pub ty: String,
    /// The name of the parameter. This field always contains either the empty
    /// string, or a valid Solidity identifier.
    /// Visible for macros, functions inside the crate, and doc tests. It is not recommended to
    /// instantiate directly. Use Param::new instead.
    #[doc(hidden)]
    pub name: String,
    /// If the parameter is a compound type (a struct or tuple), a list of the
    /// parameter's components, in order. Empty otherwise
    pub components: Vec<Param>,
    /// The internal type of the parameter. This type represents the type that
    /// the author of the Solidity contract specified. E.g. for a contract, this
    /// will be `contract MyContract` while the `type` field will be `address`.
    pub internal_type: Option<InternalType>,
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(it) = &self.internal_type { it.fmt(f) } else { f.write_str(&self.ty) }?;
        f.write_str(" ")?;
        f.write_str(&self.name)
    }
}

impl<'de> Deserialize<'de> for Param {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ParamInner::deserialize(deserializer).and_then(|inner| {
            if inner.indexed.is_none() {
                inner.validate_fields()?;
                Ok(Self {
                    name: inner.name,
                    ty: inner.ty,
                    internal_type: inner.internal_type,
                    components: inner.components,
                })
            } else {
                Err(serde::de::Error::custom("indexed is not supported in params"))
            }
        })
    }
}

impl Serialize for Param {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_inner().serialize(serializer)
    }
}

impl FromStr for Param {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Param {
    /// Parse a parameter from a Solidity parameter string.
    ///
    /// # Examples
    ///
    /// ```
    /// # use alloy_json_abi::Param;
    /// assert_eq!(
    ///     Param::parse("uint256[] foo"),
    ///     Ok(Param {
    ///         name: "foo".into(),
    ///         ty: "uint256[]".into(),
    ///         components: vec![],
    ///         internal_type: None,
    ///     })
    /// );
    /// ```
    pub fn parse(input: &str) -> parser::Result<Self> {
        ParameterSpecifier::parse(input).map(|p| mk_param(p.name, p.ty))
    }

    /// Validate and create new instance of Param.
    pub fn new(
        name: &str,
        ty: &str,
        components: Vec<Self>,
        internal_type: Option<InternalType>,
    ) -> parser::Result<Self> {
        Self::validate_fields(name, ty, !components.is_empty())?;
        Ok(Self { ty: ty.into(), name: name.into(), components, internal_type })
    }

    /// The name of the parameter. This function always returns either an empty
    /// slice, or a valid Solidity identifier.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The internal type of the parameter.
    #[inline]
    pub const fn internal_type(&self) -> Option<&InternalType> {
        self.internal_type.as_ref()
    }

    /// True if the parameter is a UDT (user-defined type).
    ///
    /// A UDT will have
    /// - an internal type that does not match its canonical type
    /// - no space in its internal type (as it does not have a keyword body)
    ///
    /// Any `Other` specifier will definitely be a UDT if it contains a
    /// contract.
    #[inline]
    pub fn is_udt(&self) -> bool {
        match self.internal_type().and_then(|it| it.as_other()) {
            Some((contract, ty)) => contract.is_some() || (self.is_simple_type() && ty != self.ty),
            _ => false,
        }
    }

    /// True if the parameter is a struct.
    #[inline]
    pub const fn is_struct(&self) -> bool {
        match self.internal_type() {
            Some(ty) => ty.is_struct(),
            None => false,
        }
    }

    /// True if the parameter is an enum.
    #[inline]
    pub const fn is_enum(&self) -> bool {
        match self.internal_type() {
            Some(ty) => ty.is_enum(),
            None => false,
        }
    }

    /// True if the parameter is a contract.
    #[inline]
    pub const fn is_contract(&self) -> bool {
        match self.internal_type() {
            Some(ty) => ty.is_contract(),
            None => false,
        }
    }

    /// The UDT specifier is a [`TypeSpecifier`] containing the UDT name and any
    /// array sizes. It is computed from the `internal_type`. If this param is
    /// not a UDT, this function will return `None`.
    #[inline]
    pub fn udt_specifier(&self) -> Option<TypeSpecifier<'_>> {
        // UDTs are more annoying to check for, so we reuse logic here.
        if !self.is_udt() {
            return None;
        }
        self.internal_type().and_then(|ty| ty.other_specifier())
    }

    /// The struct specifier is a [`TypeSpecifier`] containing the struct name
    /// and any array sizes. It is computed from the `internal_type` If this
    /// param is not a struct, this function will return `None`.
    #[inline]
    pub fn struct_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.internal_type().and_then(|ty| ty.struct_specifier())
    }

    /// The enum specifier is a [`TypeSpecifier`] containing the enum name and
    /// any array sizes. It is computed from the `internal_type`. If this param
    /// is not a enum, this function will return `None`.
    #[inline]
    pub fn enum_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.internal_type().and_then(|ty| ty.enum_specifier())
    }

    /// The struct specifier is a [`TypeSpecifier`] containing the contract name
    /// and any array sizes. It is computed from the `internal_type` If this
    /// param is not a struct, this function will return `None`.
    #[inline]
    pub fn contract_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.internal_type().and_then(|ty| ty.contract_specifier())
    }

    /// True if the type is simple
    #[inline]
    pub fn is_simple_type(&self) -> bool {
        self.components.is_empty()
    }

    /// True if the type is complex (tuple or struct)
    #[inline]
    pub fn is_complex_type(&self) -> bool {
        !self.components.is_empty()
    }

    /// Formats the canonical type of this parameter into the given string.
    ///
    /// This is used to encode the preimage of a function or error selector.
    #[inline]
    pub fn selector_type_raw(&self, s: &mut String) {
        if self.components.is_empty() {
            s.push_str(&self.ty);
        } else {
            crate::utils::params_abi_tuple(&self.components, s);
            // checked during deserialization, but might be invalid from a user
            if let Some(suffix) = self.ty.strip_prefix("tuple") {
                s.push_str(suffix);
            }
        }
    }

    /// Formats the canonical type of this parameter into the given string including then names of
    /// the params.
    #[inline]
    pub fn full_selector_type_raw(&self, s: &mut String) {
        if self.components.is_empty() {
            s.push_str(&self.ty);
        } else {
            s.push_str("tuple");
            crate::utils::params_tuple(&self.components, s);
            // checked during deserialization, but might be invalid from a user
            if let Some(suffix) = self.ty.strip_prefix("tuple") {
                s.push_str(suffix);
            }
        }
    }

    /// Returns the canonical type of this parameter.
    ///
    /// This is used to encode the preimage of a function or error selector.
    #[inline]
    pub fn selector_type(&self) -> Cow<'_, str> {
        if self.components.is_empty() {
            Cow::Borrowed(&self.ty)
        } else {
            let mut s = String::with_capacity(self.components.len() * 32);
            self.selector_type_raw(&mut s);
            Cow::Owned(s)
        }
    }

    #[inline]
    fn borrowed_internal_type(&self) -> Option<BorrowedInternalType<'_>> {
        self.internal_type().as_ref().map(|it| it.as_borrowed())
    }

    #[inline]
    fn as_inner(&self) -> BorrowedParamInner<'_> {
        BorrowedParamInner {
            name: &self.name,
            ty: &self.ty,
            indexed: None,
            internal_type: self.borrowed_internal_type(),
            components: Cow::Borrowed(&self.components),
        }
    }

    #[inline]
    fn validate_fields(name: &str, ty: &str, has_components: bool) -> parser::Result<()> {
        if !name.is_empty() && !parser::is_valid_identifier(name) {
            return Err(Error::invalid_identifier_string(name));
        }

        // any components means type is "tuple" + maybe brackets, so we can skip
        // parsing with TypeSpecifier
        if !has_components {
            parser::TypeSpecifier::parse(ty)?;
        } else {
            // https://docs.soliditylang.org/en/latest/abi-spec.html#handling-tuple-types
            // checking for "tuple" prefix should be enough
            if !ty.starts_with("tuple") {
                return Err(Error::invalid_type_string(ty));
            }
        }
        Ok(())
    }
}

/// A Solidity Event parameter.
///
/// Event parameters are distinct from function parameters in that they have an
/// `indexed` field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct EventParam {
    /// The canonical Solidity type of the parameter, using the word "tuple" to
    /// represent complex types. E.g. `uint256` or `bytes[2]` or `tuple` or
    /// `tuple[2]`.
    ///
    /// Generally, this is a valid [`TypeSpecifier`], but in very rare
    /// circumstances, such as when a function in a library contains an enum
    /// in its parameters or return types, this will be `Contract.EnumName`
    /// instead of the actual type (`uint8`).
    /// Visible for macros, functions inside the crate, and doc tests. It is not recommended to
    /// instantiate directly. Use Param::new instead.
    #[doc(hidden)]
    pub ty: String,
    /// The name of the parameter. This field always contains either the empty
    /// string, or a valid Solidity identifier.
    /// Visible for macros, functions inside the crate, and doc tests. It is not recommended to
    /// instantiate directly. Use Param::new instead.
    #[doc(hidden)]
    pub name: String,
    /// Whether the parameter is indexed. Indexed parameters have their
    /// value, or the hash of their value, stored in the log topics.
    pub indexed: bool,
    /// If the parameter is a compound type (a struct or tuple), a list of the
    /// parameter's components, in order. Empty otherwise. Because the
    /// components are not top-level event params, they will not have an
    /// `indexed` field.
    pub components: Vec<Param>,
    /// The internal type of the parameter. This type represents the type that
    /// the author of the Solidity contract specified. E.g. for a contract, this
    /// will be `contract MyContract` while the `type` field will be `address`.
    pub internal_type: Option<InternalType>,
}

impl fmt::Display for EventParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(it) = &self.internal_type { it.fmt(f) } else { f.write_str(&self.ty) }?;
        f.write_str(" ")?;
        f.write_str(&self.name)
    }
}

impl<'de> Deserialize<'de> for EventParam {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ParamInner::deserialize(deserializer).and_then(|inner| {
            inner.validate_fields()?;
            Ok(Self {
                name: inner.name,
                ty: inner.ty,
                indexed: inner.indexed.unwrap_or(false),
                internal_type: inner.internal_type,
                components: inner.components,
            })
        })
    }
}

impl Serialize for EventParam {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_inner().serialize(serializer)
    }
}

impl FromStr for EventParam {
    type Err = parser::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl EventParam {
    /// Parse an event parameter from a Solidity parameter string.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::panic::catch_unwind;
    /// use alloy_json_abi::EventParam;
    /// assert_eq!(
    ///     EventParam::parse("uint256[] indexed foo"),
    ///     Ok(EventParam {
    ///         name: "foo".into(),
    ///         ty: "uint256[]".into(),
    ///         indexed: true,
    ///         components: vec![],
    ///         internal_type: None,
    ///     })
    /// );
    /// ```
    #[inline]
    pub fn parse(input: &str) -> parser::Result<Self> {
        ParameterSpecifier::parse(input).map(mk_eparam)
    }

    /// Validate and create new instance of EventParam
    pub fn new(
        name: &str,
        ty: &str,
        indexed: bool,
        components: Vec<Param>,
        internal_type: Option<InternalType>,
    ) -> parser::Result<Self> {
        Param::validate_fields(name, ty, !components.is_empty())?;
        Ok(Self { name: name.into(), ty: ty.into(), indexed, components, internal_type })
    }

    /// The internal type of the parameter.
    #[inline]
    pub const fn internal_type(&self) -> Option<&InternalType> {
        self.internal_type.as_ref()
    }

    /// True if the parameter is a UDT (user-defined type).
    ///
    /// A UDT will have
    /// - an internal type that does not match its canonical type
    /// - no space in its internal type (as it does not have a keyword body)
    ///
    /// Any `Other` specifier will definitely be a UDT if it contains a
    /// contract.
    #[inline]
    pub fn is_udt(&self) -> bool {
        match self.internal_type().and_then(|it| it.as_other()) {
            Some((contract, ty)) => contract.is_some() || (self.is_simple_type() && ty != self.ty),
            _ => false,
        }
    }

    /// True if the parameter is a struct.
    #[inline]
    pub const fn is_struct(&self) -> bool {
        match self.internal_type() {
            Some(ty) => ty.is_struct(),
            None => false,
        }
    }

    /// True if the parameter is an enum.
    #[inline]
    pub const fn is_enum(&self) -> bool {
        match self.internal_type() {
            Some(ty) => ty.is_enum(),
            None => false,
        }
    }

    /// True if the parameter is a contract.
    #[inline]
    pub const fn is_contract(&self) -> bool {
        match self.internal_type() {
            Some(ty) => ty.is_contract(),
            None => false,
        }
    }

    /// The UDT specifier is a [`TypeSpecifier`] containing the UDT name and any
    /// array sizes. It is computed from the `internal_type`. If this param is
    /// not a UDT, this function will return `None`.
    #[inline]
    pub fn udt_specifier(&self) -> Option<TypeSpecifier<'_>> {
        // UDTs are more annoying to check for, so we reuse logic here.
        if !self.is_udt() {
            return None;
        }
        self.internal_type().and_then(|ty| ty.other_specifier())
    }

    /// The struct specifier is a [`TypeSpecifier`] containing the struct name
    /// and any array sizes. It is computed from the `internal_type` If this
    /// param is not a struct, this function will return `None`.
    #[inline]
    pub fn struct_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.internal_type().and_then(|ty| ty.struct_specifier())
    }

    /// The enum specifier is a [`TypeSpecifier`] containing the enum name and
    /// any array sizes. It is computed from the `internal_type`. If this param
    /// is not a enum, this function will return `None`.
    #[inline]
    pub fn enum_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.internal_type().and_then(|ty| ty.enum_specifier())
    }

    /// The struct specifier is a [`TypeSpecifier`] containing the contract name
    /// and any array sizes. It is computed from the `internal_type` If this
    /// param is not a struct, this function will return `None`.
    #[inline]
    pub fn contract_specifier(&self) -> Option<TypeSpecifier<'_>> {
        self.internal_type().and_then(|ty| ty.contract_specifier())
    }

    /// True if the type is simple
    #[inline]
    pub fn is_simple_type(&self) -> bool {
        self.components.is_empty()
    }

    /// True if the type is complex (tuple or struct)
    #[inline]
    pub fn is_complex_type(&self) -> bool {
        !self.components.is_empty()
    }

    /// Formats the canonical type of this parameter into the given string.
    ///
    /// This is used to encode the preimage of the event selector.
    #[inline]
    pub fn selector_type_raw(&self, s: &mut String) {
        if self.components.is_empty() {
            s.push_str(&self.ty);
        } else {
            crate::utils::params_abi_tuple(&self.components, s);
            // checked during deserialization, but might be invalid from a user
            if let Some(suffix) = self.ty.strip_prefix("tuple") {
                s.push_str(suffix);
            }
        }
    }

    /// Formats the canonical type of this parameter into the given string including then names of
    /// the params.
    #[inline]
    pub fn full_selector_type_raw(&self, s: &mut String) {
        if self.components.is_empty() {
            s.push_str(&self.ty);
        } else {
            s.push_str("tuple");
            crate::utils::params_tuple(&self.components, s);
            // checked during deserialization, but might be invalid from a user
            if let Some(suffix) = self.ty.strip_prefix("tuple") {
                s.push_str(suffix);
            }
        }
    }

    /// Returns the canonical type of this parameter.
    ///
    /// This is used to encode the preimage of the event selector.
    #[inline]
    pub fn selector_type(&self) -> Cow<'_, str> {
        if self.components.is_empty() {
            Cow::Borrowed(&self.ty)
        } else {
            let mut s = String::with_capacity(self.components.len() * 32);
            self.selector_type_raw(&mut s);
            Cow::Owned(s)
        }
    }

    #[inline]
    fn borrowed_internal_type(&self) -> Option<BorrowedInternalType<'_>> {
        self.internal_type().as_ref().map(|it| it.as_borrowed())
    }

    #[inline]
    fn as_inner(&self) -> BorrowedParamInner<'_> {
        BorrowedParamInner {
            name: &self.name,
            ty: &self.ty,
            indexed: Some(self.indexed),
            internal_type: self.borrowed_internal_type(),
            components: Cow::Borrowed(&self.components),
        }
    }
}

#[derive(Deserialize)]
struct ParamInner {
    #[serde(default)]
    name: String,
    #[serde(rename = "type")]
    ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    indexed: Option<bool>,
    #[serde(rename = "internalType", default, skip_serializing_if = "Option::is_none")]
    internal_type: Option<InternalType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    components: Vec<Param>,
}

impl Serialize for ParamInner {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_borrowed().serialize(serializer)
    }
}

impl ParamInner {
    #[inline]
    fn validate_fields<E: serde::de::Error>(&self) -> Result<(), E> {
        self.as_borrowed().validate_fields()
    }

    #[inline]
    fn as_borrowed(&self) -> BorrowedParamInner<'_> {
        BorrowedParamInner {
            name: &self.name,
            ty: &self.ty,
            indexed: self.indexed,
            internal_type: self.internal_type.as_ref().map(InternalType::as_borrowed),
            components: Cow::Borrowed(&self.components),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct BorrowedParamInner<'a> {
    #[serde(default)]
    name: &'a str,
    #[serde(rename = "type")]
    ty: &'a str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    indexed: Option<bool>,
    #[serde(rename = "internalType", default, skip_serializing_if = "Option::is_none")]
    internal_type: Option<BorrowedInternalType<'a>>,
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    components: Cow<'a, [Param]>,
}

impl BorrowedParamInner<'_> {
    fn validate_fields<E: serde::de::Error>(&self) -> Result<(), E> {
        validate_identifier(self.name)?;

        // any components means type is "tuple" + maybe brackets, so we can skip
        // parsing with TypeSpecifier
        if self.components.is_empty() {
            if parser::TypeSpecifier::parse(self.ty).is_err() {
                return Err(E::invalid_value(
                    Unexpected::Str(self.ty),
                    &"a valid Solidity type specifier",
                ));
            }
        } else {
            // https://docs.soliditylang.org/en/latest/abi-spec.html#handling-tuple-types
            // checking for "tuple" prefix should be enough
            if !self.ty.starts_with("tuple") {
                return Err(E::invalid_value(
                    Unexpected::Str(self.ty),
                    &"a string prefixed with `tuple`, optionally followed by a sequence of `[]` or `[k]` with integers `k`",
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_from_json() {
        let param = r#"{
            "internalType": "string",
            "name": "reason",
            "type": "string"
        }"#;
        let expected = Param {
            name: "reason".into(),
            ty: "string".into(),
            internal_type: Some(InternalType::Other { contract: None, ty: "string".into() }),
            components: vec![],
        };

        assert_eq!(serde_json::from_str::<Param>(param).unwrap(), expected);

        let param_value = serde_json::from_str::<serde_json::Value>(param).unwrap();
        assert_eq!(serde_json::from_value::<Param>(param_value).unwrap(), expected);

        #[cfg(feature = "std")]
        {
            let reader = std::io::Cursor::new(param);
            assert_eq!(serde_json::from_reader::<_, Param>(reader).unwrap(), expected);
        }
    }

    #[test]
    fn param_from_new() {
        let param = Param::new("something", "string", vec![], None);
        assert_eq!(
            param,
            Ok(Param {
                name: "something".into(),
                ty: "string".into(),
                components: vec![],
                internal_type: None,
            })
        );

        let err_not_a_type = Param::new("something", "not a type", vec![], None);
        assert!(err_not_a_type.is_err());

        let err_not_tuple = Param::new("something", "string", vec![param.unwrap()], None);
        assert!(err_not_tuple.is_err())
    }
}
