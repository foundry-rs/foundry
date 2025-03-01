use alloy_primitives::U256;

/// A hex encoded or decimal index that's intended to be used as a rust index, hence it's
/// deserialized into a `usize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Index(pub usize);

impl From<Index> for usize {
    fn from(idx: Index) -> Self {
        idx.0
    }
}

impl From<Index> for U256 {
    fn from(idx: Index) -> Self {
        Self::from(idx.0)
    }
}

impl From<usize> for Index {
    fn from(idx: usize) -> Self {
        Self(idx)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Index {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("0x{:x}", self.0))
    }
}

#[cfg(feature = "serde")]
impl<'a> serde::Deserialize<'a> for Index {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        struct IndexVisitor;

        impl serde::de::Visitor<'_> for IndexVisitor {
            type Value = Index;

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(formatter, "hex-encoded or decimal index")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Index(value as usize))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                value.strip_prefix("0x").map_or_else(
                    || {
                        value.parse::<usize>().map(Index).map_err(|e| {
                            serde::de::Error::custom(format!("Failed to parse numeric index: {e}"))
                        })
                    },
                    |val| {
                        usize::from_str_radix(val, 16).map(Index).map_err(|e| {
                            serde::de::Error::custom(format!(
                                "Failed to parse hex encoded index value: {e}"
                            ))
                        })
                    },
                )
            }
        }

        deserializer.deserialize_any(IndexVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{thread_rng, Rng};
    use serde_json::json;
    use similar_asserts::assert_eq;

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_index_rand() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let index = Index(rng.gen());
            let val = serde_json::to_string(&index).unwrap();
            let de: Index = serde_json::from_str(&val).unwrap();
            assert_eq!(index, de);
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_index_deserialization() {
        // Test decimal index
        let json_data = json!(42);
        let index: Index =
            serde_json::from_value(json_data).expect("Failed to deserialize decimal index");
        assert_eq!(index, Index::from(42));

        // Test hex index
        let json_data = json!("0x2A");
        let index: Index =
            serde_json::from_value(json_data).expect("Failed to deserialize hex index");
        assert_eq!(index, Index::from(42));

        // Test invalid hex index
        let json_data = json!("0xGHI");
        let result: Result<Index, _> = serde_json::from_value(json_data);
        assert!(result.is_err());

        // Test invalid decimal index
        let json_data = json!("abc");
        let result: Result<Index, _> = serde_json::from_value(json_data);
        assert!(result.is_err());

        // Test string decimal index
        let json_data = json!("123");
        let index: Index =
            serde_json::from_value(json_data).expect("Failed to deserialize string decimal index");
        assert_eq!(index, Index::from(123));

        // Test invalid numeric string
        let json_data = json!("123abc");
        let result: Result<Index, _> = serde_json::from_value(json_data);
        assert!(result.is_err());

        // Test negative index
        let json_data = json!(-1);
        let result: Result<Index, _> = serde_json::from_value(json_data);
        assert!(result.is_err());

        // Test large index
        let json_data = json!(u64::MAX);
        let index: Index =
            serde_json::from_value(json_data).expect("Failed to deserialize large index");
        assert_eq!(index, Index::from(u64::MAX as usize));
    }
}
