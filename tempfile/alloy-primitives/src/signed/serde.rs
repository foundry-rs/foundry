use super::Signed;
use alloc::string::String;
use core::fmt;
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

impl<const BITS: usize, const LIMBS: usize> Serialize for Signed<BITS, LIMBS> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de, const BITS: usize, const LIMBS: usize> Deserialize<'de> for Signed<BITS, LIMBS> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SignedVisitor<const BITS: usize, const LIMBS: usize>;

        impl<const BITS: usize, const LIMBS: usize> Visitor<'_> for SignedVisitor<BITS, LIMBS> {
            type Value = Signed<BITS, LIMBS>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a {BITS} bit signed integer")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Signed::try_from(v).map_err(de::Error::custom)
            }

            fn visit_u128<E: de::Error>(self, v: u128) -> Result<Self::Value, E> {
                Signed::try_from(v).map_err(de::Error::custom)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Signed::try_from(v).map_err(de::Error::custom)
            }

            fn visit_i128<E: de::Error>(self, v: i128) -> Result<Self::Value, E> {
                Signed::try_from(v).map_err(de::Error::custom)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse().map_err(serde::de::Error::custom)
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
        }

        deserializer.deserialize_any(SignedVisitor)
    }
}

// TODO: Tests
