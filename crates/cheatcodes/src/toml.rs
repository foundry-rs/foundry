//! Implementations of [`Toml`](spec::Group::Toml) cheatcodes.

use crate::{
    Cheatcode, Cheatcodes, Result,
    Vm::*,
    json::{
        check_json_key_exists, parse_json, parse_json_coerce, parse_json_coerce_default,
        parse_json_keys, resolve_type, split_value_key,
    },
};
use alloy_dyn_abi::DynSolType;
use alloy_sol_types::SolValue;
use foundry_common::{fmt::StructDefinitions, fs};
use foundry_config::fs_permissions::FsAccessKind;
use foundry_evm_core::evm::FoundryEvmNetwork;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;
use toml_edit::{DocumentMut, Item, Table, TableLike};

impl Cheatcode for keyExistsTomlCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml, key } = self;
        check_json_key_exists(&toml_to_json_string(toml)?, key)
    }
}

impl Cheatcode for parseToml_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml } = self;
        parse_toml(
            toml,
            "$",
            state.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok()),
        )
    }
}

impl Cheatcode for parseToml_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml, key } = self;
        parse_toml(
            toml,
            key,
            state.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok()),
        )
    }
}

macro_rules! impl_parse_toml {
    ($call:ident, $call_with_default:ident, $ty:expr) => {
        impl Cheatcode for $call {
            fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
                let Self { toml, key } = self;
                parse_toml_coerce(toml, key, &$ty)
            }
        }

        impl Cheatcode for $call_with_default {
            fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
                let Self { toml, key, defaultValue } = self;
                parse_toml_coerce_default(toml, key, &$ty, defaultValue)
            }
        }
    };
}

impl_parse_toml!(parseTomlUint_0Call, parseTomlUint_1Call, DynSolType::Uint(256));
impl_parse_toml!(
    parseTomlUintArray_0Call,
    parseTomlUintArray_1Call,
    DynSolType::Array(Box::new(DynSolType::Uint(256)))
);
impl_parse_toml!(parseTomlInt_0Call, parseTomlInt_1Call, DynSolType::Int(256));
impl_parse_toml!(
    parseTomlIntArray_0Call,
    parseTomlIntArray_1Call,
    DynSolType::Array(Box::new(DynSolType::Int(256)))
);
impl_parse_toml!(parseTomlBool_0Call, parseTomlBool_1Call, DynSolType::Bool);
impl_parse_toml!(
    parseTomlBoolArray_0Call,
    parseTomlBoolArray_1Call,
    DynSolType::Array(Box::new(DynSolType::Bool))
);
impl_parse_toml!(parseTomlAddress_0Call, parseTomlAddress_1Call, DynSolType::Address);
impl_parse_toml!(
    parseTomlAddressArray_0Call,
    parseTomlAddressArray_1Call,
    DynSolType::Array(Box::new(DynSolType::Address))
);
impl_parse_toml!(parseTomlString_0Call, parseTomlString_1Call, DynSolType::String);
impl_parse_toml!(
    parseTomlStringArray_0Call,
    parseTomlStringArray_1Call,
    DynSolType::Array(Box::new(DynSolType::String))
);
impl_parse_toml!(parseTomlBytes_0Call, parseTomlBytes_1Call, DynSolType::Bytes);
impl_parse_toml!(
    parseTomlBytesArray_0Call,
    parseTomlBytesArray_1Call,
    DynSolType::Array(Box::new(DynSolType::Bytes))
);
impl_parse_toml!(parseTomlBytes32_0Call, parseTomlBytes32_1Call, DynSolType::FixedBytes(32));
impl_parse_toml!(
    parseTomlBytes32Array_0Call,
    parseTomlBytes32Array_1Call,
    DynSolType::Array(Box::new(DynSolType::FixedBytes(32)))
);

impl Cheatcode for parseTomlType_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml, typeDescription } = self;
        parse_toml_coerce(
            toml,
            "$",
            &resolve_type(
                typeDescription,
                state.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok()),
            )?,
        )
        .map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseTomlType_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml, key, typeDescription } = self;
        parse_toml_coerce(
            toml,
            key,
            &resolve_type(
                typeDescription,
                state.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok()),
            )?,
        )
        .map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseTomlTypeArrayCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml, key, typeDescription } = self;
        let ty = resolve_type(
            typeDescription,
            state.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok()),
        )?;
        parse_toml_coerce(toml, key, &DynSolType::Array(Box::new(ty))).map(|v| v.abi_encode())
    }
}

impl Cheatcode for parseTomlKeysCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { toml, key } = self;
        parse_toml_keys(toml, key)
    }
}

impl Cheatcode for writeToml_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { json, path } = self;
        let value =
            serde_json::from_str(json).unwrap_or_else(|_| JsonValue::String(json.to_owned()));

        let toml_string = format_json_to_toml(value)?;
        super::fs::write_file(state, path.as_ref(), toml_string.as_bytes())
    }
}

impl Cheatcode for writeToml_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { json: value, path, valueKey } = self;

        // Read and parse the TOML file, keeping its formatting and comments.
        // If the file doesn't exist, start with an empty document so the file is created.
        let data_path = state.config.ensure_path_allowed(path, FsAccessKind::Read)?;
        let mut document = if data_path.exists() {
            parse_toml_document(&fs::locked_read_to_string(&data_path)?)?
        } else {
            DocumentMut::new()
        };
        upsert_toml_value(&mut document, value, valueKey)?;

        super::fs::write_file(state, path.as_ref(), document.to_string().as_bytes())
    }
}

/// Parse
fn parse_toml_str(toml: &str) -> Result<TomlValue> {
    toml::from_str(toml).map_err(|e| fmt_err!("failed parsing TOML: {e}"))
}

/// Parse a TOML string and return the value at the given path.
fn parse_toml(toml: &str, key: &str, struct_defs: Option<&StructDefinitions>) -> Result {
    parse_json(&toml_to_json_string(toml)?, key, struct_defs)
}

/// Parse a TOML string and return the value at the given path, coercing it to the given type.
fn parse_toml_coerce(toml: &str, key: &str, ty: &DynSolType) -> Result {
    parse_json_coerce(&toml_to_json_string(toml)?, key, ty)
}

/// Parse a TOML string and return the value at the given path, coercing it to the given type, or
/// return the default if the path does not exist.
fn parse_toml_coerce_default<T: SolValue>(
    toml: &str,
    key: &str,
    ty: &DynSolType,
    default: &T,
) -> Result {
    parse_json_coerce_default(&toml_to_json_string(toml)?, key, ty, default)
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
pub(super) fn toml_to_json_value(toml: TomlValue) -> JsonValue {
    match toml {
        TomlValue::String(s) => match s.as_str() {
            "null" => JsonValue::Null,
            _ => JsonValue::String(s),
        },
        TomlValue::Integer(i) => JsonValue::Number(i.into()),
        TomlValue::Float(f) => match serde_json::Number::from_f64(f) {
            Some(n) => JsonValue::Number(n),
            None => JsonValue::String(f.to_string()),
        },
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

/// Parses a TOML string into a document that keeps its formatting and comments.
fn parse_toml_document(toml: &str) -> Result<DocumentMut> {
    toml.parse().map_err(|e| fmt_err!("failed parsing TOML: {e}"))
}

/// Inserts or replaces the value at `key` in a TOML document, creating intermediate tables if
/// necessary.
///
/// Only the item at `key` is rewritten, so comments and formatting elsewhere in the document are
/// kept.
fn upsert_toml_value(document: &mut DocumentMut, value: &str, key: &str) -> Result<()> {
    let parts = split_value_key(key)?;

    // Separate the final key from the path.
    // Traverse the tables, creating implicit intermediary ones if necessary.
    if let Some((key_to_insert, path_to_parent)) = parts.split_last() {
        let mut current_level = document.as_item_mut();

        for segment in path_to_parent {
            let is_inline = current_level.is_inline_table();
            let Some(table) = current_level.as_table_like_mut() else {
                return Err(fmt_err!("path segment '{segment}' does not resolve to an object."));
            };
            if !table.contains_key(segment) {
                let mut intermediary = Table::new();
                intermediary.set_implicit(true);
                insert_toml_item(table, is_inline, segment, Item::Table(intermediary));
            }
            current_level = table.get_mut(segment).unwrap();
        }

        let is_inline = current_level.is_inline_table();
        let Some(parent) = current_level.as_table_like_mut() else {
            return Err(fmt_err!("final destination is not an object, cannot insert key."));
        };

        let value =
            serde_json::from_str(value).unwrap_or_else(|_| JsonValue::String(value.to_owned()));
        let mut item = json_to_toml_item(value)?;

        // Replace an existing item in place: `insert` would reset the key's formatting, which holds
        // the comments above it.
        match parent.get_mut(key_to_insert) {
            Some(existing) if !existing.is_none() => {
                if existing.is_value() {
                    // Keep inline values inline instead of turning them into table sections.
                    item = item.into_value().map_or_else(|item| item, Item::Value);
                }
                let shape_changed =
                    std::mem::discriminant(existing) != std::mem::discriminant(&item);
                // The comments above a table header are part of the table's decor.
                let header_prefix =
                    tables(existing).first().and_then(|table| table.decor().prefix()).cloned();
                match (&*existing, &mut item) {
                    (Item::Value(old), Item::Value(new)) => {
                        *new.decor_mut() = old.decor().clone();
                    }
                    (old, new) => {
                        for (old, new) in tables(old).into_iter().zip(tables_mut(new)) {
                            *new.decor_mut() = old.decor().clone();
                            new.set_position(old.position());
                            if !old.is_implicit() {
                                new.set_implicit(false);
                            }
                        }
                    }
                }
                let new_is_value = item.is_value();
                *existing = item;
                if shape_changed {
                    let mut key = parent.key_mut(key_to_insert).expect("replaced key must exist");
                    let decor = key.leaf_decor_mut();
                    decor.clear();
                    if new_is_value {
                        decor.set_suffix(" ");
                        if let Some(prefix) = header_prefix {
                            decor.set_prefix(prefix);
                        }
                    }
                }
            }
            _ => insert_toml_item(parent, is_inline, key_to_insert, item),
        }
    }

    Ok(())
}

/// Inserts a new `item` at `key` into `parent`.
///
/// Inline tables can only hold values. There, the new value takes over the whitespace or comment
/// that followed the previous last value, so the inline table keeps its layout.
fn insert_toml_item(parent: &mut dyn TableLike, is_inline: bool, key: &str, mut item: Item) {
    if is_inline {
        let trailing =
            parent.iter_mut().filter_map(|(_, item)| item.as_value_mut()).last().and_then(|last| {
                let suffix = last.decor().suffix().cloned();
                last.decor_mut().set_suffix("");
                suffix
            });
        item = item.into_value().map_or_else(|item| item, Item::Value);
        if let Some(value) = item.as_value_mut() {
            value.decor_mut().clear();
            if let Some(trailing) = trailing {
                value.decor_mut().set_suffix(trailing);
            }
        }
    }
    parent.insert(key, item);
}

/// Converts a JSON value to a TOML item, formatted the same way as [`format_json_to_toml`].
fn json_to_toml_item(value: JsonValue) -> Result<Item> {
    const KEY: &str = "value";

    let wrapper = JsonValue::Object([(KEY.to_string(), value)].into_iter().collect());
    let mut item = parse_toml_document(&format_json_to_toml(wrapper)?)?
        .remove(KEY)
        .ok_or_else(|| fmt_err!("failed to serialize TOML value"))?;
    // Drop the layout of the temporary document so new tables are placed after their parent.
    reset_table_layout(&mut item);
    Ok(item)
}

/// Recursively clears the document position and header whitespace of all tables in `item`.
fn reset_table_layout(item: &mut Item) {
    for table in tables_mut(item) {
        table.set_position(None);
        table.decor_mut().clear();
        for (_, item) in table.iter_mut() {
            reset_table_layout(item);
        }
    }
}

/// Returns the tables of `item` that are written with a header.
fn tables(item: &Item) -> Vec<&Table> {
    match item {
        Item::Table(table) => vec![table],
        Item::ArrayOfTables(array) => array.iter().collect(),
        _ => Vec::new(),
    }
}

/// Returns the tables of `item` that are written with a header.
fn tables_mut(item: &mut Item) -> Vec<&mut Table> {
    match item {
        Item::Table(table) => vec![table],
        Item::ArrayOfTables(array) => array.iter_mut().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"# Deployment config.

owner = "0x000000000000000000000000000000000000dEaD" # multisig
deployed_at = 2024-04-27T11:57:21Z
max_supply = 0xffff_ffff
salt = 'literal-string'

# Mainnet settings.
[mainnet]
token = "0x0000000000000000000000000000000000000000" # filled by script
limits = { daily = 1_000, weekly = 5_000 }

# Base settings.
[base]
chain_id = 8453
"#;

    fn upsert(toml: &str, value: &str, key: &str) -> Result<String> {
        let mut document = parse_toml_document(toml)?;
        upsert_toml_value(&mut document, value, key)?;
        Ok(document.to_string())
    }

    #[test]
    fn upsert_toml_keeps_comment_above_key() {
        let toml = "key1 = \"1\"\n\n# this is key2\nkey2 = \"2\"\n";
        assert_eq!(
            upsert(toml, "abcd", ".key2").unwrap(),
            "key1 = \"1\"\n\n# this is key2\nkey2 = \"abcd\"\n"
        );
    }

    #[test]
    fn upsert_toml_replaces_only_the_value() {
        let address = "0x000000000000000000000000000000000000bEEF";
        assert_eq!(
            upsert(CONFIG, address, ".mainnet.token").unwrap(),
            CONFIG.replace(
                r#"token = "0x0000000000000000000000000000000000000000""#,
                &format!(r#"token = "{address}""#)
            )
        );
        assert_eq!(
            upsert(CONFIG, "2000", ".mainnet.limits.daily").unwrap(),
            CONFIG.replace("daily = 1_000", "daily = 2000")
        );
        assert_eq!(
            upsert(CONFIG, "{\"chain_id\": 10}", "base").unwrap(),
            CONFIG.replace("chain_id = 8453", "chain_id = 10")
        );
        assert_eq!(
            upsert(CONFIG, "{\"daily\": 2000}", ".mainnet.limits").unwrap(),
            CONFIG.replace("daily = 1_000, weekly = 5_000", "daily = 2000")
        );
    }

    #[test]
    fn upsert_toml_adds_keys() {
        assert_eq!(
            upsert(CONFIG, "30000000", ".base.gas_limit").unwrap(),
            format!("{CONFIG}gas_limit = 30000000\n")
        );
        assert_eq!(
            upsert(CONFIG, "{\"block\": 123}", ".optimism.contracts").unwrap(),
            format!("{CONFIG}\n[optimism.contracts]\nblock = 123\n")
        );
        assert_eq!(
            upsert(CONFIG, "{\"monthly\": 9}", ".mainnet.limits.extra").unwrap(),
            CONFIG.replace("weekly = 5_000 }", "weekly = 5_000, extra = { monthly = 9 } }")
        );
    }

    #[test]
    fn upsert_toml_preserves_inline_table_formatting_when_adding_keys() {
        let toml = "limits = { daily = 1_000, weekly = 5_000 }\n";
        assert_eq!(
            upsert(toml, "9000", ".limits.monthly").unwrap(),
            "limits = { daily = 1_000, weekly = 5_000, monthly = 9000 }\n"
        );
        assert_eq!(
            upsert(toml, "9000", ".limits.extra.monthly").unwrap(),
            "limits = { daily = 1_000, weekly = 5_000, extra = { monthly = 9000 } }\n"
        );

        let toml = "limits={daily=1_000,weekly = 5_000}\n";
        assert_eq!(
            upsert(toml, "9000", ".limits.monthly").unwrap(),
            "limits={daily=1_000,weekly = 5_000, monthly = 9000}\n"
        );

        let toml = "limits = {\n    # Keep this cap conservative.\n    daily  = 1_000,\n    weekly = 5_000,\n}\n";
        assert_eq!(
            upsert(toml, "9000", ".limits.monthly").unwrap(),
            "limits = {\n    # Keep this cap conservative.\n    daily  = 1_000,\n    weekly = 5_000, monthly = 9000,\n}\n"
        );
    }

    #[test]
    fn upsert_toml_replaces_arrays_of_tables_in_place() {
        let toml = "# Production RPC endpoints.\n[[rpc]]\nurl = \"old\"\n";
        assert_eq!(
            upsert(toml, r#"[{"url":"new"}]"#, ".rpc").unwrap(),
            "# Production RPC endpoints.\n[[rpc]]\nurl = \"new\"\n"
        );
    }

    #[test]
    fn upsert_toml_keeps_comments_when_the_item_shape_changes() {
        let toml = "# Deployment settings.\n[deployment] # maintained by the deploy script\nchain_id = 1\n";
        assert_eq!(
            upsert(toml, "disabled", ".deployment").unwrap(),
            "# Deployment settings.\ndeployment = \"disabled\"\n"
        );

        let toml = "# Production RPC endpoint.\n[rpc]\nurl = \"a\"\n";
        assert_eq!(
            upsert(toml, r#"[{"url":"a"},{"url":"b"}]"#, ".rpc").unwrap(),
            "# Production RPC endpoint.\n[[rpc]]\nurl = \"a\"\n\n[[rpc]]\nurl = \"b\"\n"
        );

        let toml = "# Production RPC endpoints.\nrpc = []\n";
        assert_eq!(
            upsert(toml, r#"[{"url":"a"},{"url":"b"}]"#, ".rpc").unwrap(),
            "# Production RPC endpoints.\nrpc = [{ url = \"a\" }, { url = \"b\" }]\n"
        );
    }

    #[test]
    fn upsert_toml_formats_new_values_like_write_toml() {
        let value = r#"{"list": ["0x01", "0x02"], "empty": {}, "nested": {"a": {"b": 1}}}"#;
        let expected = format_json_to_toml(
            serde_json::json!({ "new": serde_json::from_str::<JsonValue>(value).unwrap() }),
        )
        .unwrap();
        assert_eq!(upsert("", value, ".new").unwrap(), expected);
    }

    #[test]
    fn upsert_toml_errors() {
        assert_eq!(
            upsert(CONFIG, "1", ".owner.x").unwrap_err().to_string(),
            "final destination is not an object, cannot insert key."
        );
        assert_eq!(
            upsert(CONFIG, "1", ".owner.x.y").unwrap_err().to_string(),
            "path segment 'x' does not resolve to an object."
        );
        assert_eq!(
            upsert(CONFIG, "1", "$.").unwrap_err().to_string(),
            "'valueKey' cannot be empty or just '$'"
        );
    }
}
