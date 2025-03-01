//! Serde functions for (de)serializing using FromStr and Display
//!
//! Useful for example in encoding SSZ `uintN` primitives using the "canonical JSON mapping"
//! described in the consensus-specs here: <https://github.com/ethereum/consensus-specs/blob/dev/ssz/simple-serialize.md#json-mapping>
//!
//! # Example
//! ```
//! use alloy_serde;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
//! pub struct Container {
//!     #[serde(with = "alloy_serde::displayfromstr")]
//!     value: u64,
//! }
//!
//! let val = Container { value: 18112749083033600 };
//! let s = serde_json::to_string(&val).unwrap();
//! assert_eq!(s, "{\"value\":\"18112749083033600\"}");
//!
//! let deserialized: Container = serde_json::from_str(&s).unwrap();
//! assert_eq!(val, deserialized);
//! ```

use crate::alloc::string::{String, ToString};
use core::{fmt, str::FromStr};
use serde::{Deserialize, Deserializer, Serializer};

/// Serialize a type `T` that implements [fmt::Display] as a quoted string.
pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: fmt::Display,
    S: Serializer,
{
    serializer.collect_str(&value.to_string())
}

/// Deserialize a quoted string to a type `T` using [FromStr].
pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
}
