//! Implementations of [`Toml`](spec::Group::Toml) cheatcodes.

use crate::{
    json::{
        canonicalize_json_path, check_json_key_exists, parse_json, parse_json_coerce,
        parse_json_keys,
    },
    Cheatcode, Cheatcodes, Result,
    Vm::*,
};
use alloy_dyn_abi::DynSolType;
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

impl Cheatcode for keyExistsTomlCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        check_json_key_exists(&toml_to_json_string(toml)?, key)
    }
}

impl Cheatcode for parseToml_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        parse_toml(toml, "$")
    }
}

impl Cheatcode for parseToml_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml(toml, key)
    }
}

impl Cheatcode for parseTomlUintCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseTomlUintArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::Uint(256))))
    }
}

impl Cheatcode for parseTomlIntCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseTomlIntArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::Int(256))))
    }
}

impl Cheatcode for parseTomlBoolCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Bool)
    }
}

impl Cheatcode for parseTomlBoolArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::Bool)))
    }
}

impl Cheatcode for parseTomlAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Address)
    }
}

impl Cheatcode for parseTomlAddressArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::Address)))
    }
}

impl Cheatcode for parseTomlStringCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::String)
    }
}

impl Cheatcode for parseTomlStringArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::String)))
    }
}

impl Cheatcode for parseTomlBytesCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseTomlBytesArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::Bytes)))
    }
}

impl Cheatcode for parseTomlBytes32Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for parseTomlBytes32ArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(DynSolType::FixedBytes(32))))
    }
}

impl Cheatcode for parseTomlKeysCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_toml_keys(toml, key)
    }
}

impl Cheatcode for writeToml_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path } = self;
        let value =
            serde_json::from_str(json).unwrap_or_else(|_| JsonValue::String(json.to_owned()));

        let toml_string = format_json_to_toml(value)?;
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

impl Cheatcode for writeToml_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path, valueKey } = self;
        let json =
            serde_json::from_str(json).unwrap_or_else(|_| JsonValue::String(json.to_owned()));

        let data_path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        let toml_data = fs::read_to_string(data_path)?;
        let json_data: JsonValue =
            toml::from_str(&toml_data).map_err(|e| fmt_err!("failed parsing TOML: {e}"))?;
        let value =
            jsonpath_lib::replace_with(json_data, &canonicalize_json_path(valueKey), &mut |_| {
                Some(json.clone())
            })?;

        let toml_string = format_json_to_toml(value)?;
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

/// Parse
fn parse_toml_str(toml: &str) -> Result<TomlValue> {
    toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML: {e}"))
}

/// Parse a TOML string and return the value at the given path.
fn parse_toml(toml: &str, key: &str) -> Result {
    parse_json(&toml_to_json_string(toml)?, key)
}

/// Parse a TOML string and return the value at the given path, coercing it to the given type.
fn parse_toml_coerce(toml: &str, key: &str, ty: &DynSolType) -> Result {
    parse_json_coerce(&toml_to_json_string(toml)?, key, ty)
}

/// Parse a TOML string and return an array of all keys at the given path.
fn parse_toml_keys(toml: &str, key: &str) -> Result {
    parse_json_keys(&toml_to_json_string(toml)?, key)
}

/// Convert a TOML string to a JSON string.
fn toml_to_json_string(toml: &str) -> Result<String> {
    let toml = parse_toml_str(toml)?;
    let json = toml_to_json_value(toml);
    serde_json::to_string(&json).map_err(|e| fmt_err!("failed to serialize JSON: {e}"))
}

/// Format a JSON value to a TOML pretty string.
fn format_json_to_toml(json: JsonValue) -> Result<String> {
    let toml = json_to_toml_value(json);
    toml::to_string_pretty(&toml).map_err(|e| fmt_err!("failed to serialize TOML: {e}"))
}

/// Convert a TOML value to a JSON value.
fn toml_to_json_value(toml: TomlValue) -> JsonValue {
    match toml {
        TomlValue::String(s) => match s.as_str() {
            "null" => JsonValue::Null,
            _ => JsonValue::String(s),
        },
        TomlValue::Integer(i) => JsonValue::Number(i.into()),
        TomlValue::Float(f) => JsonValue::Number(serde_json::Number::from_f64(f).unwrap()),
        TomlValue::Boolean(b) => JsonValue::Bool(b),
        TomlValue::Array(a) => JsonValue::Array(a.into_iter().map(toml_to_json_value).collect()),
        TomlValue::Table(t) => {
            JsonValue::Object(t.into_iter().map(|(k, v)| (k, toml_to_json_value(v))).collect())
        }
        TomlValue::Datetime(d) => JsonValue::String(d.to_string()),
    }
}

/// Convert a JSON value to a TOML value.
fn json_to_toml_value(json: JsonValue) -> TomlValue {
    match json {
        JsonValue::String(s) => TomlValue::String(s),
        JsonValue::Number(n) => match n.as_i64() {
            Some(i) => TomlValue::Integer(i),
            None => match n.as_f64() {
                Some(f) => TomlValue::Float(f),
                None => TomlValue::String(n.to_string()),
            },
        },
        JsonValue::Bool(b) => TomlValue::Boolean(b),
        JsonValue::Array(a) => TomlValue::Array(a.into_iter().map(json_to_toml_value).collect()),
        JsonValue::Object(o) => {
            TomlValue::Table(o.into_iter().map(|(k, v)| (k, json_to_toml_value(v))).collect())
        }
        JsonValue::Null => TomlValue::String("null".to_string()),
    }
}
