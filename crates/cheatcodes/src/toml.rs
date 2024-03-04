//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{json::serialize_json, Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, B256};
use toml::Value;

// TODO: add documentation (`parse-toml`, `serialize-toml`, `write-toml) in Foundry Book
// TODO: add comprehensive tests, including edge cases
// TODO: add upstream support to `forge-std` for the proposed cheatcodes
// TODO: make sure this is the correct way of implementing new cheatcodes, incl. specification
// TODO: make sure serialization and deserialization is correct

impl Cheatcode for parseTomlCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        parse_toml(toml)
    }
}

impl Cheatcode for serializeTomlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        // To allow for TOML manipulation we re-use the existing JSON serialization mechanism.
        // This avoids the need to implement a parallel implementation including cheatcodes.
        let Self { objectKey, value } = self;
        serialize_json(state, objectKey, None, value)
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
