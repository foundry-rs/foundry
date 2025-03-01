//! Serde functions for encoding optional values.

use serde::{Deserialize, Deserializer};

/// For use with serde's `deserialize_with` on a sequence that must be
/// deserialized as a single but optional (i.e. possibly `null`) value.
pub fn null_as_default<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de> + Default,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// For use with serde's `deserialize_with` on a field that must be missing.
pub fn reject_if_some<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let value = Option::<T>::deserialize(deserializer)?;

    if value.is_some() {
        return Err(serde::de::Error::custom("unexpected value"));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestStruct {
        #[serde(default, deserialize_with = "null_as_default")]
        value: Vec<i32>,

        #[serde(default, deserialize_with = "reject_if_some")]
        should_be_none: Option<String>,
    }

    #[test]
    fn test_null_as_default_with_null() {
        let json_data = json!({ "value": null });
        let result: TestStruct = serde_json::from_value(json_data).unwrap();
        assert_eq!(result.value, Vec::<i32>::new());
    }

    #[test]
    fn test_null_as_default_with_value() {
        let json_data = json!({ "value": [1, 2, 3] });
        let result: TestStruct = serde_json::from_value(json_data).unwrap();
        assert_eq!(result.value, vec![1, 2, 3]);
    }

    #[test]
    fn test_null_as_default_with_missing_field() {
        let json_data = json!({});
        let result: TestStruct = serde_json::from_value(json_data).unwrap();
        assert_eq!(result.value, Vec::<i32>::new());
    }

    #[test]
    fn test_reject_if_some_with_none() {
        let json_data = json!({});
        let result: TestStruct = serde_json::from_value(json_data).unwrap();
        assert_eq!(result.should_be_none, None);
    }

    #[test]
    fn test_reject_if_some_with_some() {
        let json_data = json!({ "should_be_none": "unexpected value" });
        let result: Result<TestStruct, _> = serde_json::from_value(json_data);
        assert!(result.is_err());
    }
}
