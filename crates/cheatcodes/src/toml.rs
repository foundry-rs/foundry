//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, B256};
use alloy_sol_types::SolValue;
use toml::Value;

impl Cheatcode for parseTomlCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        parse_toml(toml)
    }
}

impl Cheatcode for serializeTomlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { objectKey, value } = self;
        serialize_toml(state, objectKey, None, value)
    }
}

impl Cheatcode for writeTomlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { toml, path } = self;
        let toml = toml::from_str(toml).unwrap_or_else(|_| Value::String(toml.to_owned()));
        let toml_string =
            toml::to_string_pretty(&toml).map_err(|e| fmt_err!("failed serializing TOML: {e}"))?;
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

fn parse_toml(toml: &str) -> Result {
    let toml = toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML: {e}"))?;
    let sol = value_to_token(&toml)?;
    Ok(sol.abi_encode())
}

fn value_to_token(value: &Value) -> Result<DynSolValue> {
    match value {
        Value::Boolean(boolean) => Ok(DynSolValue::Bool(*boolean)),
        Value::Integer(integer) => {
            let s = integer.to_string();

            if let Ok(n) = s.parse() {
                return Ok(DynSolValue::Uint(n, 256))
            }

            if let Ok(n) = s.parse() {
                return Ok(DynSolValue::Int(n, 256))
            }

            Err(fmt_err!("unsupported TOML integer: {integer}"))
        }
        Value::Float(float) => {
            // Check if the number has decimal digits because the EVM does not support floating
            // point math
            if float.fract() == 0.0 {
                let s = float.to_string();

                if s.contains('e') {
                    if let Ok(n) = s.parse() {
                        return Ok(DynSolValue::Uint(n, 256))
                    }
                    if let Ok(n) = s.parse() {
                        return Ok(DynSolValue::Int(n, 256))
                    }
                }
            }

            Err(fmt_err!("unsupported TOML float: {float}"))
        }
        Value::Datetime(datetime) => Err(fmt_err!("unsupported TOML datetime: {datetime}")),
        Value::Array(array) => {
            array.iter().map(value_to_token).collect::<Result<_>>().map(DynSolValue::Array)
        }
        Value::Table(table) => {
            table.values().map(value_to_token).collect::<Result<_>>().map(DynSolValue::Tuple)
        }
        Value::String(string) => {
            if let Some(mut val) = string.strip_prefix("0x") {
                let s;
                if val.len() % 2 != 0 {
                    s = format!("0{}", val);
                    val = &s[..];
                }
                let bytes = hex::decode(val)?;
                Ok(match bytes.len() {
                    20 => DynSolValue::Address(Address::from_slice(&bytes)),
                    32 => DynSolValue::FixedBytes(B256::from_slice(&bytes), 32),
                    _ => DynSolValue::Bytes(bytes),
                })
            } else {
                Ok(DynSolValue::String(string.to_owned()))
            }
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
/// object structures (TOML).
fn serialize_toml(
    state: &mut Cheatcodes,
    object_key: &str,
    value_key: Option<&str>,
    value: &str,
) -> Result {
    let map = state.serialized_tomls.entry(object_key.into()).or_default();
    if let Some(value_key) = value_key {
        let parsed_value = toml::from_str(value).unwrap_or_else(|_| Value::String(value.into()));
        map.insert(value_key.into(), parsed_value);
    } else {
        *map =
            toml::from_str(value).map_err(|err| fmt_err!("failed to parse TOML object: {err}"))?;
    }
    let stringified = toml::to_string(map).unwrap();
    Ok(stringified.abi_encode())
}
