//! Implementations of [`Json`](spec::Group::Json) cheatcodes.

use crate::{string, Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{hex, Address, B256, I256};
use alloy_sol_types::SolValue;
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeMap, fmt::Write};

impl Cheatcode for keyExistsCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        check_json_key_exists(json, key)
    }
}

impl Cheatcode for keyExistsJsonCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        check_json_key_exists(json, key)
    }
}

impl Cheatcode for parseJson_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json } = self;
        parse_json(json, "$")
    }
}

impl Cheatcode for parseJson_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json(json, key)
    }
}

impl Cheatcode for parseJsonUintCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseJsonUintArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseJsonIntCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseJsonIntArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseJsonBoolCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Bool)
    }
}

impl Cheatcode for parseJsonBoolArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Bool)
    }
}

impl Cheatcode for parseJsonAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Address)
    }
}

impl Cheatcode for parseJsonAddressArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Address)
    }
}

impl Cheatcode for parseJsonStringCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::String)
    }
}

impl Cheatcode for parseJsonStringArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::String)
    }
}

impl Cheatcode for parseJsonBytesCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseJsonBytesArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseJsonBytes32Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for parseJsonBytes32ArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_coerce(json, key, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for parseJsonKeysCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json_keys(json, key)
    }
}

impl Cheatcode for serializeJsonCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, value } = self;
        serialize_json(state, objectKey, None, value)
    }
}

impl Cheatcode for serializeBool_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), &value.to_string())
    }
}

impl Cheatcode for serializeUint_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), &value.to_string())
    }
}

impl Cheatcode for serializeInt_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), &value.to_string())
    }
}

impl Cheatcode for serializeAddress_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), &value.to_string())
    }
}

impl Cheatcode for serializeBytes32_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), &value.to_string())
    }
}

impl Cheatcode for serializeString_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), value)
    }
}

impl Cheatcode for serializeBytes_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, Some(valueKey), &hex::encode_prefixed(value))
    }
}

impl Cheatcode for serializeBool_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, false))
    }
}

impl Cheatcode for serializeUint_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, false))
    }
}

impl Cheatcode for serializeInt_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, false))
    }
}

impl Cheatcode for serializeAddress_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, true))
    }
}

impl Cheatcode for serializeBytes32_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, true))
    }
}

impl Cheatcode for serializeString_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, true))
    }
}

impl Cheatcode for serializeBytes_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        let values = values.iter().map(hex::encode_prefixed);
        serialize_json(state, objectKey, Some(valueKey), &array_str(values, true))
    }
}

impl Cheatcode for serializeUintToHexCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        let hex = format!("0x{value:x}");
        serialize_json(state, objectKey, Some(valueKey), &hex)
    }
}

impl Cheatcode for writeJson_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path } = self;
        let json = serde_json::from_str(json).unwrap_or_else(|_| Value::String(json.to_owned()));
        let json_string = serde_json::to_string_pretty(&json)?;
        super::fs::write_file(state, path.as_ref(), json_string.as_bytes())
    }
}

impl Cheatcode for writeJson_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path, valueKey } = self;
        let json = serde_json::from_str(json).unwrap_or_else(|_| Value::String(json.to_owned()));

        let data_path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        let data_s = fs::read_to_string(data_path)?;
        let data = serde_json::from_str(&data_s)?;
        let value =
            jsonpath_lib::replace_with(data, &canonicalize_json_path(valueKey), &mut |_| {
                Some(json.clone())
            })?;

        let json_string = serde_json::to_string_pretty(&value)?;
        super::fs::write_file(state, path.as_ref(), json_string.as_bytes())
    }
}

pub(super) fn check_json_key_exists(json: &str, key: &str) -> Result {
    let json = parse_json_str(json)?;
    let values = select(&json, key)?;
    let exists = !values.is_empty();
    Ok(exists.abi_encode())
}

pub(super) fn parse_json(json: &str, path: &str) -> Result {
    let value = parse_json_str(json)?;
    let selected = select(&value, path)?;
    let sol = json_to_sol(&selected)?;
    Ok(encode(sol))
}

pub(super) fn parse_json_coerce(json: &str, path: &str, ty: &DynSolType) -> Result {
    let value = parse_json_str(json)?;
    let values = select(&value, path)?;
    ensure!(!values.is_empty(), "no matching value found at {path:?}");

    ensure!(
        values.iter().all(|value| !value.is_object()),
        "values at {path:?} must not be JSON objects"
    );

    let to_string = |v: &Value| {
        let mut s = v.to_string();
        s.retain(|c: char| c != '"');
        s
    };
    if let Some(array) = values[0].as_array() {
        debug!(target: "cheatcodes", %ty, "parsing array");
        string::parse_array(array.iter().map(to_string), ty)
    } else {
        debug!(target: "cheatcodes", %ty, "parsing string");
        string::parse(&to_string(values[0]), ty)
    }
}

pub(super) fn parse_json_keys(json: &str, key: &str) -> Result {
    let json = parse_json_str(json)?;
    let values = select(&json, key)?;
    let [value] = values[..] else {
        bail!("key {key:?} must return exactly one JSON object");
    };
    let Value::Object(object) = value else {
        bail!("JSON value at {key:?} is not an object");
    };
    let keys = object.keys().collect::<Vec<_>>();
    Ok(keys.abi_encode())
}

fn parse_json_str(json: &str) -> Result<Value> {
    serde_json::from_str(json).map_err(|e| fmt_err!("failed parsing JSON: {e}"))
}

fn json_to_sol(json: &[&Value]) -> Result<Vec<DynSolValue>> {
    let mut sol = Vec::with_capacity(json.len());
    for value in json {
        sol.push(json_value_to_token(value)?);
    }
    Ok(sol)
}

fn select<'a>(value: &'a Value, mut path: &str) -> Result<Vec<&'a Value>> {
    // Handle the special case of the root key
    if path == "." {
        path = "$";
    }
    // format error with debug string because json_path errors may contain newlines
    jsonpath_lib::select(value, &canonicalize_json_path(path))
        .map_err(|e| fmt_err!("failed selecting from JSON: {:?}", e.to_string()))
}

fn encode(values: Vec<DynSolValue>) -> Vec<u8> {
    // Double `abi_encode` is intentional
    let bytes = match &values[..] {
        [] => Vec::new(),
        [one] => one.abi_encode(),
        _ => DynSolValue::Array(values).abi_encode(),
    };
    bytes.abi_encode()
}

/// Canonicalize a json path key to always start from the root of the document.
/// Read more about json path syntax: <https://goessner.net/articles/JsonPath/>
pub(super) fn canonicalize_json_path(path: &str) -> Cow<'_, str> {
    if !path.starts_with('$') {
        format!("${path}").into()
    } else {
        path.into()
    }
}

/// Converts a JSON [`Value`] to a [`DynSolValue`].
///
/// The function is designed to run recursively, so that in case of an object
/// it will call itself to convert each of it's value and encode the whole as a
/// Tuple
#[instrument(target = "cheatcodes", level = "trace", ret)]
pub(super) fn json_value_to_token(value: &Value) -> Result<DynSolValue> {
    match value {
        Value::Null => Ok(DynSolValue::FixedBytes(B256::ZERO, 32)),
        Value::Bool(boolean) => Ok(DynSolValue::Bool(*boolean)),
        Value::Array(array) => {
            array.iter().map(json_value_to_token).collect::<Result<_>>().map(DynSolValue::Array)
        }
        value @ Value::Object(_) => {
            // See: [#3647](https://github.com/foundry-rs/foundry/pull/3647)
            let ordered_object: BTreeMap<String, Value> =
                serde_json::from_value(value.clone()).unwrap();
            ordered_object
                .values()
                .map(json_value_to_token)
                .collect::<Result<_>>()
                .map(DynSolValue::Tuple)
        }
        Value::Number(number) => {
            if let Some(f) = number.as_f64() {
                // Check if the number has decimal digits because the EVM does not support floating
                // point math
                if f.fract() == 0.0 {
                    // Use the string representation of the `serde_json` Number type instead of
                    // calling f.to_string(), because some numbers are wrongly rounded up after
                    // being convented to f64.
                    // Example: 18446744073709551615 becomes 18446744073709552000 after parsing it
                    // to f64.
                    let s = number.to_string();

                    // Coerced to scientific notation, so short-circuit to using fallback.
                    // This will not have a problem with hex numbers, as for parsing these
                    // We'd need to prefix this with 0x.
                    // See also <https://docs.soliditylang.org/en/latest/types.html#rational-and-integer-literals>
                    if s.contains('e') {
                        // Calling Number::to_string with powers of ten formats the number using
                        // scientific notation and causes from_dec_str to fail. Using format! with
                        // f64 keeps the full number representation.
                        // Example: 100000000000000000000 becomes 1e20 when Number::to_string is
                        // used.
                        let fallback_s = f.to_string();
                        if let Ok(n) = fallback_s.parse() {
                            return Ok(DynSolValue::Uint(n, 256));
                        }
                        if let Ok(n) = I256::from_dec_str(&fallback_s) {
                            return Ok(DynSolValue::Int(n, 256));
                        }
                    }

                    if let Ok(n) = s.parse() {
                        return Ok(DynSolValue::Uint(n, 256));
                    }
                    if let Ok(n) = s.parse() {
                        return Ok(DynSolValue::Int(n, 256));
                    }
                }
            }

            Err(fmt_err!("unsupported JSON number: {number}"))
        }
        Value::String(string) => {
            if let Some(mut val) = string.strip_prefix("0x") {
                let s;
                if val.len() % 2 != 0 {
                    s = format!("0{val}");
                    val = &s[..];
                }
                if let Ok(bytes) = hex::decode(val) {
                    return Ok(match bytes.len() {
                        20 => DynSolValue::Address(Address::from_slice(&bytes)),
                        32 => DynSolValue::FixedBytes(B256::from_slice(&bytes), 32),
                        _ => DynSolValue::Bytes(bytes),
                    });
                }
            }
            Ok(DynSolValue::String(string.to_owned()))
        }
    }
}

/// Serializes a key:value pair to a specific object. If the key is Some(valueKey), the value is
/// expected to be an object, which will be set as the root object for the provided object key,
/// overriding the whole root object if the object key already exists. By calling this function
/// multiple times, the user can serialize multiple KV pairs to the same object. The value can be of
/// any type, even a new object in itself. The function will return a stringified version of the
/// object, so that the user can use that as a value to a new invocation of the same function with a
/// new object key. This enables the user to reuse the same function to crate arbitrarily complex
/// object structures (JSON).
fn serialize_json(
    state: &mut Cheatcodes,
    object_key: &str,
    value_key: Option<&str>,
    value: &str,
) -> Result {
    let map = state.serialized_jsons.entry(object_key.into()).or_default();
    if let Some(value_key) = value_key {
        let parsed_value =
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into()));
        map.insert(value_key.into(), parsed_value);
    } else {
        *map = serde_json::from_str(value)
            .map_err(|err| fmt_err!("failed to parse JSON object: {err}"))?;
    }
    let stringified = serde_json::to_string(map).unwrap();
    Ok(stringified.abi_encode())
}

fn array_str<I, T>(values: I, quoted: bool) -> String
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator<Item = T>,
    T: std::fmt::Display,
{
    let iter = values.into_iter();
    let mut s = String::with_capacity(2 + iter.len() * 32);
    s.push('[');
    for (i, item) in iter.enumerate() {
        if i > 0 {
            s.push(',');
        }

        if quoted {
            s.push('"');
        }
        write!(s, "{item}").unwrap();
        if quoted {
            s.push('"');
        }
    }
    s.push(']');
    s
}
