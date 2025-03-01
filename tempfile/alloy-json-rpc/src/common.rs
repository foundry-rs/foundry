use serde::{de::Visitor, Deserialize, Serialize};
use std::fmt;

/// A JSON-RPC 2.0 ID object. This may be a number, a string, or null.
///
/// ### Ordering
///
/// This type implements [`PartialOrd`], [`Ord`], [`PartialEq`], and [`Eq`] so
/// that it can be used as a key in a [`BTreeMap`] or an item in a
/// [`BTreeSet`]. The ordering is as follows:
///
/// 1. Numbers are less than strings.
/// 2. Strings are less than null.
/// 3. Null is equal to null.
///
/// ### Hash
///
/// This type implements [`Hash`] so that it can be used as a key in a
/// [`HashMap`] or an item in a [`HashSet`].
///
/// [`BTreeMap`]: std::collections::BTreeMap
/// [`BTreeSet`]: std::collections::BTreeSet
/// [`HashMap`]: std::collections::HashMap
/// [`HashSet`]: std::collections::HashSet
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    /// A number.
    Number(u64),
    /// A string.
    String(String),
    /// Null.
    None,
}

impl From<u64> for Id {
    fn from(value: u64) -> Self {
        Self::Number(value)
    }
}

impl From<String> for Id {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::String(s) => f.write_str(s),
            Self::None => f.write_str("null"),
        }
    }
}

impl Serialize for Id {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Number(n) => serializer.serialize_u64(*n),
            Self::String(s) => serializer.serialize_str(s),
            Self::None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct IdVisitor;

        impl Visitor<'_> for IdVisitor {
            type Value = Id;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a string, a number, or null")
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(v.into())
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(v.to_owned().into())
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Id::None)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Id::None)
            }
        }

        deserializer.deserialize_any(IdVisitor)
    }
}

impl PartialOrd for Id {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Id {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // numbers < strings
        // strings < null
        // null == null
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Number(_), _) => std::cmp::Ordering::Less,

            (Self::String(_), Self::Number(_)) => std::cmp::Ordering::Greater,
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::String(_), Self::None) => std::cmp::Ordering::Less,

            (Self::None, Self::None) => std::cmp::Ordering::Equal,
            (Self::None, _) => std::cmp::Ordering::Greater,
        }
    }
}

impl Id {
    /// Returns `true` if the ID is a number.
    pub const fn is_number(&self) -> bool {
        matches!(self, Self::Number(_))
    }

    /// Returns `true` if the ID is a string.
    pub const fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    /// Returns `true` if the ID is `None`.
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns the ID as a number, if it is one.
    pub const fn as_number(&self) -> Option<u64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Returns the ID as a string, if it is one.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestCase {
        id: Id,
    }

    #[test]
    fn it_serializes_and_deserializes() {
        let cases = [
            (TestCase { id: Id::Number(1) }, r#"{"id":1}"#),
            (TestCase { id: Id::String("foo".to_string()) }, r#"{"id":"foo"}"#),
            (TestCase { id: Id::None }, r#"{"id":null}"#),
        ];
        for (case, expected) in cases {
            let serialized = serde_json::to_string(&case).unwrap();
            assert_eq!(serialized, expected);

            let deserialized: TestCase = serde_json::from_str(expected).unwrap();
            assert_eq!(deserialized, case);
        }
    }

    #[test]
    fn test_is_methods() {
        let id_number = Id::Number(42);
        let id_string = Id::String("test_string".to_string());
        let id_none = Id::None;

        assert!(id_number.is_number());
        assert!(!id_number.is_string());
        assert!(!id_number.is_none());

        assert!(!id_string.is_number());
        assert!(id_string.is_string());
        assert!(!id_string.is_none());

        assert!(!id_none.is_number());
        assert!(!id_none.is_string());
        assert!(id_none.is_none());
    }

    #[test]
    fn test_as_methods() {
        let id_number = Id::Number(42);
        let id_string = Id::String("test_string".to_string());
        let id_none = Id::None;

        assert_eq!(id_number.as_number(), Some(42));
        assert_eq!(id_string.as_number(), None);
        assert_eq!(id_none.as_number(), None);

        assert_eq!(id_number.as_string(), None);
        assert_eq!(id_string.as_string(), Some("test_string"));
        assert_eq!(id_none.as_string(), None);
    }

    #[test]
    fn test_ordering() {
        let id_number = Id::Number(42);
        let id_string = Id::String("test_string".to_string());
        let id_none = Id::None;

        assert!(id_number < id_string);
        assert!(id_string < id_none);
        assert!(id_none == Id::None);
    }

    #[test]
    fn test_serialization_deserialization_edge_cases() {
        // Edge cases for large numbers, empty strings, and None.
        let cases = [
            (TestCase { id: Id::Number(u64::MAX) }, r#"{"id":18446744073709551615}"#),
            (TestCase { id: Id::String("".to_string()) }, r#"{"id":""}"#),
            (TestCase { id: Id::None }, r#"{"id":null}"#),
        ];
        for (case, expected) in cases {
            let serialized = serde_json::to_string(&case).unwrap();
            assert_eq!(serialized, expected);

            let deserialized: TestCase = serde_json::from_str(expected).unwrap();
            assert_eq!(deserialized, case);
        }
    }

    #[test]
    fn test_partial_eq_and_hash() {
        let id1 = Id::Number(42);
        let id2 = Id::String("foo".to_string());
        let id3 = Id::None;

        let mut hash_set = HashSet::new();
        let mut btree_set = BTreeSet::new();

        hash_set.insert(id1.clone());
        hash_set.insert(id2.clone());
        hash_set.insert(id3.clone());

        btree_set.insert(id1);
        btree_set.insert(id2);
        btree_set.insert(id3);

        assert_eq!(hash_set.len(), 3);
        assert_eq!(btree_set.len(), 3);

        assert!(hash_set.contains(&Id::Number(42)));
        assert!(btree_set.contains(&Id::String("foo".to_string())));
    }
}
