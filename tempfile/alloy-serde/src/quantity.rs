//! Serde functions for encoding primitive numbers using the Ethereum JSON-RPC "quantity" format.
//!
//! This is defined as a "hex encoded unsigned integer", with a special case of 0 being `0x0`.
//!
//! A regex for this format is: `^0x([1-9a-f]+[0-9a-f]*|0)$`.
//!
//! This is only valid for human-readable [`serde`] implementations.
//! For non-human-readable implementations, the format is unspecified.
//! Currently, it uses a fixed-width big-endian byte-array.

use private::ConvertRuint;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serializes a primitive number as a "quantity" hex string.
pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: ConvertRuint,
    S: Serializer,
{
    value.into_ruint().serialize(serializer)
}

/// Deserializes a primitive number from a "quantity" hex string.
pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: ConvertRuint,
    D: Deserializer<'de>,
{
    T::Ruint::deserialize(deserializer).map(T::from_ruint)
}

/// Serde functions for encoding optional primitive numbers using the Ethereum "quantity" format.
///
/// See [`quantity`](self) for more information.
pub mod opt {
    use super::private::ConvertRuint;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serializes an optional primitive number as a "quantity" hex string.
    pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: ConvertRuint,
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&value.into_ruint()),
            None => serializer.serialize_none(),
        }
    }

    /// Deserializes an optional primitive number from a "quantity" hex string.
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        T: ConvertRuint,
        D: Deserializer<'de>,
    {
        Ok(Option::<T::Ruint>::deserialize(deserializer)?.map(T::from_ruint))
    }
}

/// Serde functions for encoding a list of primitive numbers using the Ethereum "quantity" format.
///
/// See [`quantity`](self) for more information.
pub mod vec {
    use super::private::ConvertRuint;
    use alloc::vec::Vec;
    use core::{fmt, marker::PhantomData};
    use serde::{
        de::{SeqAccess, Visitor},
        ser::SerializeSeq,
        Deserializer, Serializer,
    };

    /// Serializes a vector of primitive numbers as a "quantity" hex string.
    pub fn serialize<T, S>(value: &[T], serializer: S) -> Result<S::Ok, S::Error>
    where
        T: ConvertRuint,
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(value.len()))?;
        for val in value {
            seq.serialize_element(&val.into_ruint())?;
        }
        seq.end()
    }

    /// Deserializes a vector of primitive numbers from a "quantity" hex string.
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
    where
        T: ConvertRuint,
        D: Deserializer<'de>,
    {
        struct VecVisitor<T> {
            marker: PhantomData<T>,
        }

        impl<'de, T> Visitor<'de> for VecVisitor<T>
        where
            T: ConvertRuint,
        {
            type Value = Vec<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a sequence")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::<T>::with_capacity(seq.size_hint().unwrap_or(0));

                while let Some(value) = seq.next_element::<T::Ruint>()? {
                    values.push(T::from_ruint(value));
                }
                Ok(values)
            }
        }

        let visitor = VecVisitor { marker: PhantomData };
        deserializer.deserialize_seq(visitor)
    }
}

/// serde functions for handling `Vec<Vec<u128>>` via [U128](alloy_primitives::U128)
pub mod u128_vec_vec_opt {
    use alloy_primitives::U128;
    use serde::{Deserialize, Deserializer, Serializer};

    #[cfg(not(feature = "std"))]
    use alloc::vec::Vec;

    /// Deserializes an `u128` accepting a hex quantity string with optional 0x prefix or
    /// a number
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<Vec<u128>>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<Vec<Vec<U128>>>::deserialize(deserializer)?.map_or_else(
            || Ok(None),
            |vec| {
                Ok(Some(
                    vec.into_iter().map(|v| v.into_iter().map(|val| val.to()).collect()).collect(),
                ))
            },
        )
    }

    /// Serializes u128 as hex string
    pub fn serialize<S: Serializer>(
        value: &Option<Vec<Vec<u128>>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(vec) => {
                let vec = vec
                    .iter()
                    .map(|v| v.iter().map(|val| U128::from(*val)).collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                s.serialize_some(&vec)
            }
            None => s.serialize_none(),
        }
    }
}

/// Serde functions for encoding a `HashMap` of primitive numbers using the Ethereum "quantity"
/// format.
///
/// See [`quantity`](self) for more information.
pub mod hashmap {
    use super::private::ConvertRuint;
    use alloy_primitives::map::HashMap;
    use core::{fmt, hash::BuildHasher, marker::PhantomData};
    use serde::{
        de::MapAccess, ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer,
    };

    /// Serializes a `HashMap` of primitive numbers as a "quantity" hex string.
    pub fn serialize<K, V, S, H>(map: &HashMap<K, V, H>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: ConvertRuint,
        V: Serialize,
        S: Serializer,
        H: BuildHasher,
    {
        let mut map_ser = serializer.serialize_map(Some(map.len()))?;
        for (key, value) in map {
            map_ser.serialize_entry(&key.into_ruint(), value)?;
        }
        map_ser.end()
    }

    /// Deserializes a `HashMap` of primitive numbers from a "quantity" hex string.
    pub fn deserialize<'de, K, V, D, H>(deserializer: D) -> Result<HashMap<K, V, H>, D::Error>
    where
        K: ConvertRuint + Eq + core::hash::Hash,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
        H: BuildHasher + Default,
    {
        struct HashMapVisitor<K, V, H> {
            marker: PhantomData<(K, V, H)>,
        }

        impl<'de, K, V, H> serde::de::Visitor<'de> for HashMapVisitor<K, V, H>
        where
            K: ConvertRuint + Eq + core::hash::Hash,
            V: Deserialize<'de>,
            H: BuildHasher + Default,
        {
            type Value = HashMap<K, V, H>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map with quantity hex-encoded keys")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values =
                    HashMap::with_capacity_and_hasher(map.size_hint().unwrap_or(0), H::default());

                while let Some((key, value)) = map.next_entry::<K::Ruint, V>()? {
                    values.insert(K::from_ruint(key), value);
                }
                Ok(values)
            }
        }

        let visitor = HashMapVisitor { marker: PhantomData };
        deserializer.deserialize_map(visitor)
    }
}

/// Serde functions for encoding a `BTreeMap` of primitive numbers using the Ethereum "quantity"
/// format.
pub mod btreemap {
    use super::private::ConvertRuint;
    use alloc::collections::BTreeMap;
    use core::{fmt, marker::PhantomData};
    use serde::{
        de::MapAccess, ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer,
    };

    /// Serializes a `BTreeMap` of primitive numbers as a "quantity" hex string.
    pub fn serialize<K, V, S>(value: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: ConvertRuint + Ord,
        V: Serialize,
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(value.len()))?;
        for (key, val) in value {
            map.serialize_entry(&key.into_ruint(), val)?;
        }
        map.end()
    }

    /// Deserializes a `BTreeMap` of primitive numbers from a "quantity" hex string.
    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: ConvertRuint + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        struct BTreeMapVisitor<K, V> {
            key_marker: PhantomData<K>,
            value_marker: PhantomData<V>,
        }

        impl<'de, K, V> serde::de::Visitor<'de> for BTreeMapVisitor<K, V>
        where
            K: ConvertRuint + Ord,
            V: Deserialize<'de>,
        {
            type Value = BTreeMap<K, V>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map with quantity hex-encoded keys")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut values = BTreeMap::new();

                while let Some((key, value)) = map.next_entry::<K::Ruint, V>()? {
                    values.insert(K::from_ruint(key), value);
                }
                Ok(values)
            }
        }

        let visitor = BTreeMapVisitor { key_marker: PhantomData, value_marker: PhantomData };
        deserializer.deserialize_map(visitor)
    }
}

/// Private implementation details of the [`quantity`](self) module.
#[allow(unnameable_types)]
mod private {
    #[doc(hidden)]
    pub trait ConvertRuint: Copy + Sized {
        // We have to use `Try*` traits because `From` is not implemented by ruint types.
        // They shouldn't ever error.
        type Ruint: Copy
            + serde::Serialize
            + serde::de::DeserializeOwned
            + TryFrom<Self>
            + TryInto<Self>;

        #[inline]
        fn into_ruint(self) -> Self::Ruint {
            self.try_into().ok().unwrap()
        }

        #[inline]
        fn from_ruint(ruint: Self::Ruint) -> Self {
            ruint.try_into().ok().unwrap()
        }
    }

    macro_rules! impl_from_ruint {
        ($($primitive:ty = $ruint:ty),* $(,)?) => {
            $(
                impl ConvertRuint for $primitive {
                    type Ruint = $ruint;
                }
            )*
        };
    }

    impl_from_ruint! {
        bool = alloy_primitives::ruint::aliases::U1,
        u8   = alloy_primitives::U8,
        u16  = alloy_primitives::U16,
        u32  = alloy_primitives::U32,
        u64  = alloy_primitives::U64,
        u128 = alloy_primitives::U128,
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec, vec::Vec};
    use serde::{Deserialize, Serialize};

    #[test]
    fn test_hex_u64() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super")]
            inner: u64,
        }

        let val = Value { inner: 1000 };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":\"0x3e8\"}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);
    }

    #[test]
    fn test_u128_via_ruint() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super")]
            inner: u128,
        }

        let val = Value { inner: 1000 };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":\"0x3e8\"}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);

        let s = "{\"inner\":\"1000\"}".to_string();
        let deserialized: Value = serde_json::from_str(&s).unwrap();

        assert_eq!(val, deserialized);
    }

    #[test]
    fn test_u128_opt_via_ruint() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super::opt")]
            inner: Option<u128>,
        }

        let val = Value { inner: Some(1000) };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":\"0x3e8\"}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);

        let s = "{\"inner\":\"1000\"}".to_string();
        let deserialized: Value = serde_json::from_str(&s).unwrap();

        assert_eq!(val, deserialized);

        let val = Value { inner: None };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":null}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);
    }

    #[test]
    fn test_u128_vec_via_ruint() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super::vec")]
            inner: Vec<u128>,
        }

        let val = Value { inner: vec![1000, 2000] };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":[\"0x3e8\",\"0x7d0\"]}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);
    }

    #[test]
    fn test_u128_vec_vec_opt_via_ruint() {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super::u128_vec_vec_opt")]
            inner: Option<Vec<Vec<u128>>>,
        }

        let val = Value { inner: Some(vec![vec![1000, 2000], vec![3000, 4000]]) };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":[[\"0x3e8\",\"0x7d0\"],[\"0xbb8\",\"0xfa0\"]]}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);

        let val = Value { inner: None };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":null}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);
    }

    #[test]
    fn test_u128_hashmap_via_ruint() {
        use alloy_primitives::map::HashMap;

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super::hashmap")]
            inner: HashMap<u128, u128>,
        }

        let mut inner_map = HashMap::default();
        inner_map.insert(1000, 2000);
        inner_map.insert(3000, 4000);

        let val = Value { inner: inner_map.clone() };
        let s = serde_json::to_string(&val).unwrap();

        // Deserialize and verify that the original `val` and deserialized version match
        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val.inner, deserialized.inner);
    }

    #[test]
    fn test_u128_btreemap_via_ruint() {
        use alloc::collections::BTreeMap;

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct Value {
            #[serde(with = "super::btreemap")]
            inner: BTreeMap<u128, u128>,
        }

        let mut inner_map = BTreeMap::new();
        inner_map.insert(1000, 2000);
        inner_map.insert(3000, 4000);

        let val = Value { inner: inner_map };
        let s = serde_json::to_string(&val).unwrap();
        assert_eq!(s, "{\"inner\":{\"0x3e8\":2000,\"0xbb8\":4000}}");

        let deserialized: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(val, deserialized);
    }
}
