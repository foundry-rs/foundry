//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{
    json::{canonicalize_json_path, parse_json, parse_json_coerce},
    Cheatcode, Cheatcodes, Result,
    Vm::*,
};
use alloy_dyn_abi::DynSolType;
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use serde_json::{Number, Value as JsonValue};
use toml::Value as TomlValue;

// TODO: add documentation (`parse-toml`, `write-toml) in Foundry Book
// TODO: add comprehensive tests, including edge cases
// TODO: add upstream support to `forge-std` for the proposed cheatcodes

impl Cheatcode for parseToml_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        parse_json(&convert(&toml)?, "$")
    }
}

impl Cheatcode for parseToml_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json(&convert(&toml)?, key)
    }
}

impl Cheatcode for parseTomlUintCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseTomlUintArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseTomlIntCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseTomlIntArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseTomlBoolCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Bool)
    }
}

impl Cheatcode for parseTomlBoolArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Bool)
    }
}

impl Cheatcode for parseTomlAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Address)
    }
}

impl Cheatcode for parseTomlAddressArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Address)
    }
}

impl Cheatcode for parseTomlStringCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::String)
    }
}

impl Cheatcode for parseTomlStringArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::String)
    }
}

impl Cheatcode for parseTomlBytesCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseTomlBytesArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseTomlBytes32Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for parseTomlBytes32ArrayCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        parse_json_coerce(&convert(&toml)?, key, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for writeToml_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path } = self;
        let value =
            serde_json::from_str(json).unwrap_or_else(|_| JsonValue::String(json.to_owned()));
        let toml = json_to_toml(value);
        let toml_string = toml::to_string_pretty(&toml).expect("failed to serialize TOML");
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

impl Cheatcode for writeToml_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path, valueKey } = self;
        let json =
            serde_json::from_str(json).unwrap_or_else(|_| JsonValue::String(json.to_owned()));

        let data_path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        let data_s = fs::read_to_string(data_path)?;
        let data = serde_json::from_str(&data_s)?;
        let value =
            jsonpath_lib::replace_with(data, &canonicalize_json_path(valueKey), &mut |_| {
                Some(json.clone())
            })?;

        let toml = json_to_toml(value);
        let toml_string = toml::to_string_pretty(&toml).expect("failed to serialize TOML");
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

/// Convert a TOML string to a JSON string.
fn convert(toml: &str) -> Result<String> {
    let toml = toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML: {e}"))?;
    let json = toml_to_json(toml);
    serde_json::to_string(&json).map_err(|e| fmt_err!("failed to convert to JSON: {e}"))
}

/// Convert a TOML value to a JSON value.
fn toml_to_json(value: TomlValue) -> JsonValue {
    info!("toml_to_json: value={:?}", value);

    match value {
        TomlValue::String(s) => JsonValue::String(s),
        TomlValue::Integer(i) => JsonValue::Number(Number::from(i)),
        TomlValue::Float(f) => {
            JsonValue::Number(Number::from_f64(f).expect("failed to convert float"))
        }
        TomlValue::Boolean(b) => JsonValue::Bool(b),
        TomlValue::Array(arr) => JsonValue::Array(arr.into_iter().map(toml_to_json).collect()),
        TomlValue::Table(table) => {
            JsonValue::Object(table.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect())
        }
        TomlValue::Datetime(d) => JsonValue::String(d.to_string()),
    }
}

/// Convert a JSON value to a TOML value.
fn json_to_toml(value: JsonValue) -> TomlValue {
    match value {
        JsonValue::String(s) => TomlValue::String(s),
        JsonValue::Number(n) => TomlValue::Integer(n.as_i64().expect("failed to convert integer")),
        JsonValue::Bool(b) => TomlValue::Boolean(b),
        JsonValue::Array(arr) => TomlValue::Array(arr.into_iter().map(json_to_toml).collect()),
        JsonValue::Object(obj) => {
            TomlValue::Table(obj.into_iter().map(|(k, v)| (k, json_to_toml(v))).collect())
        }
        JsonValue::Null => TomlValue::String("null".to_owned()),
    }
}
