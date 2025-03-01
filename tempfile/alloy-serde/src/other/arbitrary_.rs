use crate::OtherFields;
use alloc::{collections::BTreeMap, string::String, vec::Vec};

impl arbitrary::Arbitrary<'_> for OtherFields {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let mut inner = BTreeMap::new();
        for _ in 0usize..u.int_in_range(0usize..=15)? {
            inner.insert(u.arbitrary()?, u.arbitrary::<ArbitraryValue>()?.into_json_value());
        }
        Ok(Self { inner })
    }
}

/// Redefinition of `serde_json::Value` for the purpose of implementing `Arbitrary`.
#[derive(Clone, Debug, arbitrary::Arbitrary)]
#[allow(unnameable_types)]
enum ArbitraryValue {
    Null,
    Bool(bool),
    Number(u64),
    String(String),
    Array(Vec<ArbitraryValue>),
    Object(BTreeMap<String, ArbitraryValue>),
}

impl ArbitraryValue {
    fn into_json_value(self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(b),
            Self::Number(n) => serde_json::Value::Number(n.into()),
            Self::String(s) => serde_json::Value::String(s),
            Self::Array(a) => {
                serde_json::Value::Array(a.into_iter().map(Self::into_json_value).collect())
            }
            Self::Object(o) => serde_json::Value::Object(
                o.into_iter().map(|(k, v)| (k, v.into_json_value())).collect(),
            ),
        }
    }
}
