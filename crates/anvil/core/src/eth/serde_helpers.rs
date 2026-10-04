//! custom serde helper functions

pub mod sequence {
    use serde::{
        Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned, ser::SerializeSeq,
    };

    pub fn serialize<S, T>(val: &T, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(val)?;
        seq.end()
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let mut seq = Vec::<T>::deserialize(d)?;
        if seq.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 1 but got {}",
                seq.len()
            )));
        }
        Ok(seq.remove(0))
    }
}

/// A module that deserializes an optional single-item sequence.
pub mod optional_sequence {
    use serde::{Deserialize, Deserializer, de::DeserializeOwned};

    pub fn deserialize<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let mut seq = Option::<Vec<Option<T>>>::deserialize(d)?.unwrap_or_default();
        if seq.len() > 1 {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 0 or 1 but got {}",
                seq.len()
            )));
        }
        Ok(seq.pop().flatten())
    }
}

/// A module that deserializes `[]` optionally
pub mod empty_params {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(d: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let seq = Option::<Vec<()>>::deserialize(d)?.unwrap_or_default();
        if !seq.is_empty() {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 0 but got {}",
                seq.len()
            )));
        }
        Ok(())
    }
}

/// A module that deserializes either a BlockNumberOrTag, or a simple number.
pub mod lenient_block_number {
    pub use alloy_eips::eip1898::LenientBlockNumberOrTag;
    use alloy_rpc_types::BlockNumberOrTag;
    use serde::{Deserialize, Deserializer};

    /// deserializes either a BlockNumberOrTag, or a simple number.
    pub use alloy_eips::eip1898::lenient_block_number_or_tag::deserialize as lenient_block_number;

    /// Same as `lenient_block_number` but requires to be `[num; 1]`
    pub fn lenient_block_number_seq<'de, D>(deserializer: D) -> Result<BlockNumberOrTag, D::Error>
    where
        D: Deserializer<'de>,
    {
        let num = <[LenientBlockNumberOrTag; 1]>::deserialize(deserializer)?[0].into();
        Ok(num)
    }
}

/// Deserializes a `trace_get` trace address, whose indices must be minimal hex quantities.
pub mod trace_address {
    use alloy_rpc_types::Index;
    use serde::{Deserialize, Deserializer, de::Error};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Index>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|index| {
                parse(index).ok_or_else(|| {
                    D::Error::custom(format!(
                        "invalid trace address index {index:?}, expected a hex quantity"
                    ))
                })
            })
            .collect()
    }

    /// Parses `^0x(0|[1-9a-f][0-9a-f]*)$`.
    fn parse(index: &str) -> Option<Index> {
        let digits = index.strip_prefix("0x")?;
        let minimal = digits == "0" || !digits.starts_with('0');
        if !minimal || !digits.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        usize::from_str_radix(digits, 16).ok().map(Index::from)
    }
}
