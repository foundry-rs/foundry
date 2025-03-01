//! Serde helpers.

use alloy_primitives::Bytes;
use serde::{Deserialize, Deserializer};

pub fn deserialize_bytes<'de, D>(d: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(d)?.parse::<Bytes>().map_err(serde::de::Error::custom)
}

pub fn deserialize_opt_bytes<'de, D>(d: D) -> Result<Option<Bytes>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(d)?;
    value.as_deref().map(str::parse).transpose().map_err(serde::de::Error::custom)
}

pub fn default_for_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

pub mod json_string_opt {
    use serde::{
        de::{self, DeserializeOwned},
        Deserialize, Deserializer, Serialize, Serializer,
    };

    pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        if let Some(value) = value {
            value.serialize(serializer)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        if let Some(s) = Option::<String>::deserialize(deserializer)? {
            if s.is_empty() {
                return Ok(None);
            }
            let value = serde_json::Value::String(s);
            serde_json::from_value(value).map_err(de::Error::custom).map(Some)
        } else {
            Ok(None)
        }
    }
}

/// deserializes empty json object `{}` as `None`
pub mod empty_json_object_opt {
    use serde::{
        de::{self, DeserializeOwned},
        Deserialize, Deserializer, Serialize, Serializer,
    };

    pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        if let Some(value) = value {
            value.serialize(serializer)
        } else {
            let empty = serde_json::Value::Object(Default::default());
            serde_json::Value::serialize(&empty, serializer)
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let json = serde_json::Value::deserialize(deserializer)?;
        if json.is_null() {
            return Ok(None);
        }
        if json.as_object().map(|obj| obj.is_empty()).unwrap_or_default() {
            return Ok(None);
        }
        serde_json::from_value(json).map_err(de::Error::custom).map(Some)
    }
}

/// serde support for string
pub mod string_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.starts_with("0x") {
            serializer.serialize_str(value.as_str())
        } else {
            serializer.serialize_str(&format!("0x{value}"))
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if let Some(rem) = value.strip_prefix("0x") {
            Ok(rem.to_string())
        } else {
            Ok(value)
        }
    }
}

pub mod display_from_str_opt {
    use serde::{de, Deserialize, Deserializer, Serializer};
    use std::{fmt, str::FromStr};

    pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: fmt::Display,
        S: Serializer,
    {
        if let Some(value) = value {
            serializer.collect_str(value)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr,
        T::Err: fmt::Display,
    {
        if let Some(s) = Option::<String>::deserialize(deserializer)? {
            s.parse().map_err(de::Error::custom).map(Some)
        } else {
            Ok(None)
        }
    }
}

pub mod display_from_str {
    use serde::{de, Deserialize, Deserializer, Serializer};
    use std::{fmt, str::FromStr};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: fmt::Display,
        S: Serializer,
    {
        serializer.collect_str(value)
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr,
        T::Err: fmt::Display,
    {
        String::deserialize(deserializer)?.parse().map_err(de::Error::custom)
    }
}

/// (De)serialize vec of tuples as map
pub mod tuple_vec_map {
    use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<K, V, S>(data: &[(K, V)], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Serialize,
        V: Serialize,
    {
        serializer.collect_map(data.iter().map(|x| (&x.0, &x.1)))
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<Vec<(K, V)>, D::Error>
    where
        D: Deserializer<'de>,
        K: DeserializeOwned,
        V: DeserializeOwned,
    {
        use serde::de::{MapAccess, Visitor};
        use std::{fmt, marker::PhantomData};

        struct TupleVecMapVisitor<K, V> {
            marker: PhantomData<Vec<(K, V)>>,
        }

        impl<K, V> TupleVecMapVisitor<K, V> {
            pub fn new() -> Self {
                Self { marker: PhantomData }
            }
        }

        impl<'de, K, V> Visitor<'de> for TupleVecMapVisitor<K, V>
        where
            K: Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = Vec<(K, V)>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map")
            }

            #[inline]
            fn visit_unit<E>(self) -> Result<Vec<(K, V)>, E> {
                Ok(Vec::new())
            }

            #[inline]
            fn visit_map<T>(self, mut access: T) -> Result<Vec<(K, V)>, T::Error>
            where
                T: MapAccess<'de>,
            {
                let mut values =
                    Vec::with_capacity(std::cmp::min(access.size_hint().unwrap_or(0), 4096));

                while let Some((key, value)) = access.next_entry()? {
                    values.push((key, value));
                }

                Ok(values)
            }
        }

        deserializer.deserialize_map(TupleVecMapVisitor::new())
    }
}
