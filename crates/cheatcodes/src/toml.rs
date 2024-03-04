//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, B256};
use alloy_sol_types::SolValue;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

// TODO: add documentation (`parse-toml`, `write-toml) in Foundry Book
// TODO: add comprehensive tests, including edge cases
// TODO: add upstream support to `forge-std` for the proposed cheatcodes

impl Cheatcode for parseToml_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        let toml = parse_toml_str(toml)?;
        let json = toml_to_json(toml)?;
        let json_string = serde_json::to_string(&json)?;
        parse_json(json_string, "$")
    }
}

impl Cheatcode for parseToml_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        let toml = parse_toml_str(toml)?;
        let json = toml_to_json(toml)?;
        let json_string = serde_json::to_string(&json)?;
        parse_json(json_string, key)
    }
}

impl Cheatcode for writeToml_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path } = self;
        let json = serde_json::from_str(json).unwrap_or_else(|_| Value::String(json.to_owned()));
        let toml = json_to_toml(json)?;
        let toml_string = toml::to_string_pretty(&toml)?;
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

impl Cheatcode for writeToml_1Call {
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

        // TODO: deduplicate

        let toml = json_to_toml(value)?;
        let toml_string = toml::to_string_pretty(&toml)?;
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

fn parse_toml_str(toml: &str) -> Result<TomlValue> {
    toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML: {e}"))
}

/// Convert a TOML value to a JSON value.
fn toml_to_json(value: TomlValue) -> Result<JsonValue> {
    match value {
        TomlValue::String(s) => Ok(JsonValue::String(s)),
        TomlValue::Float(f) => {}
        TomlValue::Integer(i) => {}
        TomlValue::Boolean(b) => Ok(JsonValue::Bool(b)),
        TomlValue::Datetime(d) => Ok(JsonValue::String(d.to_string())),
        TomlValue::Array(a) => {
            Ok(JsonValue::Array(a.into_iter().map(|v| toml_to_json(v).unwrap()).collect()))
        }
        TomlValue::Table(t) => Ok(JsonValue::Object(
            t.into_iter().map(|(k, v)| (k, toml_to_json(v).unwrap())).collect(),
        )),
    }
}

/// Convert a JSON value to a TOML value.
fn json_to_toml(value: JsonValue) -> Result<TomlValue> {
    match value {
        JsonValue::Null => {}
        JsonValue::Bool(b) => Ok(TomlValue::Boolean(b)),
        JsonValue::Number(n) => {}
        JsonValue::String(s) => Ok(TomlValue::String(s)),
        JsonValue::Array(a) => {
            Ok(TomlValue::Array(a.into_iter().map(|v| json_to_toml(v).unwrap()).collect()))
        }
        JsonValue::Object(o) => Ok(TomlValue::Table(
            o.into_iter().map(|(k, v)| (k, json_to_toml(v).unwrap())).collect(),
        )),
    }
}
