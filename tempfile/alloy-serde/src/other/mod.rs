//! Support for capturing other fields.

use alloc::{collections::BTreeMap, format, string::String};
use core::{
    fmt,
    ops::{Deref, DerefMut},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[cfg(any(test, feature = "arbitrary"))]
mod arbitrary_;

/// Generic type for capturing additional fields when deserializing structs.
///
/// For example, the [optimism `eth_getTransactionByHash` request][optimism] returns additional
/// fields that this type will capture instead.
///
/// Use `deserialize_as` or `deserialize_into` with a struct that captures the unknown fields, or
/// deserialize the individual fields manually with `get_deserialized`.
///
/// This type must be used with [`#[serde(flatten)]`][flatten].
///
/// [optimism]: https://docs.alchemy.com/alchemy/apis/optimism/eth-gettransactionbyhash
/// [flatten]: https://serde.rs/field-attrs.html#flatten
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OtherFields {
    inner: BTreeMap<String, serde_json::Value>,
}

impl OtherFields {
    /// Creates a new [`OtherFields`] instance.
    pub const fn new(inner: BTreeMap<String, serde_json::Value>) -> Self {
        Self { inner }
    }

    /// Inserts a given value as serialized [`serde_json::Value`] into the map.
    pub fn insert_value(&mut self, key: String, value: impl Serialize) -> serde_json::Result<()> {
        self.inner.insert(key, serde_json::to_value(value)?);
        Ok(())
    }

    /// Inserts a given value as serialized [`serde_json::Value`] into the map and returns the
    /// updated instance.
    pub fn with_value(mut self, key: String, value: impl Serialize) -> serde_json::Result<Self> {
        self.insert_value(key, value)?;
        Ok(self)
    }

    /// Deserialized this type into another container type.
    pub fn deserialize_as<T: DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_value(Value::Object(self.inner.clone().into_iter().collect()))
    }

    /// Deserialized this type into another container type.
    pub fn deserialize_into<T: DeserializeOwned>(self) -> serde_json::Result<T> {
        serde_json::from_value(serde_json::Value::Object(self.inner.into_iter().collect()))
    }

    /// Returns the deserialized value of the field, if it exists.
    /// Deserializes the value with the given closure
    pub fn get_with<F, V>(&self, key: impl AsRef<str>, with: F) -> Option<V>
    where
        F: FnOnce(serde_json::Value) -> V,
    {
        self.inner.get(key.as_ref()).cloned().map(with)
    }

    /// Returns the deserialized value of the field, if it exists
    pub fn get_deserialized<V: DeserializeOwned>(
        &self,
        key: impl AsRef<str>,
    ) -> Option<serde_json::Result<V>> {
        self.get_with(key, serde_json::from_value)
    }

    /// Returns the deserialized value of the field.
    ///
    /// Returns an error if the field is missing
    pub fn try_get_deserialized<V: DeserializeOwned>(
        &self,
        key: impl AsRef<str>,
    ) -> serde_json::Result<V> {
        let key = key.as_ref();
        self.get_deserialized(key)
            .ok_or_else(|| serde::de::Error::custom(format!("Missing field `{}`", key)))?
    }

    /// Removes the deserialized value of the field, if it exists
    ///
    /// **Note:** this will also remove the value if deserializing it resulted in an error
    pub fn remove_deserialized<V: DeserializeOwned>(
        &mut self,
        key: impl AsRef<str>,
    ) -> Option<serde_json::Result<V>> {
        self.inner.remove(key.as_ref()).map(serde_json::from_value)
    }

    /// Removes the deserialized value of the field, if it exists.
    /// Deserializes the value with the given closure
    ///
    /// **Note:** this will also remove the value if deserializing it resulted in an error
    pub fn remove_with<F, V>(&mut self, key: impl AsRef<str>, with: F) -> Option<V>
    where
        F: FnOnce(serde_json::Value) -> V,
    {
        self.inner.remove(key.as_ref()).map(with)
    }

    /// Removes the deserialized value of the field, if it exists and also returns the key
    ///
    /// **Note:** this will also remove the value if deserializing it resulted in an error
    pub fn remove_entry_deserialized<V: DeserializeOwned>(
        &mut self,
        key: impl AsRef<str>,
    ) -> Option<(String, serde_json::Result<V>)> {
        self.inner
            .remove_entry(key.as_ref())
            .map(|(key, value)| (key, serde_json::from_value(value)))
    }
}

impl fmt::Debug for OtherFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OtherFields ")?;
        self.inner.fmt(f)
    }
}

impl TryFrom<serde_json::Value> for OtherFields {
    type Error = serde_json::Error;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map(Self::new)
    }
}

impl<K> FromIterator<(K, serde_json::Value)> for OtherFields
where
    K: Into<String>,
{
    fn from_iter<T: IntoIterator<Item = (K, serde_json::Value)>>(iter: T) -> Self {
        Self { inner: iter.into_iter().map(|(key, value)| (key.into(), value)).collect() }
    }
}

impl Deref for OtherFields {
    type Target = BTreeMap<String, serde_json::Value>;

    #[inline]
    fn deref(&self) -> &BTreeMap<String, serde_json::Value> {
        self.as_ref()
    }
}

impl DerefMut for OtherFields {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl AsRef<BTreeMap<String, serde_json::Value>> for OtherFields {
    fn as_ref(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.inner
    }
}

impl IntoIterator for OtherFields {
    type Item = (String, serde_json::Value);
    type IntoIter = alloc::collections::btree_map::IntoIter<String, serde_json::Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a> IntoIterator for &'a OtherFields {
    type Item = (&'a String, &'a serde_json::Value);
    type IntoIter = alloc::collections::btree_map::Iter<'a, String, serde_json::Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_ref().iter()
    }
}

/// An extension to a struct that allows to capture additional fields when deserializing.
///
/// See [`OtherFields`] for more information.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub struct WithOtherFields<T> {
    /// The inner struct.
    #[serde(flatten)]
    pub inner: T,
    /// All fields not present in the inner struct.
    #[serde(flatten)]
    pub other: OtherFields,
}

impl<T, U> AsRef<U> for WithOtherFields<T>
where
    T: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        self.inner.as_ref()
    }
}

impl<T> WithOtherFields<T> {
    /// Creates a new [`WithOtherFields`] instance.
    pub fn new(inner: T) -> Self {
        Self { inner, other: Default::default() }
    }
}

impl<T> Deref for WithOtherFields<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for WithOtherFields<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'de, T> Deserialize<'de> for WithOtherFields<T>
where
    T: Deserialize<'de> + Serialize,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WithOtherFieldsHelper<T> {
            #[serde(flatten)]
            inner: T,
            #[serde(flatten)]
            other: OtherFields,
        }

        let mut helper = WithOtherFieldsHelper::deserialize(deserializer)?;
        // remove all fields present in the inner struct from the other fields, this is to avoid
        // duplicate fields in the catch all other fields because serde flatten does not exclude
        // already deserialized fields when deserializing the other fields.
        if let Value::Object(map) =
            serde_json::to_value(&helper.inner).map_err(serde::de::Error::custom)?
        {
            for key in map.keys() {
                helper.other.remove(key);
            }
        }

        Ok(Self { inner: helper.inner, other: helper.other })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use rand::Rng;
    use serde_json::json;
    use similar_asserts::assert_eq;

    #[test]
    fn other_fields_arbitrary() {
        let mut bytes = [0u8; 1024];
        rand::thread_rng().fill(bytes.as_mut_slice());

        let _ = arbitrary::Unstructured::new(&bytes).arbitrary::<OtherFields>().unwrap();
    }

    #[test]
    fn test_correct_other() {
        #[derive(Serialize, Deserialize)]
        struct Inner {
            a: u64,
        }

        #[derive(Serialize, Deserialize)]
        struct InnerWrapper {
            #[serde(flatten)]
            inner: Inner,
        }

        let with_other: WithOtherFields<InnerWrapper> =
            serde_json::from_str("{\"a\": 1, \"b\": 2}").unwrap();
        assert_eq!(with_other.inner.inner.a, 1);
        assert_eq!(
            with_other.other,
            OtherFields::new(BTreeMap::from_iter([("b".to_string(), serde_json::json!(2))]))
        );
    }

    #[test]
    fn test_with_other_fields_serialization() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Inner {
            a: u64,
            b: String,
        }

        let inner = Inner { a: 42, b: "Hello".to_string() };
        let mut other = BTreeMap::new();
        other.insert("extra".to_string(), json!(99));

        let with_other = WithOtherFields { inner, other: OtherFields::new(other.clone()) };
        let serialized = serde_json::to_string(&with_other).unwrap();

        let expected = r#"{"a":42,"b":"Hello","extra":99}"#;
        assert_eq!(serialized, expected);
    }

    #[test]
    fn test_remove_and_access_other_fields() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Inner {
            a: u64,
            b: String,
        }

        let json_data = r#"{"a":42,"b":"Hello","extra":99, "another": "test"}"#;
        let mut with_other: WithOtherFields<Inner> = serde_json::from_str(json_data).unwrap();

        assert_eq!(with_other.other.inner.get("extra"), Some(&json!(99)));
        assert_eq!(with_other.other.inner.get("another"), Some(&json!("test")));

        with_other.other.remove("extra");
        assert!(!with_other.other.inner.contains_key("extra"));
    }

    #[test]
    fn test_deserialize_as() {
        let mut map = BTreeMap::new();
        map.insert("a".to_string(), json!(1));
        let other_fields = OtherFields::new(map);
        let deserialized: Result<BTreeMap<String, u64>, _> = other_fields.deserialize_as();
        assert_eq!(deserialized.unwrap().get("a"), Some(&1));
    }

    #[test]
    fn test_deserialize_into() {
        let mut map = BTreeMap::new();
        map.insert("a".to_string(), json!(1));
        let other_fields = OtherFields::new(map);
        let deserialized: Result<BTreeMap<String, u64>, _> = other_fields.deserialize_into();
        assert_eq!(deserialized.unwrap().get("a"), Some(&1));
    }

    #[test]
    fn test_get_with() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), json!(42));
        let other_fields = OtherFields::new(map);
        let value: Option<u64> = other_fields.get_with("key", |v| v.as_u64().unwrap());
        assert_eq!(value, Some(42));
    }

    #[test]
    fn test_get_deserialized() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), json!(42));
        let other_fields = OtherFields::new(map);
        let value: Option<serde_json::Result<u64>> = other_fields.get_deserialized("key");
        assert_eq!(value.unwrap().unwrap(), 42);
    }

    #[test]
    fn test_remove_deserialized() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), json!(42));
        let mut other_fields = OtherFields::new(map);
        let value: Option<serde_json::Result<u64>> = other_fields.remove_deserialized("key");
        assert_eq!(value.unwrap().unwrap(), 42);
        assert!(!other_fields.inner.contains_key("key"));
    }

    #[test]
    fn test_remove_with() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), json!(42));
        let mut other_fields = OtherFields::new(map);
        let value: Option<u64> = other_fields.remove_with("key", |v| v.as_u64().unwrap());
        assert_eq!(value, Some(42));
        assert!(!other_fields.inner.contains_key("key"));
    }

    #[test]
    fn test_remove_entry_deserialized() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), json!(42));
        let mut other_fields = OtherFields::new(map);
        let entry: Option<(String, serde_json::Result<u64>)> =
            other_fields.remove_entry_deserialized("key");
        assert!(entry.is_some());
        let (key, value) = entry.unwrap();
        assert_eq!(key, "key");
        assert_eq!(value.unwrap(), 42);
        assert!(!other_fields.inner.contains_key("key"));
    }

    #[test]
    fn test_try_from_value() {
        let json_value = json!({ "key": "value" });
        let other_fields = OtherFields::try_from(json_value).unwrap();
        assert_eq!(other_fields.inner.get("key").unwrap(), &json!("value"));
    }

    #[test]
    fn test_into_iter() {
        let mut map = BTreeMap::new();
        map.insert("key1".to_string(), json!("value1"));
        map.insert("key2".to_string(), json!("value2"));
        let other_fields = OtherFields::new(map.clone());

        let iterated_map: BTreeMap<_, _> = other_fields.into_iter().collect();
        assert_eq!(iterated_map, map);
    }
}
