//! Implementations of [`Json`](spec::Group::Json) cheatcodes.

use crate::{string, Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::{eip712_parser::EncodeType, DynSolType, DynSolValue, Resolver};
use alloy_primitives::{hex, Address, B256, I256};
use alloy_sol_types::SolValue;
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use serde_json::{Map, Value};
use std::{borrow::Cow, collections::BTreeMap};

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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::Uint(256))))
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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::Int(256))))
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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::Bool)))
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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::Address)))
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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::String)))
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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::Bytes)))
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
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(DynSolType::FixedBytes(32))))
    }
}

impl Cheatcode for parseJsonType_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, typeDescription } = self;
        parse_json_coerce(json, "$", &resolve_type(typeDescription)?).map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseJsonType_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key, typeDescription } = self;
        parse_json_coerce(json, key, &resolve_type(typeDescription)?).map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseJsonTypeArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { json, key, typeDescription } = self;
        let ty = resolve_type(typeDescription)?;
        parse_json_coerce(json, key, &DynSolType::Array(Box::new(ty))).map(|v| v.abi_encode())
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
        *state.serialized_jsons.entry(objectKey.into()).or_default() = serde_json::from_str(value)?;
        Ok(value.abi_encode())
    }
}

impl Cheatcode for serializeBool_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, (*value).into())
    }
}

impl Cheatcode for serializeUint_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, (*value).into())
    }
}

impl Cheatcode for serializeInt_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, (*value).into())
    }
}

impl Cheatcode for serializeAddress_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, (*value).into())
    }
}

impl Cheatcode for serializeBytes32_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, DynSolValue::FixedBytes(*value, 32))
    }
}

impl Cheatcode for serializeString_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, value.clone().into())
    }
}

impl Cheatcode for serializeBytes_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        serialize_json(state, objectKey, valueKey, value.to_vec().into())
    }
}

impl Cheatcode for serializeBool_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(values.iter().copied().map(DynSolValue::Bool).collect()),
        )
    }
}

impl Cheatcode for serializeUint_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(values.iter().map(|v| DynSolValue::Uint(*v, 256)).collect()),
        )
    }
}

impl Cheatcode for serializeInt_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(values.iter().map(|v| DynSolValue::Int(*v, 256)).collect()),
        )
    }
}

impl Cheatcode for serializeAddress_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(values.iter().copied().map(DynSolValue::Address).collect()),
        )
    }
}

impl Cheatcode for serializeBytes32_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(values.iter().map(|v| DynSolValue::FixedBytes(*v, 32)).collect()),
        )
    }
}

impl Cheatcode for serializeString_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(values.iter().cloned().map(DynSolValue::String).collect()),
        )
    }
}

impl Cheatcode for serializeBytes_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, values } = self;
        serialize_json(
            state,
            objectKey,
            valueKey,
            DynSolValue::Array(
                values.iter().cloned().map(Into::into).map(DynSolValue::Bytes).collect(),
            ),
        )
    }
}

impl Cheatcode for serializeJsonType_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { typeDescription, value } = self;
        let ty = resolve_type(typeDescription)?;
        let value = ty.abi_decode(value)?;
        let value = serialize_value_as_json(value)?;
        Ok(value.to_string().abi_encode())
    }
}

impl Cheatcode for serializeJsonType_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, typeDescription, value } = self;
        let ty = resolve_type(typeDescription)?;
        let value = ty.abi_decode(value)?;
        serialize_json(state, objectKey, valueKey, value)
    }
}

impl Cheatcode for serializeUintToHexCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, value } = self;
        let hex = format!("0x{value:x}");
        serialize_json(state, objectKey, valueKey, hex.into())
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
    let json = parse_json_str(json)?;
    let [value] = select(&json, path)?[..] else {
        bail!("path {path:?} must return exactly one JSON value");
    };

    parse_json_as(value, ty).map(|v| v.abi_encode())
}

/// Parses given [serde_json::Value] as a [DynSolValue].
pub(super) fn parse_json_as(value: &Value, ty: &DynSolType) -> Result<DynSolValue> {
    let to_string = |v: &Value| {
        let mut s = v.to_string();
        s.retain(|c: char| c != '"');
        s
    };

    match (value, ty) {
        (Value::Array(array), ty) => parse_json_array(array, ty),
        (Value::Object(object), ty) => parse_json_map(object, ty),
        (Value::String(s), DynSolType::String) => Ok(DynSolValue::String(s.clone())),
        _ => string::parse_value(&to_string(value), ty),
    }
}

pub(super) fn parse_json_array(array: &[Value], ty: &DynSolType) -> Result<DynSolValue> {
    match ty {
        DynSolType::Tuple(types) => {
            ensure!(array.len() == types.len(), "array length mismatch");
            let values = array
                .iter()
                .zip(types)
                .map(|(e, ty)| parse_json_as(e, ty))
                .collect::<Result<Vec<_>>>()?;

            Ok(DynSolValue::Tuple(values))
        }
        DynSolType::Array(inner) => {
            let values =
                array.iter().map(|e| parse_json_as(e, inner)).collect::<Result<Vec<_>>>()?;
            Ok(DynSolValue::Array(values))
        }
        DynSolType::FixedArray(inner, len) => {
            ensure!(array.len() == *len, "array length mismatch");
            let values =
                array.iter().map(|e| parse_json_as(e, inner)).collect::<Result<Vec<_>>>()?;
            Ok(DynSolValue::FixedArray(values))
        }
        _ => bail!("expected {ty}, found array"),
    }
}

pub(super) fn parse_json_map(map: &Map<String, Value>, ty: &DynSolType) -> Result<DynSolValue> {
    let Some((name, fields, types)) = ty.as_custom_struct() else {
        bail!("expected {ty}, found JSON object");
    };

    let mut values = Vec::with_capacity(fields.len());
    for (field, ty) in fields.iter().zip(types.iter()) {
        let Some(value) = map.get(field) else { bail!("field {field:?} not found in JSON object") };
        values.push(parse_json_as(value, ty)?);
    }

    Ok(DynSolValue::CustomStruct {
        name: name.to_string(),
        prop_names: fields.to_vec(),
        tuple: values,
    })
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

/// Converts a JSON [`Value`] to a [`DynSolValue`] by trying to guess encoded type. For safer
/// decoding, use [`parse_json_as`].
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

/// Serializes given [DynSolValue] into a [serde_json::Value].
fn serialize_value_as_json(value: DynSolValue) -> Result<Value> {
    match value {
        DynSolValue::Bool(b) => Ok(Value::Bool(b)),
        DynSolValue::String(s) => {
            // Strings are allowed to contain strigified JSON objects, so we try to parse it like
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
            // let serde handle number parsing
            let n = serde_json::from_str(&i.to_string())?;
            Ok(Value::Number(n))
        }
        DynSolValue::Uint(i, _) => {
            // let serde handle number parsing
            let n = serde_json::from_str(&i.to_string())?;
            Ok(Value::Number(n))
        }
        DynSolValue::Address(a) => Ok(Value::String(a.to_string())),
        DynSolValue::Array(e) | DynSolValue::FixedArray(e) => {
            Ok(Value::Array(e.into_iter().map(serialize_value_as_json).collect::<Result<_>>()?))
        }
        DynSolValue::CustomStruct { name: _, prop_names, tuple } => {
            let values =
                tuple.into_iter().map(serialize_value_as_json).collect::<Result<Vec<_>>>()?;
            let map = prop_names.into_iter().zip(values).collect();

            Ok(Value::Object(map))
        }
        DynSolValue::Tuple(values) => Ok(Value::Array(
            values.into_iter().map(serialize_value_as_json).collect::<Result<_>>()?,
        )),
        DynSolValue::Function(_) => bail!("cannot serialize function pointer"),
    }
}

/// Serializes a key:value pair to a specific object. If the key is valueKey, the value is
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
    value_key: &str,
    value: DynSolValue,
) -> Result {
    let value = serialize_value_as_json(value)?;
    let map = state.serialized_jsons.entry(object_key.into()).or_default();
    map.insert(value_key.into(), value);
    let stringified = serde_json::to_string(map).unwrap();
    Ok(stringified.abi_encode())
}

/// Resolves a [DynSolType] from user input.
fn resolve_type(type_description: &str) -> Result<DynSolType> {
    if let Ok(ty) = DynSolType::parse(type_description) {
        return Ok(ty);
    };

    if let Ok(encoded) = EncodeType::parse(type_description) {
        let main_type = encoded.types[0].type_name;
        let mut resolver = Resolver::default();
        for t in encoded.types {
            resolver.ingest(t.to_owned());
        }

        return Ok(resolver.resolve(main_type)?)
    };

    bail!("type description should be a valid Solidity type or a EIP712 `encodeType` string")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::FixedBytes;
    use proptest::strategy::Strategy;

    fn contains_tuple(value: &DynSolValue) -> bool {
        match value {
            DynSolValue::Tuple(_) | DynSolValue::CustomStruct { .. } => true,
            DynSolValue::Array(v) | DynSolValue::FixedArray(v) => {
                v.first().map_or(false, contains_tuple)
            }
            _ => false,
        }
    }

    /// [DynSolValue::Bytes] of length 32 and 20 are converted to [DynSolValue::FixedBytes] and
    /// [DynSolValue::Address] respectively. Thus, we can't distinguish between address and bytes of
    /// length 20 during decoding. Because of that, there are issues with handling of arrays of
    /// those types.
    fn fixup_guessable(value: DynSolValue) -> DynSolValue {
        match value {
            DynSolValue::Array(mut v) | DynSolValue::FixedArray(mut v) => {
                if let Some(DynSolValue::Bytes(_)) = v.first() {
                    v.retain(|v| {
                        let len = v.as_bytes().unwrap().len();
                        len != 32 && len != 20
                    })
                }
                DynSolValue::Array(v.into_iter().map(fixup_guessable).collect())
            }
            DynSolValue::FixedBytes(v, _) => DynSolValue::FixedBytes(v, 32),
            DynSolValue::Bytes(v) if v.len() == 32 => {
                DynSolValue::FixedBytes(FixedBytes::from_slice(&v), 32)
            }
            DynSolValue::Bytes(v) if v.len() == 20 => DynSolValue::Address(Address::from_slice(&v)),
            _ => value,
        }
    }

    fn guessable_types() -> impl proptest::strategy::Strategy<Value = DynSolValue> {
        proptest::arbitrary::any::<DynSolValue>()
            .prop_map(fixup_guessable)
            .prop_filter("tuples are not supported", |v| !contains_tuple(v))
            .prop_filter("filter out values without type", |v| v.as_type().is_some())
    }

    // Tests to ensure that conversion [DynSolValue] -> [serde_json::Value] -> [DynSolValue]
    proptest::proptest! {
        #[test]
        fn test_json_roundtrip_guessed(v in guessable_types()) {
            let json = serialize_value_as_json(v.clone()).unwrap();
            let value = json_value_to_token(&json).unwrap();

            // do additional abi_encode -> abi_decode to avoid zero signed integers getting decoded as unsigned and causing assert_eq to fail.
            let decoded = v.as_type().unwrap().abi_decode(&value.abi_encode()).unwrap();
            assert_eq!(decoded, v);
        }

        #[test]
        fn test_json_roundtrip(v in proptest::arbitrary::any::<DynSolValue>().prop_filter("filter out values without type", |v| v.as_type().is_some())) {
                let json = serialize_value_as_json(v.clone()).unwrap();
            let value = parse_json_as(&json, &v.as_type().unwrap()).unwrap();
                assert_eq!(value, v);
        }
    }
}
