//! JSON schema of `foundry.toml`.
//!
//! The schema describes the file as users write it: `[profile.<name>]` tables with the config
//! keys, plus the standalone sections such as `[fmt]` or `[rpc_endpoints]`. It is generated from
//! the config types and committed to `crates/config/assets/foundry.schema.json`, so editors can
//! offer completion, documentation and validation for `foundry.toml`.

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
/// - removes `required` and `null`: every key in `foundry.toml` is optional, and TOML has no null.
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
    // Forge converts the keys of a profile to snake_case, so `viaIr` and `via-ir` are accepted.
    defs["Config"]["patternProperties"] = json!({ "[A-Z-]": {} });
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
    schema.remove("required");
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
    json_schema!({
        "anyOf": [
            { "type": "integer", "minimum": 0 },
            { "type": "string", "description": "`max`, or a decimal number." },
        ],
    })
}

/// A percentage from 0 to 100.
pub(crate) fn percent(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            { "type": "integer", "minimum": 0, "maximum": 100 },
            { "type": "string", "description": "A number from 0 to 100." },
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
        "type": ["integer", "string"],
        "description": "An unsigned 256-bit integer. Use a decimal or `0x` hex string for values \
                        that do not fit in a TOML integer.",
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
    GasLimit => json_schema!({
        "anyOf": [
            { "type": "integer", "minimum": 0 },
            { "type": "string", "description": "`max`, or a decimal number." },
        ],
    });
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
            { "type": "integer", "minimum": 1 },
            { "type": "string", "pattern": "^\\s*(?:[aA][uU][tT][oO]|[1-9][0-9]*)\\s*$" },
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

#[cfg(test)]
mod tests {
    use super::*;
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
        use clap::ValueEnum;
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
}
