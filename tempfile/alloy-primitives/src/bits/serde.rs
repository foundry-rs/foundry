use super::FixedBytes;
use core::fmt;
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

impl<const N: usize> Serialize for FixedBytes<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            let mut buf = hex::Buffer::<N, true>::new();
            serializer.serialize_str(buf.format(&self.0))
        } else {
            serializer.serialize_bytes(self.as_slice())
        }
    }
}

impl<'de, const N: usize> Deserialize<'de> for FixedBytes<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FixedVisitor<const N: usize>;

        impl<'de, const N: usize> Visitor<'de> for FixedVisitor<N> {
            type Value = FixedBytes<N>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "{} bytes, represented as a hex string of length {}, an array of u8, or raw bytes",
                    N,
                    N * 2
                )
            }

            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                FixedBytes::try_from(v).map_err(de::Error::custom)
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let len_error =
                    |i| de::Error::invalid_length(i, &format!("exactly {N} bytes").as_str());
                let mut bytes = [0u8; N];

                for (i, byte) in bytes.iter_mut().enumerate() {
                    *byte = seq.next_element()?.ok_or_else(|| len_error(i))?;
                }

                if let Ok(Some(_)) = seq.next_element::<u8>() {
                    return Err(len_error(N + 1));
                }

                Ok(FixedBytes(bytes))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                <FixedBytes<N> as hex::FromHex>::from_hex(v).map_err(de::Error::custom)
            }
        }

        if deserializer.is_human_readable() {
            deserializer.deserialize_any(FixedVisitor::<N>)
        } else {
            deserializer.deserialize_bytes(FixedVisitor::<N>)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct TestCase<const N: usize> {
        fixed: FixedBytes<N>,
    }

    #[test]
    fn serde() {
        let bytes = FixedBytes([0, 0, 0, 0, 1, 35, 69, 103, 137, 171, 205, 239]);
        let ser = serde_json::to_string(&bytes).unwrap();
        assert_eq!(ser, "\"0x000000000123456789abcdef\"");
        assert_eq!(serde_json::from_str::<FixedBytes<12>>(&ser).unwrap(), bytes);

        let val = serde_json::to_value(bytes).unwrap();
        assert_eq!(val, serde_json::json! {"0x000000000123456789abcdef"});
        assert_eq!(serde_json::from_value::<FixedBytes<12>>(val).unwrap(), bytes);
    }

    #[test]
    fn serde_num_array() {
        let json = serde_json::json! {{"fixed": [0,1,2,3,4]}};

        assert_eq!(
            serde_json::from_value::<TestCase<5>>(json.clone()).unwrap().fixed,
            FixedBytes([0, 1, 2, 3, 4])
        );

        let e = serde_json::from_value::<TestCase<4>>(json).unwrap_err();
        let es = e.to_string();
        assert!(es.contains("invalid length 5, expected exactly 4 bytes"), "{es}");
    }

    #[test]
    fn test_bincode_roundtrip() {
        let bytes = FixedBytes([0, 0, 0, 0, 1, 35, 69, 103, 137, 171, 205, 239]);

        let bin = bincode::serialize(&bytes).unwrap();
        assert_eq!(bincode::deserialize::<FixedBytes<12>>(&bin).unwrap(), bytes);
    }
}
