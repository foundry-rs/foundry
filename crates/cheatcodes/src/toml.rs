//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

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
        check_json_key_exists(&convert_toml_to_json(toml)?, key)
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
        parse_toml_coerce(toml, key, &DynSolType::Uint(256))
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
        parse_toml_coerce(toml, key, &DynSolType::Int(256))
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
        parse_toml_coerce(toml, key, &DynSolType::Bool)
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
        parse_toml_coerce(toml, key, &DynSolType::Address)
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
        parse_toml_coerce(toml, key, &DynSolType::String)
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
        parse_toml_coerce(toml, key, &DynSolType::Bytes)
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
        parse_toml_coerce(toml, key, &DynSolType::FixedBytes(32))
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

/// Parse a TOML string and return the value at the given path.
fn parse_toml(toml: &str, key: &str) -> Result {
    parse_json(&convert_toml_to_json(toml)?, key)
}

/// Parse a TOML string and return the value at the given path, coercing it to the given type.
fn parse_toml_coerce(toml: &str, key: &str, ty: &DynSolType) -> Result {
    parse_json_coerce(&convert_toml_to_json(toml)?, key, ty)
}

/// Parse a TOML string and return an array of all keys at the given path.
fn parse_toml_keys(toml: &str, key: &str) -> Result {
    parse_json_keys(&convert_toml_to_json(toml)?, key)
}

/// Convert a TOML string to a JSON string.
fn convert_toml_to_json(toml: &str) -> Result<String> {
    let json: JsonValue =
        toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML into JSON: {e}"))?;
    serde_json::to_string(&json).map_err(|e| fmt_err!("failed to serialize JSON: {e}"))
}

/// Format a JSON value to a TOML pretty string.
fn format_json_to_toml(json: JsonValue) -> Result<String> {
    let toml: TomlValue =
        serde_json::from_value(json).map_err(|e| fmt_err!("failed parsing JSON into TOML: {e}"))?;
    toml::to_string_pretty(&toml).map_err(|e| fmt_err!("failed to serialize TOML: {e}"))
}
