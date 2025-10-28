//! Implementations of [`Json`](spec::Group::Json) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*, string};
use alloy_dyn_abi::{DynSolType, DynSolValue, Resolver, eip712_parser::EncodeType};
use alloy_primitives::{Address, B256, I256, U256, hex};
use alloy_sol_types::SolValue;
use foundry_common::{fmt::StructDefinitions, fs};
use foundry_config::fs_permissions::FsAccessKind;
use serde_json::{Map, Value};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

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
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json } = self;
        parse_json(json, "$", state.struct_defs())
    }
}

impl Cheatcode for parseJson_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, key } = self;
        parse_json(json, key, state.struct_defs())
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
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, typeDescription } = self;
        parse_json_coerce(json, "$", &resolve_type(typeDescription, state.struct_defs())?)
            .map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseJsonType_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, key, typeDescription } = self;
        parse_json_coerce(json, key, &resolve_type(typeDescription, state.struct_defs())?)
            .map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseJsonTypeArrayCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, key, typeDescription } = self;
        let ty = resolve_type(typeDescription, state.struct_defs())?;
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
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { typeDescription, value } = self;
        let ty = resolve_type(typeDescription, state.struct_defs())?;
        let value = ty.abi_decode(value)?;
        let value = foundry_common::fmt::serialize_value_as_json(value, state.struct_defs())?;
        Ok(value.to_string().abi_encode())
    }
}

impl Cheatcode for serializeJsonType_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, valueKey, typeDescription, value } = self;
        let ty = resolve_type(typeDescription, state.struct_defs())?;
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
        let Self { json: value, path, valueKey } = self;

        // Read, parse, and update the JSON object
        let data_path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        let data_string = fs::locked_read_to_string(&data_path)?;
        let mut data =
            serde_json::from_str(&data_string).unwrap_or_else(|_| Value::String(data_string));
        upsert_json_value(&mut data, value, valueKey)?;

        // Write the updated content back to the file
        let json_string = serde_json::to_string_pretty(&data)?;
        super::fs::write_file(state, path.as_ref(), json_string.as_bytes())
    }
}

pub(super) fn check_json_key_exists(json: &str, key: &str) -> Result {
    let json = parse_json_str(json)?;
    let values = select(&json, key)?;
    let exists = !values.is_empty();
    Ok(exists.abi_encode())
}

pub(super) fn parse_json(json: &str, path: &str, defs: Option<&StructDefinitions>) -> Result {
    let value = parse_json_str(json)?;
    let selected = select(&value, path)?;
    let sol = json_to_sol(defs, &selected)?;
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
        (Value::String(s), DynSolType::Uint(_) | DynSolType::Int(_)) => string::parse_value(s, ty),
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

fn json_to_sol(defs: Option<&StructDefinitions>, json: &[&Value]) -> Result<Vec<DynSolValue>> {
    let mut sol = Vec::with_capacity(json.len());
    for value in json {
        sol.push(json_value_to_token(value, defs)?);
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
    if !path.starts_with('$') { format!("${path}").into() } else { path.into() }
}

/// Converts a JSON [`Value`] to a [`DynSolValue`] by trying to guess encoded type. For safer
/// decoding, use [`parse_json_as`].
///
/// The function is designed to run recursively, so that in case of an object
/// it will call itself to convert each of it's value and encode the whole as a
/// Tuple
#[instrument(target = "cheatcodes", level = "trace", ret)]
pub(super) fn json_value_to_token(
    value: &Value,
    defs: Option<&StructDefinitions>,
) -> Result<DynSolValue> {
    if let Some(defs) = defs {
        _json_value_to_token(value, defs)
    } else {
        _json_value_to_token(value, &StructDefinitions::default())
    }
}

fn _json_value_to_token(value: &Value, defs: &StructDefinitions) -> Result<DynSolValue> {
    match value {
        Value::Null => Ok(DynSolValue::FixedBytes(B256::ZERO, 32)),
        Value::Bool(boolean) => Ok(DynSolValue::Bool(*boolean)),
        Value::Array(array) => array
            .iter()
            .map(|v| _json_value_to_token(v, defs))
            .collect::<Result<_>>()
            .map(DynSolValue::Array),
        Value::Object(map) => {
            // Try to find a struct definition that matches the object keys.
            let keys: BTreeSet<_> = map.keys().map(|s| s.as_str()).collect();
            let matching_def = defs.values().find(|fields| {
                fields.len() == keys.len()
                    && fields.iter().map(|(name, _)| name.as_str()).collect::<BTreeSet<_>>() == keys
            });

            if let Some(fields) = matching_def {
                // Found a struct with matching field names, use the order from the definition.
                fields
                    .iter()
                    .map(|(name, _)| {
                        // unwrap is safe because we know the key exists.
                        _json_value_to_token(map.get(name).unwrap(), defs)
                    })
                    .collect::<Result<_>>()
                    .map(DynSolValue::Tuple)
            } else {
                // Fallback to alphabetical sorting if no matching struct is found.
                // See: [#3647](https://github.com/foundry-rs/foundry/pull/3647)
                let ordered_object: BTreeMap<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                ordered_object
                    .values()
                    .map(|value| _json_value_to_token(value, defs))
                    .collect::<Result<_>>()
                    .map(DynSolValue::Tuple)
            }
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
            //  Hanfl hex strings
            if let Some(mut val) = string.strip_prefix("0x") {
                let s;
                if val.len() == 39 {
                    return Err(format!("Cannot parse \"{val}\" as an address. If you want to specify address, prepend zero to the value.").into());
                }
                if !val.len().is_multiple_of(2) {
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

            // Handle large numbers that were potentially encoded as strings because they exceed the
            // capacity of a 64-bit integer.
            // Note that number-like strings that *could* fit in an `i64`/`u64` will fall through
            // and be treated as literal strings.
            if let Ok(n) = string.parse::<I256>()
                && i64::try_from(n).is_err()
            {
                return Ok(DynSolValue::Int(n, 256));
            } else if let Ok(n) = string.parse::<U256>()
                && u64::try_from(n).is_err()
            {
                return Ok(DynSolValue::Uint(n, 256));
            }

            // Otherwise, treat as a regular string
            Ok(DynSolValue::String(string.to_owned()))
        }
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
    let value = foundry_common::fmt::serialize_value_as_json(value, state.struct_defs())?;
    let map = state.serialized_jsons.entry(object_key.into()).or_default();
    map.insert(value_key.into(), value);
    let stringified = serde_json::to_string(map).unwrap();
    Ok(stringified.abi_encode())
}

/// Resolves a [DynSolType] from user input.
pub(super) fn resolve_type(
    type_description: &str,
    struct_defs: Option<&StructDefinitions>,
) -> Result<DynSolType> {
    let ordered_ty = |ty| -> Result<DynSolType> {
        if let Some(defs) = struct_defs { reorder_type(ty, defs) } else { Ok(ty) }
    };

    if let Ok(ty) = DynSolType::parse(type_description) {
        return ordered_ty(ty);
    };

    if let Ok(encoded) = EncodeType::parse(type_description) {
        let main_type = encoded.types[0].type_name;
        let mut resolver = Resolver::default();
        for t in &encoded.types {
            resolver.ingest(t.to_owned());
        }

        // Get the alphabetically-sorted type from the resolver, and reorder if necessary.
        return ordered_ty(resolver.resolve(main_type)?);
    }

    bail!("type description should be a valid Solidity type or a EIP712 `encodeType` string")
}

/// Upserts a value into a JSON object based on a dot-separated key.
///
/// This function navigates through a mutable `serde_json::Value` object using a
/// path-like key. It creates nested JSON objects if they do not exist along the path.
/// The value is inserted at the final key in the path.
///
/// # Arguments
///
/// * `data` - A mutable reference to the `serde_json::Value` to be modified.
/// * `value` - The string representation of the value to upsert. This string is first parsed as
///   JSON, and if that fails, it's treated as a plain JSON string.
/// * `key` - A dot-separated string representing the path to the location for upserting.
pub(super) fn upsert_json_value(data: &mut Value, value: &str, key: &str) -> Result<()> {
    // Parse the path key into segments.
    let canonical_key = canonicalize_json_path(key);
    let parts: Vec<&str> = canonical_key
        .strip_prefix("$.")
        .unwrap_or(key)
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        return Err(fmt_err!("'valueKey' cannot be empty or just '$'"));
    }

    // Separate the final key from the path.
    // Traverse the objects, creating intermediary ones if necessary.
    if let Some((key_to_insert, path_to_parent)) = parts.split_last() {
        let mut current_level = data;

        for segment in path_to_parent {
            if !current_level.is_object() {
                return Err(fmt_err!("path segment '{segment}' does not resolve to an object."));
            }
            current_level = current_level
                .as_object_mut()
                .unwrap()
                .entry(segment.to_string())
                .or_insert(Value::Object(Map::new()));
        }

        // Upsert the new value
        if let Some(parent_obj) = current_level.as_object_mut() {
            parent_obj.insert(
                key_to_insert.to_string(),
                serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned())),
            );
        } else {
            return Err(fmt_err!("final destination is not an object, cannot insert key."));
        }
    }

    Ok(())
}

/// Recursively traverses a `DynSolType` and reorders the fields of any
/// `CustomStruct` variants according to the provided `StructDefinitions`.
///
/// This is necessary because the EIP-712 resolver sorts struct fields alphabetically,
/// but we want to respect the order defined in the Solidity source code.
fn reorder_type(ty: DynSolType, struct_defs: &StructDefinitions) -> Result<DynSolType> {
    match ty {
        DynSolType::CustomStruct { name, prop_names, tuple } => {
            if let Some(def) = struct_defs.get(&name)? {
                // The incoming `prop_names` and `tuple` are alphabetically sorted.
                let type_map: std::collections::HashMap<String, DynSolType> =
                    prop_names.into_iter().zip(tuple).collect();

                let mut sorted_props = Vec::with_capacity(def.len());
                let mut sorted_tuple = Vec::with_capacity(def.len());
                for (field_name, _) in def {
                    sorted_props.push(field_name.clone());
                    if let Some(field_ty) = type_map.get(field_name) {
                        sorted_tuple.push(reorder_type(field_ty.clone(), struct_defs)?);
                    } else {
                        bail!(
                            "mismatch between struct definition and type description: field '{field_name}' not found in provided type for struct '{name}'"
                        );
                    }
                }
                Ok(DynSolType::CustomStruct { name, prop_names: sorted_props, tuple: sorted_tuple })
            } else {
                // No definition found, so we can't reorder. However, we still reorder its children
                // in case they have known structs.
                let new_tuple = tuple
                    .into_iter()
                    .map(|t| reorder_type(t, struct_defs))
                    .collect::<Result<Vec<_>>>()?;
                Ok(DynSolType::CustomStruct { name, prop_names, tuple: new_tuple })
            }
        }
        DynSolType::Array(inner) => {
            Ok(DynSolType::Array(Box::new(reorder_type(*inner, struct_defs)?)))
        }
        DynSolType::FixedArray(inner, len) => {
            Ok(DynSolType::FixedArray(Box::new(reorder_type(*inner, struct_defs)?), len))
        }
        DynSolType::Tuple(inner) => Ok(DynSolType::Tuple(
            inner.into_iter().map(|t| reorder_type(t, struct_defs)).collect::<Result<Vec<_>>>()?,
        )),
        _ => Ok(ty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::FixedBytes;
    use foundry_common::fmt::{TypeDefMap, serialize_value_as_json};
    use proptest::{arbitrary::any, prop_oneof, strategy::Strategy};
    use std::collections::HashSet;

    fn valid_value(value: &DynSolValue) -> bool {
        (match value {
            DynSolValue::String(s) if s == "{}" => false,

            DynSolValue::Tuple(_) | DynSolValue::CustomStruct { .. } => false,

            DynSolValue::Array(v) | DynSolValue::FixedArray(v) => v.iter().all(valid_value),
            _ => true,
        }) && value.as_type().is_some()
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
        any::<DynSolValue>().prop_map(fixup_guessable).prop_filter("invalid value", valid_value)
    }

    /// A proptest strategy for generating a (simple) `DynSolValue::CustomStruct`
    /// and its corresponding `StructDefinitions` object.
    fn custom_struct_strategy() -> impl Strategy<Value = (StructDefinitions, DynSolValue)> {
        // Define a strategy for basic field names and values.
        let field_name_strat = "[a-z]{4,12}";
        let field_value_strat = prop_oneof![
            any::<bool>().prop_map(DynSolValue::Bool),
            any::<u32>().prop_map(|v| DynSolValue::Uint(U256::from(v), 256)),
            any::<[u8; 20]>().prop_map(Address::from).prop_map(DynSolValue::Address),
            any::<[u8; 32]>().prop_map(B256::from).prop_map(|b| DynSolValue::FixedBytes(b, 32)),
            ".*".prop_map(DynSolValue::String),
        ];

        // Combine them to create a list of unique fields that preserve the random order.
        let fields_strat = proptest::collection::vec((field_name_strat, field_value_strat), 1..8)
            .prop_map(|fields| {
                let mut unique_fields = Vec::with_capacity(fields.len());
                let mut seen_names = HashSet::new();
                for (name, value) in fields {
                    if seen_names.insert(name.clone()) {
                        unique_fields.push((name, value));
                    }
                }
                unique_fields
            });

        // Generate the `CustomStruct` and its definition.
        ("[A-Z][a-z]{4,8}", fields_strat).prop_map(|(struct_name, fields)| {
            let (prop_names, tuple): (Vec<String>, Vec<DynSolValue>) =
                fields.clone().into_iter().unzip();
            let def_fields: Vec<(String, String)> = fields
                .iter()
                .map(|(name, value)| (name.clone(), value.as_type().unwrap().to_string()))
                .collect();
            let mut defs_map = TypeDefMap::default();
            defs_map.insert(struct_name.clone(), def_fields);
            (defs_map.into(), DynSolValue::CustomStruct { name: struct_name, prop_names, tuple })
        })
    }

    // Tests to ensure that conversion [DynSolValue] -> [serde_json::Value] -> [DynSolValue]
    proptest::proptest! {
        #[test]
        fn test_json_roundtrip_guessed(v in guessable_types()) {
            let json = serialize_value_as_json(v.clone(), None).unwrap();
            let value = json_value_to_token(&json, None).unwrap();

            // do additional abi_encode -> abi_decode to avoid zero signed integers getting decoded as unsigned and causing assert_eq to fail.
            let decoded = v.as_type().unwrap().abi_decode(&value.abi_encode()).unwrap();
            assert_eq!(decoded, v);
        }

        #[test]
        fn test_json_roundtrip(v in any::<DynSolValue>().prop_filter("filter out values without type", |v| v.as_type().is_some())) {
            let json = serialize_value_as_json(v.clone(), None).unwrap();
            let value = parse_json_as(&json, &v.as_type().unwrap()).unwrap();
            assert_eq!(value, v);
        }

        #[test]
        fn test_json_roundtrip_with_struct_defs((struct_defs, v) in custom_struct_strategy()) {
            let json = serialize_value_as_json(v.clone(), Some(&struct_defs)).unwrap();
            let sol_type = v.as_type().unwrap();
            let parsed_value = parse_json_as(&json, &sol_type).unwrap();
            assert_eq!(parsed_value, v);
        }
    }

    #[test]
    fn test_resolve_type_with_definitions() -> Result<()> {
        // Define a struct with fields in a specific order (not alphabetical)
        let mut struct_defs = TypeDefMap::new();
        struct_defs.insert(
            "Apple".to_string(),
            vec![
                ("color".to_string(), "string".to_string()),
                ("sweetness".to_string(), "uint8".to_string()),
                ("sourness".to_string(), "uint8".to_string()),
            ],
        );
        struct_defs.insert(
            "FruitStall".to_string(),
            vec![
                ("name".to_string(), "string".to_string()),
                ("apples".to_string(), "Apple[]".to_string()),
            ],
        );

        // Simulate resolver output: type string, using alphabetical order for fields.
        let ty_desc = "FruitStall(Apple[] apples,string name)Apple(string color,uint8 sourness,uint8 sweetness)";

        // Resolve type and ensure struct definition order is preserved.
        let ty = resolve_type(ty_desc, Some(&struct_defs.into())).unwrap();
        if let DynSolType::CustomStruct { name, prop_names, tuple } = ty {
            assert_eq!(name, "FruitStall");
            assert_eq!(prop_names, vec!["name", "apples"]);
            assert_eq!(tuple.len(), 2);
            assert_eq!(tuple[0], DynSolType::String);

            if let DynSolType::Array(apple_ty_boxed) = &tuple[1]
                && let DynSolType::CustomStruct { name, prop_names, tuple } = &**apple_ty_boxed
            {
                assert_eq!(*name, "Apple");
                // Check that the inner struct's fields are also in definition order.
                assert_eq!(*prop_names, vec!["color", "sweetness", "sourness"]);
                assert_eq!(
                    *tuple,
                    vec![DynSolType::String, DynSolType::Uint(8), DynSolType::Uint(8)]
                );

                return Ok(());
            }
        }
        panic!("Expected FruitStall and Apple to be CustomStruct");
    }

    #[test]
    fn test_resolve_type_without_definitions() -> Result<()> {
        // Simulate resolver output: type string, using alphabetical order for fields.
        let ty_desc = "Person(bool active,uint256 age,string name)";

        // Resolve the type without providing any struct definitions and ensure that original
        // (alphabetical) order is unchanged.
        let ty = resolve_type(ty_desc, None).unwrap();
        if let DynSolType::CustomStruct { name, prop_names, tuple } = ty {
            assert_eq!(name, "Person");
            assert_eq!(prop_names, vec!["active", "age", "name"]);
            assert_eq!(tuple.len(), 3);
            assert_eq!(tuple, vec![DynSolType::Bool, DynSolType::Uint(256), DynSolType::String]);
            return Ok(());
        }
        panic!("Expected Person to be CustomStruct");
    }

    #[test]
    fn test_resolve_type_for_array_of_structs() -> Result<()> {
        // Define a struct with fields in a specific, non-alphabetical order.
        let mut struct_defs = TypeDefMap::new();
        struct_defs.insert(
            "Item".to_string(),
            vec![
                ("name".to_string(), "string".to_string()),
                ("price".to_string(), "uint256".to_string()),
                ("id".to_string(), "uint256".to_string()),
            ],
        );

        // Simulate resolver output: type string, using alphabetical order for fields.
        let ty_desc = "Item(uint256 id,string name,uint256 price)";

        // Resolve type and ensure struct definition order is preserved.
        let ty = resolve_type(ty_desc, Some(&struct_defs.into())).unwrap();
        let array_ty = DynSolType::Array(Box::new(ty));
        if let DynSolType::Array(item_ty) = array_ty
            && let DynSolType::CustomStruct { name, prop_names, tuple } = *item_ty
        {
            assert_eq!(name, "Item");
            assert_eq!(prop_names, vec!["name", "price", "id"]);
            assert_eq!(
                tuple,
                vec![DynSolType::String, DynSolType::Uint(256), DynSolType::Uint(256)]
            );
            return Ok(());
        }
        panic!("Expected CustomStruct in array");
    }

    #[test]
    fn test_parse_json_missing_field() {
        // Define a struct with a specific field order.
        let mut struct_defs = TypeDefMap::new();
        struct_defs.insert(
            "Person".to_string(),
            vec![
                ("name".to_string(), "string".to_string()),
                ("age".to_string(), "uint256".to_string()),
            ],
        );

        // JSON missing the "age" field
        let json_str = r#"{ "name": "Alice" }"#;

        // Simulate resolver output: type string, using alphabetical order for fields.
        let type_description = "Person(uint256 age,string name)";
        let ty = resolve_type(type_description, Some(&struct_defs.into())).unwrap();

        // Now, attempt to parse the incomplete JSON using the ordered type.
        let json_value: Value = serde_json::from_str(json_str).unwrap();
        let result = parse_json_as(&json_value, &ty);

        // Should fail with a missing field error because `parse_json_map` requires all fields.
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("field \"age\" not found in JSON object"));
    }

    #[test]
    fn test_serialize_json_with_struct_def_order() {
        // Define a struct with a specific, non-alphabetical field order.
        let mut struct_defs = TypeDefMap::new();
        struct_defs.insert(
            "Item".to_string(),
            vec![
                ("name".to_string(), "string".to_string()),
                ("id".to_string(), "uint256".to_string()),
                ("active".to_string(), "bool".to_string()),
            ],
        );

        // Create a DynSolValue instance for the struct.
        let item_struct = DynSolValue::CustomStruct {
            name: "Item".to_string(),
            prop_names: vec!["name".to_string(), "id".to_string(), "active".to_string()],
            tuple: vec![
                DynSolValue::String("Test Item".to_string()),
                DynSolValue::Uint(U256::from(123), 256),
                DynSolValue::Bool(true),
            ],
        };

        // Serialize the value to JSON and verify that the order is preserved.
        let json_value = serialize_value_as_json(item_struct, Some(&struct_defs.into())).unwrap();
        let json_string = serde_json::to_string(&json_value).unwrap();
        assert_eq!(json_string, r#"{"name":"Test Item","id":123,"active":true}"#);
    }

    #[test]
    fn test_json_full_cycle_typed_with_struct_defs() {
        // Define a struct with a specific, non-alphabetical field order.
        let mut struct_defs = TypeDefMap::new();
        struct_defs.insert(
            "Wallet".to_string(),
            vec![
                ("owner".to_string(), "address".to_string()),
                ("balance".to_string(), "uint256".to_string()),
                ("id".to_string(), "bytes32".to_string()),
            ],
        );

        // Create the "original" DynSolValue instance.
        let owner_address = Address::from([1; 20]);
        let wallet_id = B256::from([2; 32]);
        let original_wallet = DynSolValue::CustomStruct {
            name: "Wallet".to_string(),
            prop_names: vec!["owner".to_string(), "balance".to_string(), "id".to_string()],
            tuple: vec![
                DynSolValue::Address(owner_address),
                DynSolValue::Uint(U256::from(5000), 256),
                DynSolValue::FixedBytes(wallet_id, 32),
            ],
        };

        // Serialize it. The resulting JSON should respect the struct definition order.
        let json_value =
            serialize_value_as_json(original_wallet.clone(), Some(&struct_defs.clone().into()))
                .unwrap();
        let json_string = serde_json::to_string(&json_value).unwrap();
        assert_eq!(
            json_string,
            format!(r#"{{"owner":"{owner_address}","balance":5000,"id":"{wallet_id}"}}"#)
        );

        // Resolve the type, which should also respect the struct definition order.
        let type_description = "Wallet(uint256 balance,bytes32 id,address owner)";
        let resolved_type = resolve_type(type_description, Some(&struct_defs.into())).unwrap();

        // Parse the JSON using the correctly ordered resolved type. Ensure that it is identical to
        // the original one.
        let parsed_value = parse_json_as(&json_value, &resolved_type).unwrap();
        assert_eq!(parsed_value, original_wallet);
    }
}
