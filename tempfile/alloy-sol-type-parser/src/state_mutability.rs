#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "serde")]
const COMPAT_ERROR: &str = "state mutability cannot be both `payable` and `constant`";

/// A JSON ABI function's state mutability.
///
/// This will serialize/deserialize as the `stateMutability` JSON ABI field's value, see
/// [`as_json_str`](Self::as_json_str).
/// For backwards compatible deserialization, see [`serde_state_mutability_compat`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum StateMutability {
    /// Pure functions promise not to read from or modify the state.
    Pure,
    /// View functions promise not to modify the state.
    View,
    /// Nonpayable functions promise not to receive Ether.
    ///
    /// This is the solidity default: <https://docs.soliditylang.org/en/latest/abi-spec.html#json>
    ///
    /// The state mutability nonpayable is reflected in Solidity by not specifying a state
    /// mutability modifier at all.
    #[default]
    NonPayable,
    /// Payable functions make no promises.
    Payable,
}

impl core::str::FromStr for StateMutability {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

impl StateMutability {
    /// Parses a state mutability from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pure" => Some(Self::Pure),
            "view" => Some(Self::View),
            "payable" => Some(Self::Payable),
            _ => None,
        }
    }

    /// Returns the string representation of the state mutability.
    #[inline]
    pub const fn as_str(self) -> Option<&'static str> {
        if let Self::NonPayable = self {
            None
        } else {
            Some(self.as_json_str())
        }
    }

    /// Returns the string representation of the state mutability when serialized to JSON.
    #[inline]
    pub const fn as_json_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::View => "view",
            Self::NonPayable => "nonpayable",
            Self::Payable => "payable",
        }
    }
}

/// [`serde`] implementation for [`StateMutability`] for backwards compatibility with older
/// versions of the JSON ABI.
///
/// In particular, this will deserialize the `stateMutability` field if it is present,
/// and otherwise fall back to the deprecated `constant` and `payable` fields.
///
/// Since it must be used in combination with `#[serde(flatten)]`, a `serialize` implementation
/// is also provided, which will always serialize the `stateMutability` field.
///
/// # Examples
///
/// Usage: `#[serde(default, flatten, with = "serde_state_mutability_compat")]` on a
/// [`StateMutability`] struct field.
///
/// ```rust
/// use alloy_sol_type_parser::{serde_state_mutability_compat, StateMutability};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize)]
/// #[serde(rename_all = "camelCase")]
/// struct MyStruct {
///     #[serde(default, flatten, with = "serde_state_mutability_compat")]
///     state_mutability: StateMutability,
/// }
///
/// let json = r#"{"constant":true,"payable":false}"#;
/// let ms = serde_json::from_str::<MyStruct>(json).expect("failed deserializing");
/// assert_eq!(ms.state_mutability, StateMutability::View);
///
/// let reserialized = serde_json::to_string(&ms).expect("failed reserializing");
/// assert_eq!(reserialized, r#"{"stateMutability":"view"}"#);
/// ```
#[cfg(feature = "serde")]
pub mod serde_state_mutability_compat {
    use super::*;
    use serde::ser::SerializeStruct;

    /// Deserializes a [`StateMutability`], compatible with older JSON ABI versions.
    ///
    /// See [the module-level documentation](self) for more information.
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<StateMutability, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct StateMutabilityCompat {
            #[serde(default)]
            state_mutability: Option<StateMutability>,
            #[serde(default)]
            payable: Option<bool>,
            #[serde(default)]
            constant: Option<bool>,
        }

        impl StateMutabilityCompat {
            fn flatten(self) -> Option<StateMutability> {
                let Self { state_mutability, payable, constant } = self;
                if state_mutability.is_some() {
                    return state_mutability;
                }
                match (payable.unwrap_or(false), constant.unwrap_or(false)) {
                    (false, false) => Some(StateMutability::default()),
                    (true, false) => Some(StateMutability::Payable),
                    (false, true) => Some(StateMutability::View),
                    (true, true) => None,
                }
            }
        }

        StateMutabilityCompat::deserialize(deserializer).and_then(|compat| {
            compat.flatten().ok_or_else(|| serde::de::Error::custom(COMPAT_ERROR))
        })
    }

    /// Serializes a [`StateMutability`] as a single-field struct (`stateMutability`).
    ///
    /// See [the module-level documentation](self) for more information.
    pub fn serialize<S: serde::Serializer>(
        state_mutability: &StateMutability,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("StateMutability", 1)?;
        s.serialize_field("stateMutability", state_mutability)?;
        s.end()
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[derive(Debug, Serialize, Deserialize)]
    struct CompatTest {
        #[serde(default, flatten, with = "serde_state_mutability_compat")]
        sm: StateMutability,
    }

    #[test]
    fn test_compat() {
        let test = |expect: StateMutability, json: &str| {
            let compat = serde_json::from_str::<CompatTest>(json).expect(json);
            assert_eq!(compat.sm, expect, "{json:?}");

            let re_ser = serde_json::to_string(&compat).expect(json);
            let expect = format!(r#"{{"stateMutability":"{}"}}"#, expect.as_json_str());
            assert_eq!(re_ser, expect, "{json:?}");
        };

        test(StateMutability::Pure, r#"{"stateMutability":"pure"}"#);
        test(
            StateMutability::Pure,
            r#"{"stateMutability":"pure","constant":false,"payable":false}"#,
        );

        test(StateMutability::View, r#"{"constant":true}"#);
        test(StateMutability::View, r#"{"constant":true,"payable":false}"#);

        test(StateMutability::Payable, r#"{"payable":true}"#);
        test(StateMutability::Payable, r#"{"constant":false,"payable":true}"#);

        test(StateMutability::NonPayable, r#"{}"#);
        test(StateMutability::NonPayable, r#"{"constant":false}"#);
        test(StateMutability::NonPayable, r#"{"payable":false}"#);
        test(StateMutability::NonPayable, r#"{"constant":false,"payable":false}"#);

        let json = r#"{"constant":true,"payable":true}"#;
        let e = serde_json::from_str::<CompatTest>(json).unwrap_err().to_string();
        assert!(e.contains(COMPAT_ERROR), "{e:?}");
    }
}
