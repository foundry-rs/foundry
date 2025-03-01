use alloc::collections::BTreeMap;
use alloy_primitives::{
    ruint::{BaseConvertError, ParseError},
    Bytes, B256, U256,
};
use core::{fmt, str::FromStr};
use serde::{Deserialize, Deserializer, Serialize};

/// A storage key type that can be serialized to and from a hex string up to 32 bytes. Used for
/// `eth_getStorageAt` and `eth_getProof` RPCs.
///
/// This is a wrapper type meant to mirror geth's serialization and deserialization behavior for
/// storage keys.
///
/// In `eth_getStorageAt`, this is used for deserialization of the `index` field. Internally, the
/// index is a [B256], but in `eth_getStorageAt` requests, its serialization can be _up to_ 32
/// bytes. To support this, the storage key is deserialized first as a U256, and converted to a
/// B256 for use internally.
///
/// `eth_getProof` also takes storage keys up to 32 bytes as input, so the `keys` field is
/// similarly deserialized. However, geth populates the storage proof `key` fields in the response
/// by mirroring the `key` field used in the input.
///
/// See how `storageKey`s (the input) are populated in the `StorageResult` (the output):
/// <https://github.com/ethereum/go-ethereum/blob/00a73fbcce3250b87fc4160f3deddc44390848f4/internal/ethapi/api.go#L658-L690>
///
/// The contained [B256] and From implementation for String are used to preserve the input and
/// implement this behavior from geth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonStorageKey {
    /// A full 32-byte key (tried first during deserialization)
    Hash(B256),
    /// A number (fallback if B256 deserialization fails)
    Number(U256),
}

impl JsonStorageKey {
    /// Returns the key as a [`B256`] value.
    pub fn as_b256(&self) -> B256 {
        match self {
            Self::Hash(hash) => *hash,
            Self::Number(num) => B256::from(*num),
        }
    }
}

impl Default for JsonStorageKey {
    fn default() -> Self {
        Self::Hash(Default::default())
    }
}

impl From<B256> for JsonStorageKey {
    fn from(value: B256) -> Self {
        Self::Hash(value)
    }
}

impl From<[u8; 32]> for JsonStorageKey {
    fn from(value: [u8; 32]) -> Self {
        B256::from(value).into()
    }
}

impl From<U256> for JsonStorageKey {
    fn from(value: U256) -> Self {
        Self::Number(value)
    }
}

impl FromStr for JsonStorageKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > 64 && !(s.len() == 66 && s.starts_with("0x")) {
            return Err(ParseError::BaseConvertError(BaseConvertError::Overflow));
        }

        if let Ok(hash) = B256::from_str(s) {
            return Ok(Self::Hash(hash));
        }
        s.parse().map(Self::Number)
    }
}

impl fmt::Display for JsonStorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(hash) => hash.fmt(f),
            Self::Number(num) => alloc::format!("{num:#x}").fmt(f),
        }
    }
}

/// Converts a Bytes value into a B256, accepting inputs that are less than 32 bytes long. These
/// inputs will be left padded with zeros.
pub fn from_bytes_to_b256<'de, D>(bytes: Bytes) -> Result<B256, D::Error>
where
    D: Deserializer<'de>,
{
    if bytes.0.len() > 32 {
        return Err(serde::de::Error::custom("input too long to be a B256"));
    }

    // left pad with zeros to 32 bytes
    let mut padded = [0u8; 32];
    padded[32 - bytes.0.len()..].copy_from_slice(&bytes.0);

    // then convert to B256 without a panic
    Ok(B256::from_slice(&padded))
}

/// Deserializes the input into a storage map, using [from_bytes_to_b256] which allows cropped
/// values:
///
/// ```json
/// {
///     "0x0000000000000000000000000000000000000000000000000000000000000001": "0x22"
/// }
/// ```
pub fn deserialize_storage_map<'de, D>(
    deserializer: D,
) -> Result<Option<BTreeMap<B256, B256>>, D::Error>
where
    D: Deserializer<'de>,
{
    let map = Option::<BTreeMap<Bytes, Bytes>>::deserialize(deserializer)?;
    match map {
        Some(map) => {
            let mut res_map = BTreeMap::new();
            for (k, v) in map {
                let k_deserialized = from_bytes_to_b256::<'de, D>(k)?;
                let v_deserialized = from_bytes_to_b256::<'de, D>(v)?;
                res_map.insert(k_deserialized, v_deserialized);
            }
            Ok(Some(res_map))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::{String, ToString};
    use serde_json::json;

    #[test]
    fn default_number_storage_key() {
        let key = JsonStorageKey::Number(Default::default());
        assert_eq!(key.to_string(), String::from("0x0"));
    }

    #[test]
    fn default_hash_storage_key() {
        let key = JsonStorageKey::default();
        assert_eq!(
            key.to_string(),
            String::from("0x0000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn test_storage_key() {
        let cases = [
            "0x0000000000000000000000000000000000000000000000000000000000000001", // Hash
            "0000000000000000000000000000000000000000000000000000000000000001",   // Hash
        ];

        let key: JsonStorageKey = serde_json::from_str(&json!(cases[0]).to_string()).unwrap();
        let key2: JsonStorageKey = serde_json::from_str(&json!(cases[1]).to_string()).unwrap();

        assert_eq!(key.as_b256(), key2.as_b256());
    }

    #[test]
    fn test_storage_key_serde_roundtrips() {
        let test_cases = [
            "0x0000000000000000000000000000000000000000000000000000000000000001", // Hash
            "0x0000000000000000000000000000000000000000000000000000000000000abc", // Hash
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",   // Number
            "0xabc",                                                              // Number
            "0xabcd",                                                             // Number
        ];

        for input in test_cases {
            let key: JsonStorageKey = serde_json::from_str(&json!(input).to_string()).unwrap();
            let output = key.to_string();

            assert_eq!(
                input, output,
                "Storage key roundtrip failed to preserve the exact hex representation for {}",
                input
            );
        }
    }

    #[test]
    fn test_as_b256() {
        let cases = [
            "0x0abc",                                                             // Number
            "0x0000000000000000000000000000000000000000000000000000000000000abc", // Hash
        ];

        let num_key: JsonStorageKey = serde_json::from_str(&json!(cases[0]).to_string()).unwrap();
        let hash_key: JsonStorageKey = serde_json::from_str(&json!(cases[1]).to_string()).unwrap();

        assert_eq!(num_key, JsonStorageKey::Number(U256::from_str(cases[0]).unwrap()));
        assert_eq!(hash_key, JsonStorageKey::Hash(B256::from_str(cases[1]).unwrap()));

        assert_eq!(num_key.as_b256(), hash_key.as_b256());
    }

    #[test]
    fn test_json_storage_key_from_b256() {
        let b256_value = B256::from([1u8; 32]);
        let key = JsonStorageKey::from(b256_value);
        assert_eq!(key, JsonStorageKey::Hash(b256_value));
        assert_eq!(
            key.to_string(),
            "0x0101010101010101010101010101010101010101010101010101010101010101"
        );
    }

    #[test]
    fn test_json_storage_key_from_u256() {
        let u256_value = U256::from(42);
        let key = JsonStorageKey::from(u256_value);
        assert_eq!(key, JsonStorageKey::Number(u256_value));
        assert_eq!(key.to_string(), "0x2a");
    }

    #[test]
    fn test_json_storage_key_from_u8_array() {
        let bytes = [0u8; 32];
        let key = JsonStorageKey::from(bytes);
        assert_eq!(key, JsonStorageKey::Hash(B256::from(bytes)));
    }

    #[test]
    fn test_from_str_parsing() {
        let hex_str = "0x0101010101010101010101010101010101010101010101010101010101010101";
        let key = JsonStorageKey::from_str(hex_str).unwrap();
        assert_eq!(key, JsonStorageKey::Hash(B256::from_str(hex_str).unwrap()));
    }

    #[test]
    fn test_from_str_with_too_long_hex_string() {
        let long_hex_str = "0x".to_string() + &"1".repeat(65);
        let result = JsonStorageKey::from_str(&long_hex_str);

        assert!(matches!(result, Err(ParseError::BaseConvertError(BaseConvertError::Overflow))));
    }

    #[test]
    fn test_deserialize_storage_map_with_valid_data() {
        let json_data = json!({
            "0x0000000000000000000000000000000000000000000000000000000000000001": "0x22",
            "0x0000000000000000000000000000000000000000000000000000000000000002": "0x33"
        });

        // Specify the deserialization type explicitly
        let deserialized: Option<BTreeMap<B256, B256>> = deserialize_storage_map(
            &serde_json::from_value::<serde_json::Value>(json_data).unwrap(),
        )
        .unwrap();

        assert_eq!(
            deserialized.unwrap(),
            BTreeMap::from([
                (B256::from(U256::from(1u128)), B256::from(U256::from(0x22u128))),
                (B256::from(U256::from(2u128)), B256::from(U256::from(0x33u128)))
            ])
        );
    }

    #[test]
    fn test_deserialize_storage_map_with_empty_data() {
        let json_data = json!({});
        let deserialized: Option<BTreeMap<B256, B256>> = deserialize_storage_map(
            &serde_json::from_value::<serde_json::Value>(json_data).unwrap(),
        )
        .unwrap();
        assert!(deserialized.unwrap().is_empty());
    }

    #[test]
    fn test_deserialize_storage_map_with_none() {
        let json_data = json!(null);
        let deserialized: Option<BTreeMap<B256, B256>> = deserialize_storage_map(
            &serde_json::from_value::<serde_json::Value>(json_data).unwrap(),
        )
        .unwrap();
        assert!(deserialized.is_none());
    }

    #[test]
    fn test_from_bytes_to_b256_with_valid_input() {
        // Test case with input less than 32 bytes, should be left-padded with zeros
        let bytes = Bytes::from(vec![0x1, 0x2, 0x3, 0x4]);
        let result = from_bytes_to_b256::<serde_json::Value>(bytes).unwrap();
        let expected = B256::from_slice(&[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
            2, 3, 4,
        ]);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_from_bytes_to_b256_with_exact_32_bytes() {
        // Test case with input exactly 32 bytes long
        let bytes = Bytes::from(vec![
            0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xD, 0xE, 0xF, 0x10, 0x11,
            0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
            0x20,
        ]);
        let result = from_bytes_to_b256::<serde_json::Value>(bytes).unwrap();
        let expected = B256::from_slice(&[
            0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xD, 0xE, 0xF, 0x10, 0x11,
            0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
            0x20,
        ]);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_from_bytes_to_b256_with_input_too_long() {
        // Test case with input longer than 32 bytes, should return an error
        let bytes = Bytes::from(vec![0x1; 33]); // 33 bytes long
        let result = from_bytes_to_b256::<serde_json::Value>(bytes);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "input too long to be a B256");
    }

    #[test]
    fn test_from_bytes_to_b256_with_empty_input() {
        // Test case with empty input, should be all zeros
        let bytes = Bytes::from(vec![]);
        let result = from_bytes_to_b256::<serde_json::Value>(bytes).unwrap();
        let expected = B256::from_slice(&[0; 32]); // All zeros
        assert_eq!(result, expected);
    }
}
