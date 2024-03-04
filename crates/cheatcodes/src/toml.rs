//! Implementations of [`Toml`](crate::Group::Toml) cheatcodes.

use crate::{
    json::{canonicalize_json_path, parse_json},
    Cheatcode, Cheatcodes, Result,
    Vm::*,
};
use foundry_common::fs;
use foundry_config::fs_permissions::FsAccessKind;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

// TODO: add documentation (`parse-toml`, `write-toml) in Foundry Book
// TODO: add comprehensive tests, including edge cases
// TODO: add upstream support to `forge-std` for the proposed cheatcodes

impl Cheatcode for parseToml_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml } = self;
        let json: JsonValue = toml::from_str(toml).expect("failed to parse TOML");
        let json_string = serde_json::to_string(&json)?;
        parse_json(&json_string, "$")
    }
}

impl Cheatcode for parseToml_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { toml, key } = self;
        let json: JsonValue = toml::from_str(toml).expect("failed to parse TOML");
        let json_string = serde_json::to_string(&json)?;
        parse_json(&json_string, key)
    }
}

impl Cheatcode for writeToml_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { json, path } = self;
        let json =
            serde_json::from_str(json).unwrap_or_else(|_| JsonValue::String(json.to_owned()));
        let toml: TomlValue = serde_json::from_value(json)?;
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

        let toml: TomlValue = serde_json::from_value(value)?;
        let toml_string = toml::to_string_pretty(&toml).expect("failed to serialize TOML");
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}
