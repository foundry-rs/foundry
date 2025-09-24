use super::{format_int_exp, format_uint_exp};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::hex;
use eyre::Result;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

/// [`DynSolValue`] formatter.
struct DynValueFormatter {
    raw: bool,
}

impl DynValueFormatter {
    /// Recursively formats a [`DynSolValue`].
    fn value(&self, value: &DynSolValue, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match value {
            DynSolValue::Address(inner) => write!(f, "{inner}"),
            DynSolValue::Function(inner) => write!(f, "{inner}"),
            DynSolValue::Bytes(inner) => f.write_str(&hex::encode_prefixed(inner)),
            DynSolValue::FixedBytes(word, size) => {
                f.write_str(&hex::encode_prefixed(&word[..*size]))
            }
            DynSolValue::Uint(inner, _) => {
                if self.raw {
                    write!(f, "{inner}")
                } else {
                    f.write_str(&format_uint_exp(*inner))
                }
            }
            DynSolValue::Int(inner, _) => {
                if self.raw {
                    write!(f, "{inner}")
                } else {
                    f.write_str(&format_int_exp(*inner))
                }
            }
            DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
                f.write_str("[")?;
                self.list(values, f)?;
                f.write_str("]")
            }
            DynSolValue::Tuple(values) => self.tuple(values, f),
            DynSolValue::String(inner) => {
                if self.raw {
                    write!(f, "{}", inner.escape_debug())
                } else {
                    write!(f, "{inner:?}") // escape strings
                }
            }
            DynSolValue::Bool(inner) => write!(f, "{inner}"),
            DynSolValue::CustomStruct { name, prop_names, tuple } => {
                if self.raw {
                    return self.tuple(tuple, f);
                }

                f.write_str(name)?;

                if prop_names.len() == tuple.len() {
                    f.write_str("({ ")?;

                    for (i, (prop_name, value)) in std::iter::zip(prop_names, tuple).enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        f.write_str(prop_name)?;
                        f.write_str(": ")?;
                        self.value(value, f)?;
                    }

                    f.write_str(" })")
                } else {
                    self.tuple(tuple, f)
                }
            }
        }
    }

    /// Recursively formats a comma-separated list of [`DynSolValue`]s.
    fn list(&self, values: &[DynSolValue], f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            self.value(value, f)?;
        }
        Ok(())
    }

    /// Formats the given values as a tuple.
    fn tuple(&self, values: &[DynSolValue], f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        self.list(values, f)?;
        f.write_str(")")
    }
}

/// Wrapper that implements [`Display`](fmt::Display) for a [`DynSolValue`].
struct DynValueDisplay<'a> {
    /// The value to display.
    value: &'a DynSolValue,
    /// The formatter.
    formatter: DynValueFormatter,
}

impl<'a> DynValueDisplay<'a> {
    /// Creates a new [`Display`](fmt::Display) wrapper for the given value.
    fn new(value: &'a DynSolValue, raw: bool) -> Self {
        Self { value, formatter: DynValueFormatter { raw } }
    }
}

impl fmt::Display for DynValueDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.formatter.value(self.value, f)
    }
}

/// Parses string input as Token against the expected ParamType
pub fn parse_tokens<'a, I: IntoIterator<Item = (&'a DynSolType, &'a str)>>(
    params: I,
) -> alloy_dyn_abi::Result<Vec<DynSolValue>> {
    params.into_iter().map(|(param, value)| DynSolType::coerce_str(param, value)).collect()
}

/// Pretty-prints a slice of tokens using [`format_token`].
pub fn format_tokens(tokens: &[DynSolValue]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token)
}

/// Pretty-prints a slice of tokens using [`format_token_raw`].
pub fn format_tokens_raw(tokens: &[DynSolValue]) -> impl Iterator<Item = String> + '_ {
    tokens.iter().map(format_token_raw)
}

/// Pretty-prints the given value into a string suitable for user output.
pub fn format_token(value: &DynSolValue) -> String {
    DynValueDisplay::new(value, false).to_string()
}

/// Pretty-prints the given value into a string suitable for re-parsing as values later.
///
/// This means:
/// - integers are not formatted with exponential notation hints
/// - structs are formatted as tuples, losing the struct and property names
pub fn format_token_raw(value: &DynSolValue) -> String {
    DynValueDisplay::new(value, true).to_string()
}

/// Serializes given [DynSolValue] into a [serde_json::Value].
pub fn serialize_value_as_json(
    value: DynSolValue,
    defs: Option<&StructDefinitions>,
) -> Result<Value> {
    if let Some(defs) = defs {
        _serialize_value_as_json(value, defs)
    } else {
        _serialize_value_as_json(value, &StructDefinitions::default())
    }
}

fn _serialize_value_as_json(value: DynSolValue, defs: &StructDefinitions) -> Result<Value> {
    match value {
        DynSolValue::Bool(b) => Ok(Value::Bool(b)),
        DynSolValue::String(s) => {
            // Strings are allowed to contain stringified JSON objects, so we try to parse it like
            // one first.
            if let Ok(map) = serde_json::from_str(&s) {
                Ok(Value::Object(map))
            } else {
                Ok(Value::String(s))
            }
        }
        DynSolValue::Bytes(b) => Ok(Value::String(hex::encode_prefixed(b))),
        DynSolValue::FixedBytes(b, size) => Ok(Value::String(hex::encode_prefixed(&b[..size]))),
        DynSolValue::Int(i, _) => {
            if let Ok(n) = i64::try_from(i) {
                // Use `serde_json::Number` if the number can be accurately represented.
                Ok(Value::Number(n.into()))
            } else {
                // Otherwise, fallback to its string representation to preserve precision and ensure
                // compatibility with alloy's `DynSolType` coercion.
                Ok(Value::String(i.to_string()))
            }
        }
        DynSolValue::Uint(i, _) => {
            if let Ok(n) = u64::try_from(i) {
                // Use `serde_json::Number` if the number can be accurately represented.
                Ok(Value::Number(n.into()))
            } else {
                // Otherwise, fallback to its string representation to preserve precision and ensure
                // compatibility with alloy's `DynSolType` coercion.
                Ok(Value::String(i.to_string()))
            }
        }
        DynSolValue::Address(a) => Ok(Value::String(a.to_string())),
        DynSolValue::Array(e) | DynSolValue::FixedArray(e) => Ok(Value::Array(
            e.into_iter().map(|v| _serialize_value_as_json(v, defs)).collect::<Result<_>>()?,
        )),
        DynSolValue::CustomStruct { name, prop_names, tuple } => {
            let values = tuple
                .into_iter()
                .map(|v| _serialize_value_as_json(v, defs))
                .collect::<Result<Vec<_>>>()?;
            let mut map: HashMap<String, Value> = prop_names.into_iter().zip(values).collect();

            // If the struct def is known, manually build a `Map` to preserve the order.
            if let Some(fields) = defs.get(&name)? {
                let mut ordered_map = Map::with_capacity(fields.len());
                for (field_name, _) in fields {
                    if let Some(serialized_value) = map.remove(field_name) {
                        ordered_map.insert(field_name.clone(), serialized_value);
                    }
                }
                // Explicitly return a `Value::Object` to avoid ambiguity.
                return Ok(Value::Object(ordered_map));
            }

            // Otherwise, fall back to alphabetical sorting for deterministic output.
            Ok(Value::Object(map.into_iter().collect::<Map<String, Value>>()))
        }
        DynSolValue::Tuple(values) => Ok(Value::Array(
            values.into_iter().map(|v| _serialize_value_as_json(v, defs)).collect::<Result<_>>()?,
        )),
        DynSolValue::Function(_) => eyre::bail!("cannot serialize function pointer"),
    }
}

// -- STRUCT DEFINITIONS -------------------------------------------------------

pub type TypeDefMap = BTreeMap<String, Vec<(String, String)>>;

#[derive(Debug, Clone, Default)]
pub struct StructDefinitions(TypeDefMap);

impl From<TypeDefMap> for StructDefinitions {
    fn from(map: TypeDefMap) -> Self {
        Self::new(map)
    }
}

impl StructDefinitions {
    pub fn new(map: TypeDefMap) -> Self {
        Self(map)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &[(String, String)]> {
        self.0.values().map(|v| v.as_slice())
    }

    pub fn get(&self, key: &str) -> eyre::Result<Option<&[(String, String)]>> {
        if let Some(value) = self.0.get(key) {
            return Ok(Some(value));
        }

        let matches: Vec<&[(String, String)]> = self
            .0
            .iter()
            .filter_map(|(k, v)| {
                if let Some((_, struct_name)) = k.split_once('.')
                    && struct_name == key
                {
                    return Some(v.as_slice());
                }
                None
            })
            .collect();

        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some(matches[0])),
            _ => eyre::bail!(
                "there are several structs with the same name. Use `<contract_name>.{key}` instead."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{U256, address};

    #[test]
    fn parse_hex_uint() {
        let ty = DynSolType::Uint(256);

        let values = parse_tokens(std::iter::once((&ty, "100"))).unwrap();
        assert_eq!(values, [DynSolValue::Uint(U256::from(100), 256)]);

        let val: U256 = U256::from(100u64);
        let hex_val = format!("0x{val:x}");
        let values = parse_tokens(std::iter::once((&ty, hex_val.as_str()))).unwrap();
        assert_eq!(values, [DynSolValue::Uint(U256::from(100), 256)]);
    }

    #[test]
    fn format_addr() {
        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md
        assert_eq!(
            format_token(&DynSolValue::Address(address!(
                "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
            ))),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
        );

        // copied from testcases in https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1191.md
        assert_ne!(
            format_token(&DynSolValue::Address(address!(
                "0xFb6916095cA1Df60bb79ce92cE3EA74c37c5d359"
            ))),
            "0xFb6916095cA1Df60bb79ce92cE3EA74c37c5d359"
        );
    }
}
