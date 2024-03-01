//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, B256, I256};
use std::collections::BTreeMap;
use toml::Value;

impl Cheatcode for parseTomlCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        parse_toml(toml, "$")
    }
}

impl Cheatcode for serializeTomlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {}
}

impl Cheatcode for writeTomlCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {}
}

fn parse_toml(toml: &str, path: &str) -> Result {
    let value = parse_toml_str(toml)?;
    let sol = toml_to_sol(&value)?;
    Ok(encode(sol))
}

fn parse_toml_str(toml: &str) -> Result<Value> {
    toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML: {e}"))
}

pub fn value_to_token(value: &Value) -> Result<DynSolValue> {
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
