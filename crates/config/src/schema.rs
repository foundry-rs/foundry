//! JSON schema of `foundry.toml`.
//!
//! The schema describes the file as users write it: `[profile.<name>]` tables with the config
//! keys, plus the standalone sections such as `[fmt]` or `[rpc_endpoints]`. It is generated from
//! the config types and committed to `crates/config/assets/foundry.schema.json`, so editors can
//! offer completion, documentation and validation for `foundry.toml`.
//!
//! Dictionary entries may be partial overrides of inherited entries. Their required fields
//! cannot be checked in isolation; array records, however, are never merged field by field.

use crate::{
    Config, DEPRECATIONS, DenyLevel, FormatterConfig, GasLimit, RegexWrapper, SolcReq,
    SolidityErrorCode,
    bind_json::BindJsonConfig,
    cache::{CachedChains, CachedEndpoints},
    coverage::CoverageConfig,
    doc::DocConfig,
    endpoints::{RpcEndpoint, RpcEndpoints},
    etherscan::{EtherscanApiKey, EtherscanConfigs},
    filter::GlobMatcher,
    fs_permissions::FsAccessPermission,
    fuzz::FuzzConfig,
    invariant::{InvariantConfig, InvariantWorkers},
    lint::{LinterConfig, Severity},
    mutation::MutationConfig,
    soldeer::SoldeerDependencyConfig,
    symbolic::SymbolicConfig,
    trace::TracingConfig,
    vyper::VyperConfig,
};
use foundry_compilers::artifacts::EvmVersion;
use heck::{ToKebabCase, ToLowerCamelCase, ToShoutySnakeCase, ToUpperCamelCase};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Map, Value, json};
use std::{borrow::Cow, collections::BTreeMap};

/// The URL the schema is published at.
pub const SCHEMA_ID: &str = "https://raw.githubusercontent.com/foundry-rs/foundry/master/crates/config/assets/foundry.schema.json";

/// Returns the JSON schema of `foundry.toml`, pretty-printed.
pub fn json_schema() -> String {
    let mut schema = serde_json::to_value(schemars::schema_for!(FoundryToml)).unwrap();
    let defaults = serde_json::to_value(Config::default()).unwrap();
    let defs = schema["$defs"].as_object().cloned().unwrap_or_default();
    schema["$defs"] = Value::Object(finalize_defs(defs, &defaults));
    let root = schema.as_object_mut().unwrap();
    root.insert("$id".into(), SCHEMA_ID.into());
    root.remove("required");
    clean(root);
    // Sort keys, so the output does not depend on serde_json's `preserve_order` feature, which
    // other workspace crates enable.
    let mut out = serde_json::to_string_pretty(&sorted(schema)).unwrap();
    out.push('\n');
    out
}

/// The layout of a `foundry.toml` file.
///
/// Settings go in `[profile.<name>]` tables. Profiles inherit from `[profile.default]`, and
/// `FOUNDRY_PROFILE` selects the active profile. The standalone sections apply to all profiles.
#[derive(JsonSchema)]
#[schemars(title = "foundry.toml", deny_unknown_fields)]
#[expect(dead_code)]
struct FoundryToml {
    /// Configuration profiles. `[profile.default]` holds the defaults that all other profiles
    /// inherit; select another profile with `FOUNDRY_PROFILE=<name>`.
    profile: BTreeMap<String, Config>,
    /// RPC endpoint aliases, usable as `--rpc-url <alias>` and in `vm.createFork("<alias>")`.
    rpc_endpoints: RpcEndpoints,
    /// Block explorer API configurations, keyed by alias.
    etherscan: EtherscanConfigs,
    /// Configuration for `forge fmt`.
    fmt: FormatterConfig,
    /// Configuration for `forge lint`.
    lint: LinterConfig,
    /// Configuration for `forge doc`.
    doc: DocConfig,
    /// Configuration for fuzz tests.
    fuzz: FuzzConfig,
    /// Configuration for invariant tests. Keys not set here fall back to `[fuzz]`.
    invariant: InvariantConfig,
    /// Configuration for symbolic tests.
    symbolic: SymbolicConfig,
    /// Configuration for `forge coverage`.
    coverage: CoverageConfig,
    /// Configuration for mutation testing.
    mutation: MutationConfig,
    /// Configuration for trace rendering.
    tracing: TracingConfig,
    #[schemars(schema_with = "deprecated_labels")]
    labels: (),
    /// Soldeer dependencies.
    dependencies: SoldeerDependencyConfig,
    /// Soldeer configuration.
    #[schemars(schema_with = "open_table")]
    soldeer: (),
    /// Configuration for the Vyper compiler.
    vyper: VyperConfig,
    /// Configuration for `forge bind-json`.
    bind_json: BindJsonConfig,
    /// Configuration read by other tools, keyed by tool name. Forge does not read it.
    #[schemars(schema_with = "external")]
    external: (),
}

/// The keys of `NetworkConfigs`, which is flattened into a profile.
///
/// Written by hand so that the schema lists every network that release builds support, whatever
/// features this crate is compiled with.
#[derive(JsonSchema)]
#[expect(dead_code)]
pub(crate) struct NetworkKeys {
    /// Enable the features of a network family.
    #[schemars(schema_with = "network")]
    network: (),
    /// Enable Celo network features.
    celo: bool,
    /// Whether to bypass prevrandao.
    bypass_prevrandao: bool,
    /// Deprecated: use `network = "optimism"`.
    #[schemars(schema_with = "deprecated_bool")]
    optimism: (),
    /// Deprecated: use `network = "tempo"`.
    #[schemars(schema_with = "deprecated_bool")]
    tempo: (),
    /// Deprecated: use `network = "monad"`.
    #[schemars(schema_with = "deprecated_bool")]
    monad: (),
}

/// Network families accepted by `network`, in release builds.
const NETWORKS: &[&str] = &["ethereum", "base", "optimism", "tempo", "monad"];

/// Every [`EvmVersion`], in order.
///
/// The `match` in `evm_versions_complete` fails to compile when a version is added, so the list
/// stays complete.
const EVM_VERSIONS: &[EvmVersion] = &[
    EvmVersion::Homestead,
    EvmVersion::TangerineWhistle,
    EvmVersion::SpuriousDragon,
    EvmVersion::Byzantium,
    EvmVersion::Constantinople,
    EvmVersion::Petersburg,
    EvmVersion::Istanbul,
    EvmVersion::Berlin,
    EvmVersion::London,
    EvmVersion::Paris,
    EvmVersion::Shanghai,
    EvmVersion::Cancun,
    EvmVersion::Prague,
    EvmVersion::Osaka,
    EvmVersion::Amsterdam,
];

/// Keys accepted under a second name, as `(type, key, alias)`.
const KEY_ALIASES: &[(&str, &str, &str)] = &[
    ("Config", "chain_id", "chain"),
    // Read by `BackwardsCompatTomlProvider`.
    ("Config", "solc", "solc_version"),
    ("VyperConfig", "opt_level", "optLevel"),
];

/// Enum values accepted under a second name, as `(type, value, alias)`.
const VALUE_ALIASES: &[(&str, &str, &str)] = &[
    ("MultilineFuncHeaderStyle", "params_always", "params_first"),
    ("InvariantDepthMode", "random", "uniform"),
];

/// Post-processes the generated definitions:
///
/// - sets the default of each key from `defaults`;
/// - adds the aliases serde accepts but schemars does not list;
/// - closes the tables to unknown keys, as forge warns about them;
/// - makes default-backed sections and inherited dictionary entries partial;
/// - removes `null`, which TOML cannot represent.
fn finalize_defs(mut defs: Map<String, Value>, defaults: &Value) -> Map<String, Value> {
    // Config sections and the default value of each, taken from `Config::default()`.
    let mut sections = vec![("Config".to_string(), defaults.clone())];
    let mut i = 0;
    while i < sections.len() {
        let (name, value) = sections[i].clone();
        i += 1;
        let Some(Value::Object(props)) = defs.get_mut(&name).and_then(|d| d.get_mut("properties"))
        else {
            continue;
        };
        for (key, prop) in props.iter_mut() {
            let Some(default) = value.get(key) else { continue };
            if let Some(Value::String(r)) = prop.get("$ref")
                && let Some(def) = r.strip_prefix("#/$defs/")
            {
                if default.is_object() && !sections.iter().any(|(n, _)| n == def) {
                    sections.push((def.to_string(), default.clone()));
                }
                continue;
            }
            if !default.is_null()
                && let Value::Object(prop) = prop
            {
                prop.entry("default").or_insert_with(|| default.clone());
            }
        }
    }
    for name in
        sections.iter().map(|(name, _)| name.as_str()).chain(["MapDependency", "EtherscanConfig"])
    {
        if let Some(Value::Object(def)) = defs.get_mut(name) {
            def.remove("required");
        }
    }

    for &(def, key, alias) in KEY_ALIASES {
        let props = defs[def]["properties"].as_object_mut().unwrap();
        let mut prop = props[key].clone();
        prop["description"] = format!("Alias of `{key}`.").into();
        props.insert(alias.into(), prop);
    }
    for &(def, value, alias) in VALUE_ALIASES {
        let variants = defs[def]["oneOf"].as_array_mut().unwrap();
        assert!(variants.iter().any(|v| v["const"] == value), "`{def}` has no `{value}`");
        variants.push(json!({
            "type": "string",
            "const": alias,
            "description": format!("Alias of `{value}`."),
        }));
    }
    // Deprecated keys that forge still reads, with a warning.
    for &(key, new) in DEPRECATIONS {
        let props = defs["Config"]["properties"].as_object_mut().unwrap();
        props.entry(key).or_insert_with(|| json!({ "type": "boolean" }));
        props[key]["description"] = format!("Deprecated: use `{new}`.").into();
        props[key]["deprecated"] = true.into();
    }

    for def in defs.values_mut() {
        let Value::Object(def) = def else { continue };
        if def.contains_key("properties") {
            def.entry("additionalProperties").or_insert(false.into());
        }
        clean(def);
    }
    // Keep the usual alternate spellings accepted by ForcedSnakeCaseData tied to their
    // canonical schemas, without suggesting duplicate keys in editor completion.
    let patterns = defs["Config"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(|key| {
            let mut aliases = vec![
                key.to_lower_camel_case(),
                key.to_upper_camel_case(),
                key.to_shouty_snake_case(),
                key.to_kebab_case(),
            ];
            aliases.retain(|alias| alias != key);
            aliases.sort();
            aliases.dedup();
            let pattern =
                aliases.iter().map(|alias| regex::escape(alias)).collect::<Vec<_>>().join("|");
            (
                format!("^(?:{pattern})$"),
                json!({ "$ref": format!("#/$defs/Config/properties/{key}") }),
            )
        })
        .collect::<Map<_, _>>();
    defs["Config"]["patternProperties"] = patterns.into();
    defs
}

/// Returns `value` with the keys of every object in sorted order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        value => value,
    }
}

/// Removes what does not apply to a TOML file.
fn clean(schema: &mut Map<String, Value>) {
    if schema.get("default").is_some_and(Value::is_null) {
        schema.remove("default");
    }
    schema.remove("writeOnly");
    if let Some(Value::Array(types)) = schema.get_mut("type") {
        types.retain(|t| t != "null");
        if types.len() == 1 {
            let ty = types.pop().unwrap();
            schema.insert("type".into(), ty);
        }
    }
    // `Option<T>` is `anyOf: [T, null]`.
    if let Some(Value::Array(variants)) = schema.get_mut("anyOf") {
        variants.retain(|v| v.get("type").is_none_or(|t| t != "null"));
        if variants.len() == 1
            && let Value::Object(inner) = variants.pop().unwrap()
        {
            schema.remove("anyOf");
            for (k, v) in inner {
                schema.entry(k).or_insert(v);
            }
        }
    }
    for value in schema.values_mut() {
        match value {
            Value::Object(map) => clean(map),
            Value::Array(items) => {
                for item in items {
                    if let Value::Object(map) = item {
                        clean(map);
                    }
                }
            }
            _ => {}
        }
    }
}

fn string_enum(description: &str, values: Vec<String>) -> Schema {
    json_schema!({ "description": description, "type": "string", "enum": values })
}

/// A string equal to one of `values` ignoring case. The values are listed for completion; the
/// pattern accepts the other casings.
fn case_insensitive(description: &str, values: &[&str]) -> Schema {
    let pattern = values
        .iter()
        .map(|value| {
            value
                .chars()
                .map(|c| {
                    if c.is_ascii_alphabetic() {
                        format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
                    } else {
                        c.to_string()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("|");
    json_schema!({
        "description": description,
        "type": "string",
        "anyOf": [{ "enum": values }, { "pattern": format!("^(?:{pattern})$") }],
    })
}

/// The EVM version of a profile, which forge lowercases before parsing.
pub(crate) fn evm_version(_: &mut SchemaGenerator) -> Schema {
    let values: Vec<String> = EVM_VERSIONS.iter().map(|v| v.as_str().to_lowercase()).collect();
    let values: Vec<&str> = values.iter().map(String::as_str).collect();
    case_insensitive("The EVM version to compile for.", &values)
}

/// The EVM version of a compilation restriction or profile, parsed as written.
pub(crate) fn evm_version_exact(_: &mut SchemaGenerator) -> Schema {
    let mut values = Vec::new();
    for version in EVM_VERSIONS {
        let name = version.as_str();
        values.push(name.to_lowercase());
        if name != name.to_lowercase() {
            values.push(name.to_string());
        }
    }
    string_enum("The EVM version to compile for.", values)
}

/// The bytecode hash of a profile, which forge lowercases before parsing.
pub(crate) fn bytecode_hash(_: &mut SchemaGenerator) -> Schema {
    case_insensitive(
        "The hash method of the metadata hash appended to the bytecode.",
        &["ipfs", "bzzr1", "none"],
    )
}

/// A count, or `max`.
pub(crate) fn usize_or_max(_: &mut SchemaGenerator) -> Schema {
    unsigned_or_max()
}

/// A percentage from 0 to 100.
pub(crate) fn percent(_: &mut SchemaGenerator) -> Schema {
    // Numeric accepts decimal, hex, octal and binary strings, including leading zeros and
    // underscores. Enumerate the bounded values in each radix rather than approximate the bound.
    let patterns = [(10, ""), (16, "0[xX]"), (8, "0[oO]"), (2, "0[bB]")].map(|(radix, prefix)| {
        let values = (1u32..=100)
            .map(|value| {
                let digits = match radix {
                    16 => format!("{value:x}"),
                    8 => format!("{value:o}"),
                    2 => format!("{value:b}"),
                    _ => value.to_string(),
                };
                digits
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphabetic() {
                            format!("[{c}{}]_*", c.to_ascii_uppercase())
                        } else {
                            format!("{c}_*")
                        }
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("|");
        format!("{prefix}[0_]*(?:{values})?")
    });
    json_schema!({
        "anyOf": [
            { "type": "integer", "minimum": 0, "maximum": 100 },
            { "type": "string", "pattern": format!("^(?:{})$", patterns.join("|")) },
        ],
    })
}

/// `BytecodeHash` read with its derived `Deserialize`, which takes the variant names.
pub(crate) fn bytecode_hash_variant(_: &mut SchemaGenerator) -> Schema {
    string_enum(
        "The hash method of the metadata hash appended to the bytecode.",
        ["Ipfs", "Bzzr1", "None"].map(String::from).to_vec(),
    )
}

pub(crate) fn revert_strings(_: &mut SchemaGenerator) -> Schema {
    string_enum(
        "How to treat revert and require reason strings.",
        ["default", "strip", "debug", "verboseDebug", "verbosedebug"].map(String::from).to_vec(),
    )
}

pub(crate) fn vyper_optimize(_: &mut SchemaGenerator) -> Schema {
    let values = [
        "gas", "codesize", "none", "O1", "O2", "O3", "Os", "o1", "o2", "o3", "os", "1", "2", "3",
        "s",
    ];
    string_enum("Vyper optimization mode or level.", values.map(String::from).to_vec())
}

fn network(_: &mut SchemaGenerator) -> Schema {
    string_enum("A network family.", NETWORKS.iter().map(|s| s.to_string()).collect())
}

fn deprecated_bool(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "boolean", "deprecated": true })
}

pub(crate) fn hardfork(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "description": "An Ethereum hardfork such as `prague`, or `<network>:<hardfork>` for \
                        another network family, such as `optimism:isthmus`.",
    })
}

pub(crate) fn uint256(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "description": "An unsigned 256-bit integer, written as a decimal or `0x` hex string.",
    })
}

pub(crate) fn address(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "pattern": "^(0x)?[0-9a-fA-F]{40}$" })
}

pub(crate) fn bytes32(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "pattern": "^(0x)?[0-9a-fA-F]{64}$" })
}

pub(crate) fn labels(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "description": "Labels shown in traces, keyed by address.",
        "propertyNames": { "pattern": "^(0x)?[0-9a-fA-F]{40}$" },
        "additionalProperties": { "type": "string" },
    })
}

pub(crate) fn deprecated_labels(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = labels(generator);
    schema.insert("description".into(), "Deprecated: use `[tracing.labels]`.".into());
    schema.insert("deprecated".into(), true.into());
    schema
}

pub(crate) fn chain(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": ["integer", "string"],
        "description": "An EIP-155 chain ID, or a chain name such as `mainnet` or `sepolia`.",
    })
}

pub(crate) fn version_req(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "description": "A semver version requirement, such as `>=0.8.20` or `=0.8.28`.",
    })
}

pub(crate) fn output_selection(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "items": {
            "type": "string",
            "examples": [
                "abi", "devdoc", "userdoc", "metadata", "ir", "irOptimized", "storageLayout",
                "evm.assembly", "evm.bytecode", "evm.deployedBytecode", "evm.gasEstimates",
                "evm.methodIdentifiers",
            ],
        },
    })
}

/// A table whose keys are not described.
pub(crate) fn open_table(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object" })
}

fn external(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object", "additionalProperties": { "type": "object" } })
}

/// Implements [`JsonSchema`] for types with a custom `Deserialize` implementation.
macro_rules! impl_schema {
    ($($ty:ident => $schema:expr;)*) => {$(
        impl JsonSchema for $ty {
            fn schema_name() -> Cow<'static, str> {
                stringify!($ty).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                $schema
            }
        }
    )*};
}

impl_schema! {
    RegexWrapper => json_schema!({
        "type": "string",
        "description": "A regular expression, in Rust `regex` syntax.",
    });
    GlobMatcher => json_schema!({ "type": "string", "description": "A glob pattern." });
    SolcReq => json_schema!({
        "type": "string",
        "description": "A solc version such as `0.8.28`, or the path to a solc binary.",
    });
    GasLimit => unsigned_or_max();
    DenyLevel => {
        let mut level = case_insensitive(
            "Case is ignored; `warning`, `w`, `note`, `n`, `false` and `f` are also accepted.",
            &["never", "warnings", "notes", "warning", "w", "note", "n", "false", "f"],
        );
        level.remove("description");
        json_schema!({
            "anyOf": [
                level,
                { "type": "boolean", "description": "Deprecated: `true` means `warnings`." },
            ],
        })
    };
    Severity => case_insensitive(
        "A lint severity. Case is ignored.",
        &["high", "med", "medium", "low", "info", "gas", "code-size", "codesize", "size"],
    );
    InvariantWorkers => json_schema!({
        "anyOf": [
            { "type": "integer", "minimum": 1, "maximum": u64::MAX },
            {
                "type": "string",
                "pattern": format!("^\\s*(?:[aA][uU][tT][oO]|{})\\s*$", unsigned_decimal_pattern()),
                "not": { "pattern": "^\\s*\\+?0+\\s*$" },
            },
        ],
    });
    CachedChains => json_schema!({
        "anyOf": [
            { "type": "string", "enum": ["all", "none"] },
            { "type": "array", "items": { "type": ["integer", "string"] } },
        ],
    });
    CachedEndpoints => json_schema!({
        "type": "string",
        "description": "`all`, `remote`, or a regex that endpoint URLs must match.",
    });
    FsAccessPermission => json_schema!({
        "anyOf": [
            {
                "type": "string",
                "enum": ["read", "write", "read-write", "readwrite", "none", "true", "false"],
            },
            { "type": "boolean", "description": "`true` means `read-write`." },
        ],
    });
    EtherscanApiKey => json_schema!({
        "type": "string",
        "description": "An API key, or `${ENV_VAR}` to read it from the environment.",
    });
    RpcEndpoint => json_schema!({
        "anyOf": [
            {
                "type": "string",
                "description": "An RPC URL. `${ENV_VAR}` placeholders are read from the \
                                environment.",
            },
            {
                "type": "object",
                "properties": {
                    "endpoint": { "type": "string", "description": "The RPC URL." },
                    "url": { "type": "string", "description": "Alias of `endpoint`." },
                    "endpoints": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "RPC URLs to balance requests across, with failover.",
                    },
                    "retries": { "type": "integer", "minimum": 0 },
                    "retry_backoff": { "type": "integer", "minimum": 0 },
                    "compute_units_per_second": { "type": "integer", "minimum": 0 },
                    "auth": {
                        "type": "string",
                        "description": "Value of the `Authorization` header.",
                    },
                },
                "additionalProperties": false,
                // Missing URLs can be inherited, but specifying two forms is always invalid.
                "not": {
                    "anyOf": [
                        { "required": ["endpoint", "url"] },
                        { "required": ["endpoint", "endpoints"] },
                        { "required": ["url", "endpoints"] },
                    ],
                },
            },
        ],
    });
}

impl JsonSchema for SolidityErrorCode {
    fn schema_name() -> Cow<'static, str> {
        "SolidityErrorCode".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<&str> =
            (0..10_000).filter_map(|code| Self::from(code).as_str().ok()).collect();
        json_schema!({
            "anyOf": [
                { "type": "integer", "minimum": 0, "description": "A solc error code." },
                { "type": "string", "enum": names },
            ],
        })
    }
}

/// A `u64`, or a decimal string or case-insensitive `max`.
fn unsigned_or_max() -> Schema {
    json_schema!({
        "anyOf": [
            { "type": "integer", "minimum": 0, "maximum": u64::MAX },
            {
                "type": "string",
                "pattern": format!("^(?:[mM][aA][xX]|{})$", unsigned_decimal_pattern()),
                "description": "`max`, or an unsigned decimal number that fits in 64 bits.",
            },
        ],
    })
}

/// Decimal strings through `u64::MAX`, allowing a leading plus and leading zeros.
fn unsigned_decimal_pattern() -> String {
    let max = u64::MAX.to_string();
    let mut alternatives = vec![format!("[0-9]{{1,{}}}", max.len() - 1)];
    for (i, digit) in max.bytes().enumerate() {
        if digit > b'0' {
            alternatives.push(format!(
                "{}[0-{}][0-9]{{{}}}",
                &max[..i],
                char::from(digit - 1),
                max.len() - i - 1,
            ));
        }
    }
    alternatives.push(max);
    format!("\\+?0*(?:{})", alternatives.join("|"))
}

/// Block numbers and timestamps additionally accept unquoted `u64` values.
pub(crate) fn u64_or_uint256(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": ["integer", "string"],
        "minimum": 0,
        "maximum": u64::MAX,
        "description": "An unsigned 64-bit integer, or a decimal or `0x` hex string for a \
                        256-bit value.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;
    use std::path::Path;

    const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/foundry.schema.json");

    #[test]
    fn schema_up_to_date() {
        let schema = json_schema();
        let path = Path::new(SCHEMA_PATH);
        if std::env::var_os("SNAPSHOTS").is_some_and(|v| v == "overwrite") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &schema).unwrap();
        }
        // A Windows checkout may have CRLF line endings.
        let current = std::fs::read_to_string(path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            current == schema,
            "{SCHEMA_PATH} is out of date; run `SNAPSHOTS=overwrite cargo test -p foundry-config \
             --features schema schema` to update it",
        );
    }

    #[test]
    fn evm_versions_complete() {
        // Fails to compile when foundry-compilers adds a version.
        let _ = |v: EvmVersion| match v {
            EvmVersion::Homestead
            | EvmVersion::TangerineWhistle
            | EvmVersion::SpuriousDragon
            | EvmVersion::Byzantium
            | EvmVersion::Constantinople
            | EvmVersion::Petersburg
            | EvmVersion::Istanbul
            | EvmVersion::Berlin
            | EvmVersion::London
            | EvmVersion::Paris
            | EvmVersion::Shanghai
            | EvmVersion::Cancun
            | EvmVersion::Prague
            | EvmVersion::Osaka
            | EvmVersion::Amsterdam => {}
        };
        assert_eq!(EVM_VERSIONS.len(), 15);
        for version in EVM_VERSIONS {
            assert_eq!(version.as_str().to_lowercase().parse::<EvmVersion>().unwrap(), *version);
        }
    }

    #[test]
    fn networks_complete() {
        for variant in foundry_evm_networks::NetworkVariant::value_variants() {
            let name = variant.to_possible_value().unwrap().get_name().to_string();
            assert!(NETWORKS.contains(&name.as_str()), "`{name}` missing from NETWORKS");
        }
    }

    #[test]
    fn standalone_sections_complete() {
        let schema: Value = serde_json::from_str(&json_schema()).unwrap();
        let props = schema["properties"].as_object().unwrap();
        for section in Config::STANDALONE_SECTIONS
            .iter()
            .chain(&[Config::PROFILE_SECTION, Config::EXTERNAL_SECTION])
        {
            assert!(props.contains_key(*section), "section `{section}` missing from the schema");
        }
        assert_eq!(props.len(), Config::STANDALONE_SECTIONS.len() + 2);
    }

    #[test]
    fn profile_key_validation() {
        let validator = validator();
        for (setting, valid) in [
            ("via_ir = true", true),
            ("viaIr = true", true),
            ("via-ir = true", true),
            ("ViaIr = true", true),
            ("VIA_IR = true", true),
            ("optimizerRuns = 200", true),
            ("evmVersion = 'SHANGHAI'", true),
            ("optimizerRuns = '200'", false),
            ("optimizer-runs = '200'", false),
            ("evmVersion = 'shangai'", false),
            ("fuzz_runs = 10000", false),
            ("fuzzRuns = 10000", false),
            ("fuzz-runs = 10000", false),
        ] {
            assert_config_validation(&validator, &format!("[profile.default]\n{setting}"), valid);
        }
    }

    #[test]
    fn record_validation() {
        let validator = validator();
        for (setting, valid) in [
            ("additional_compiler_profiles = [{ name = 'ir', via_ir = true }]", true),
            ("additional_compiler_profiles = [{ via_ir = true }]", false),
            ("compilation_restrictions = [{ paths = 'src/**', optimizer_runs = 200 }]", true),
            ("compilation_restrictions = [{ optimizer_runs = 200 }]", false),
            ("fs_permissions = [{ access = 'read', path = 'out' }]", true),
            ("fs_permissions = [{ access = 'read' }]", false),
        ] {
            assert_config_validation(&validator, &format!("[profile.default]\n{setting}"), valid);
        }
    }

    #[test]
    fn rpc_endpoint_validation() {
        let validator = validator();
        for (endpoint, valid) in [
            ("'https://example.com'", true),
            ("{ endpoint = 'https://example.com', retries = 3 }", true),
            ("{ url = 'https://example.com' }", true),
            ("{ endpoints = ['https://example.com', 'https://fallback.example.com'] }", true),
            ("{ endpoint = 'https://example.com', endpoints = ['https://example.com'] }", false),
            ("{ url = 'https://example.com', endpoints = ['https://example.com'] }", false),
            ("{ endpoint = 'https://example.com', url = 'https://example.com' }", false),
            ("{ endpoints = [] }", false),
        ] {
            assert_config_validation(
                &validator,
                &format!("[rpc_endpoints]\nmainnet = {endpoint}"),
                valid,
            );
        }
    }

    #[test]
    fn numeric_string_validation() {
        let validator = validator();
        for (setting, valid) in [
            ("gas_limit = 'max'", true),
            ("gas_limit = 'MaX'", true),
            ("gas_limit = '+00021'", true),
            ("gas_limit = '18446744073709551615'", true),
            ("gas_limit = '18446744073709551616'", false),
            ("gas_limit = 'unlimited'", false),
            ("gas_limit = '-1'", false),
            ("fuzz.max_fuzz_dictionary_values = 'MAX'", true),
            ("fuzz.max_fuzz_dictionary_values = '-1'", false),
            ("fuzz.dictionary_weight = 100", true),
            ("fuzz.dictionary_weight = 101", false),
            ("fuzz.dictionary_weight = '00100'", true),
            ("fuzz.dictionary_weight = '101'", false),
            ("fuzz.dictionary_weight = '0x64'", true),
            ("fuzz.dictionary_weight = '0x65'", false),
            ("fuzz.dictionary_weight = '0o144'", true),
            ("fuzz.dictionary_weight = '0o145'", false),
            ("fuzz.dictionary_weight = '0b110_0100'", true),
            ("fuzz.dictionary_weight = '0b110_0101'", false),
            ("fuzz.dictionary_weight = 'invalid'", false),
            ("invariant.workers = '01'", true),
            ("invariant.workers = '+1'", true),
            ("invariant.workers = '  AuTo  '", true),
            ("invariant.workers = '0'", false),
            ("invariant.workers = '18446744073709551616'", false),
        ] {
            assert_config_validation(&validator, &format!("[profile.default]\n{setting}"), valid);
        }
    }

    #[test]
    fn inherited_dictionary_validation() {
        let validator = validator();
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_PROFILE", "ci");
            let input = r#"
                [profile.default]
                additional_compiler_profiles = [{ name = "ir", via_ir = true }]
                compilation_restrictions = [{ paths = "src/**", optimizer_runs = 200 }]
                [profile.default.dependencies]
                example = { version = "1.0.0", git = "https://example.com/dependency.git" }
                [profile.default.rpc_endpoints]
                mainnet = { endpoint = "https://example.com", retries = 1 }

                [profile.ci]
                fuzz = { runs = 32 }
                [profile.ci.dependencies]
                example = { rev = "some-revision" }
                [profile.ci.rpc_endpoints]
                mainnet = { retries = 3 }

                [rpc_endpoints]
                mainnet = { retry_backoff = 5 }
            "#;
            jail.create_file("foundry.toml", input)?;
            assert!(validator.is_valid(&toml::from_str::<Value>(input).unwrap()));
            let config = Config::load().unwrap();
            assert_eq!(config.fuzz.runs, 32);
            assert_eq!(config.additional_compiler_profiles[0].name, "ir");
            assert_eq!(config.compilation_restrictions.len(), 1);
            assert_eq!(
                config.rpc_endpoints["mainnet"].endpoint.as_url(),
                Some("https://example.com")
            );
            assert_eq!(config.rpc_endpoints["mainnet"].config.retries, Some(3));
            assert_eq!(config.rpc_endpoints["mainnet"].config.retry_backoff, Some(5));
            let dependencies = config.dependencies.unwrap();
            let crate::soldeer::SoldeerDependencyValue::Map(dependency) =
                &dependencies.0["example"]
            else {
                panic!("expected an inherited dependency table");
            };
            assert_eq!(dependency.version, "1.0.0");
            assert_eq!(dependency.rev.as_deref(), Some("some-revision"));

            // Arrays replace inherited records; their missing fields cannot be filled in.
            for setting in [
                "additional_compiler_profiles = [{ via_ir = true }]",
                "compilation_restrictions = [{ optimizer_runs = 500 }]",
            ] {
                let input = input.replace("fuzz = { runs = 32 }", setting);
                jail.create_file("foundry.toml", &input)?;
                assert!(!validator.is_valid(&toml::from_str::<Value>(&input).unwrap()));
                assert!(Config::load().is_err());
            }
            Ok(())
        });
    }

    fn assert_config_validation(validator: &jsonschema::Validator, input: &str, valid: bool) {
        figment::Jail::expect_with(|jail| {
            jail.create_file("foundry.toml", input)?;
            let result = Config::load();
            assert_eq!(
                result.as_ref().is_ok_and(|config| config.warnings.is_empty()),
                valid,
                "config validation for {input}: {result:?}",
            );
            Ok(())
        });
        let document = toml::from_str::<Value>(input).unwrap();
        let result = validator.validate(&document);
        assert_eq!(result.is_ok(), valid, "schema validation for {input}: {result:?}");
    }

    fn validator() -> jsonschema::Validator {
        let schema = serde_json::from_str(&std::fs::read_to_string(SCHEMA_PATH).unwrap()).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    #[test]
    fn uint256_validation() {
        let validator = validator();
        for (setting, valid) in [
            ("initial_balance = '1000000'", true),
            ("fuzz.seed = '42'", true),
            ("fuzz.seed = '0x2a'", true),
            ("block_number = 100", true),
            ("block_number = '0x10000000000000000'", true),
            ("block_timestamp = 200", true),
            ("block_timestamp = '200'", true),
            ("initial_balance = 1000000", false),
            ("fuzz.seed = 42", false),
        ] {
            assert_config_validation(&validator, &format!("[profile.default]\n{setting}"), valid);
        }
    }

    #[test]
    fn fuzz_runs_validation() {
        let validator = validator();
        for (setting, valid) in
            [("fuzz.runs = 1", true), ("invariant.runs = 0", true), ("fuzz.runs = 0", false)]
        {
            assert_config_validation(&validator, &format!("[profile.default]\n{setting}"), valid);
        }
    }
}
