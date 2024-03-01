//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, B256, I256};
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
        // Integer
        // Float
        // Datetime
        // Array
        // Table
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
