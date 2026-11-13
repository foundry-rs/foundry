//! Generates the machine-readable `foundry.toml` reference.
//!
//! The Foundry Book imports [`REFERENCE_PATH`] to generate its configuration reference, so
//! user-facing documentation for a setting lives in the doc comment of its field. Run
//! `cargo config-reference` to update the file after adding or changing a setting.

use crate::{Config, DEPRECATIONS};
use heck::{ToKebabCase, ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Attribute, Expr, ExprLit, Fields, GenericArgument, Item, Lit, Meta, PathArguments, Type,
    punctuated::Punctuated, token::Comma,
};

/// Path of the generated reference, relative to the crate root.
const REFERENCE_PATH: &str = "assets/config-reference.json";

/// Fields that are serialized but are not `foundry.toml` settings.
const EXCLUDED_KEYS: &[&str] = &["root"];

/// Environment variables that set a key outside the generic `FOUNDRY_` and `DAPP_` mappings.
const SPECIAL_ENV: &[(&str, &[&str])] = &[
    ("extends", &[]),
    ("remappings", &["FOUNDRY_REMAPPINGS", "DAPP_REMAPPINGS"]),
    ("libraries", &["FOUNDRY_LIBRARIES", "DAPP_LIBRARIES"]),
];

/// User-facing types of types defined outside this crate.
const EXTERNAL_TYPES: &[(&str, &str)] = &[
    ("Address", "string (address)"),
    ("B256", "string (32-byte hex)"),
    ("BytecodeHash", "string"),
    ("Chain", "string or integer (chain name or ID)"),
    ("ContractOutputSelection", "string"),
    ("EvmVersion", "string"),
    ("Extends", "string or table"),
    ("FoundryHardfork", "string"),
    ("Glob", "string (glob)"),
    ("GlobMatcher", "string (glob)"),
    ("ModelCheckerSettings", "table"),
    ("OptimizerDetails", "table"),
    ("PathBuf", "string (path)"),
    ("RegexWrapper", "string (regular expression)"),
    ("Regex", "string (regular expression)"),
    ("RevertStrings", "string"),
    ("SolcReq", "string (version or path)"),
    ("SoldeerConfig", "table"),
    ("SoldeerDependencyConfig", "table"),
    ("SolidityErrorCode", "string or integer"),
    ("String", "string"),
    ("U256", "integer"),
    ("Url", "string (URL)"),
    ("Version", "string (version)"),
    ("VyperOptimizationLevel", "string"),
    ("VyperOptimizationMode", "string"),
    ("VyperVenomSettings", "table"),
];

/// The generated reference.
#[derive(Serialize)]
struct Reference {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    settings: Vec<Setting>,
}

/// One `foundry.toml` key.
#[derive(Serialize)]
struct Setting {
    /// The dotted key below `[profile.<name>]`, such as `fuzz.runs`.
    key: String,
    /// The standalone section the key belongs to, such as `fuzz`, if any.
    section: Option<String>,
    /// The user-facing type.
    #[serde(rename = "type")]
    ty: String,
    /// Whether the key holds a table of other documented keys.
    table: bool,
    /// The default value as TOML, if the key has one.
    default: Option<String>,
    /// Accepted values for an enumeration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    values: Vec<String>,
    /// Alternative names accepted for the key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    /// Environment variables that set the key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    env: Vec<String>,
    /// The replacement for a deprecated key.
    #[serde(skip_serializing_if = "Option::is_none")]
    deprecated: Option<String>,
    /// The Markdown description from the field's doc comment.
    description: String,
}

/// Struct and enum definitions of this crate by name.
#[derive(Default)]
struct Definitions {
    structs: HashMap<String, syn::ItemStruct>,
    enums: HashMap<String, syn::ItemEnum>,
}

impl Definitions {
    fn load(dir: &Path) -> Self {
        let mut this = Self::default();
        for entry in walkdir::WalkDir::new(dir) {
            let entry = entry.unwrap();
            if entry.path().extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let file = syn::parse_file(&fs::read_to_string(entry.path()).unwrap())
                .unwrap_or_else(|e| panic!("failed to parse {}: {e}", entry.path().display()));
            this.collect(file.items);
        }
        this
    }

    fn collect(&mut self, items: Vec<Item>) {
        for item in items {
            match item {
                Item::Struct(item) if !is_test_only(&item.attrs) => {
                    self.structs.entry(item.ident.to_string()).or_insert(item);
                }
                Item::Enum(item) if !is_test_only(&item.attrs) => {
                    self.enums.entry(item.ident.to_string()).or_insert(item);
                }
                Item::Mod(item) if !is_test_only(&item.attrs) => {
                    if let Some((_, items)) = item.content {
                        self.collect(items);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Serde attributes that change a field's key.
#[derive(Default)]
struct SerdeAttrs {
    skip: bool,
    skip_serializing: bool,
    transparent: bool,
    flatten: bool,
    rename: Option<String>,
    rename_all: Option<String>,
    aliases: Vec<String>,
}

impl SerdeAttrs {
    fn parse(attrs: &[Attribute]) -> Self {
        let mut this = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
            let Ok(metas) = attr.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)
            else {
                continue;
            };
            for meta in metas {
                let name = meta.path().get_ident().map(ToString::to_string).unwrap_or_default();
                match name.as_str() {
                    "skip" | "skip_deserializing" => this.skip = true,
                    "skip_serializing" => this.skip_serializing = true,
                    "transparent" => this.transparent = true,
                    "flatten" => this.flatten = true,
                    "rename" => this.rename = string_value(&meta),
                    "rename_all" => this.rename_all = string_value(&meta),
                    "alias" => this.aliases.extend(string_value(&meta)),
                    _ => {}
                }
            }
        }
        this
    }
}

fn string_value(meta: &Meta) -> Option<String> {
    match meta {
        Meta::NameValue(nv) => match &nv.value {
            Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => Some(s.value()),
            _ => None,
        },
        // `rename(deserialize = "..")`.
        Meta::List(list) => {
            let metas = list.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated).ok()?;
            metas.iter().find(|meta| meta.path().is_ident("deserialize")).and_then(string_value)
        }
        Meta::Path(_) => None,
    }
}

fn is_doc_hidden(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("doc")
            && attr.parse_args::<syn::Ident>().is_ok_and(|ident| ident == "hidden")
    })
}

fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr.parse_args::<syn::Ident>().is_ok_and(|ident| ident == "test")
    })
}

fn rename(name: &str, rule: Option<&str>) -> String {
    match rule {
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        Some("snake_case") => name.to_snake_case(),
        Some("kebab-case") => name.to_kebab_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_upper_camel_case(),
        Some("SCREAMING_SNAKE_CASE") => name.to_shouty_snake_case(),
        _ => name.to_string(),
    }
}

/// Returns the field's doc comment as Markdown, with rustdoc-only links reduced to their text.
fn docs(attrs: &[Attribute]) -> String {
    let lines = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .filter_map(|attr| match &attr.meta {
            Meta::NameValue(nv) => match &nv.value {
                Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => Some(s.value()),
                _ => None,
            },
            _ => None,
        })
        .map(|line| line.strip_prefix(' ').map(str::to_string).unwrap_or(line))
        .collect::<Vec<_>>();
    let text = lines.join("\n");
    let intra_doc = regex::Regex::new(r"\[([^\]]+)\]\(([^)]*)\)").unwrap();
    let text = intra_doc.replace_all(&text, |caps: &regex::Captures<'_>| {
        let target = &caps[2];
        if target.contains("://") || target.starts_with('/') || target.starts_with('#') {
            caps[0].to_string()
        } else {
            caps[1].to_string()
        }
    });
    let bare = regex::Regex::new(r"\[(\x60[^\x60\]]+\x60)\](?:[^(\[]|$)").unwrap();
    let text = bare.replace_all(&text, |caps: &regex::Captures<'_>| {
        let whole = &caps[0];
        let tail = &whole[caps[1].len() + 2..];
        format!("{}{tail}", &caps[1])
    });
    text.trim().to_string()
}

/// The last path segment of a type and its generic arguments.
fn segment(ty: &Type) -> Option<(String, Vec<&Type>)> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    let args = match &last.arguments {
        PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgument::Type(ty) => Some(ty),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some((last.ident.to_string(), args))
}

fn unwrap_option(ty: &Type) -> &Type {
    match segment(ty) {
        Some((name, args)) if name == "Option" && args.len() == 1 => args[0],
        _ => ty,
    }
}

struct Generator {
    defs: Definitions,
    defaults: Value,
    out: Vec<Setting>,
}

impl Generator {
    fn type_name(&self, ty: &Type) -> String {
        let ty = unwrap_option(ty);
        if let Type::Tuple(tuple) = ty {
            let items = tuple.elems.iter().map(|ty| self.type_name(ty)).collect::<Vec<_>>();
            return format!("[{}]", items.join(", "));
        }
        let Some((name, args)) = segment(ty) else { return "value".to_string() };
        match name.as_str() {
            "bool" => "boolean".to_string(),
            "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64"
            | "i128" | "isize" | "NonZeroU32" | "NonZeroU64" | "NonZeroUsize" => {
                "integer".to_string()
            }
            "f32" | "f64" => "number".to_string(),
            "Vec" | "HashSet" | "BTreeSet" if args.len() == 1 => {
                format!("array of {}", self.type_name(args[0]))
            }
            "HashMap" | "BTreeMap" | "IndexMap" | "AddressHashMap" => "table".to_string(),
            _ => {
                if let Some((_, external)) = EXTERNAL_TYPES.iter().find(|(n, _)| *n == name) {
                    return external.to_string();
                }
                if self.defs.enums.contains_key(&name) {
                    return "string".to_string();
                }
                if let Some(item) = self.defs.structs.get(&name) {
                    if SerdeAttrs::parse(&item.attrs).transparent
                        && let Some(field) = item.fields.iter().next()
                    {
                        return self.type_name(&field.ty);
                    }
                    return match &item.fields {
                        Fields::Named(_) => "table".to_string(),
                        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                            self.type_name(&fields.unnamed[0].ty)
                        }
                        _ => "value".to_string(),
                    };
                }
                "value".to_string()
            }
        }
    }

    /// Accepted values of a unit-only enum defined in this crate.
    fn enum_values(&self, ty: &Type) -> Vec<String> {
        let Some((name, _)) = segment(unwrap_option(ty)) else { return Vec::new() };
        let Some(item) = self.defs.enums.get(&name) else { return Vec::new() };
        if item.variants.iter().any(|v| !matches!(v.fields, Fields::Unit)) {
            return Vec::new();
        }
        let container = SerdeAttrs::parse(&item.attrs);
        item.variants
            .iter()
            .filter_map(|variant| {
                let attrs = SerdeAttrs::parse(&variant.attrs);
                if attrs.skip {
                    return None;
                }
                Some(attrs.rename.unwrap_or_else(|| {
                    rename(&variant.ident.to_string(), container.rename_all.as_deref())
                }))
            })
            .collect()
    }

    /// The struct that a field's settings are read from, if it is one of this crate's.
    fn table_struct(&self, ty: &Type) -> Option<syn::ItemStruct> {
        let (name, _) = segment(unwrap_option(ty))?;
        let item = self.defs.structs.get(&name)?;
        let transparent = SerdeAttrs::parse(&item.attrs).transparent;
        (matches!(item.fields, Fields::Named(_)) && !transparent).then(|| item.clone())
    }

    /// Emits the settings of `item` below `prefix`.
    ///
    /// With `serialized_only`, fields that the type doesn't serialize are skipped. This filters
    /// legacy aliases out of flattened types with custom `Serialize` impls.
    fn walk(&mut self, item: &syn::ItemStruct, prefix: &[String], serialized_only: bool) {
        let container = SerdeAttrs::parse(&item.attrs);
        let Fields::Named(fields) = &item.fields else { return };
        for field in &fields.named {
            let attrs = SerdeAttrs::parse(&field.attrs);
            // Internal state that is read but never written, such as provenance flags.
            if attrs.skip || (attrs.skip_serializing && is_doc_hidden(&field.attrs)) {
                continue;
            }
            let ident = field.ident.as_ref().unwrap().to_string();
            let ident = ident.strip_prefix("r#").unwrap_or(&ident);
            if attrs.flatten {
                if let Some(inner) = self.table_struct(&field.ty) {
                    self.walk(&inner, prefix, true);
                }
                continue;
            }
            let name = attrs
                .rename
                .clone()
                .unwrap_or_else(|| rename(ident, container.rename_all.as_deref()));
            if name.starts_with('_') || (prefix.is_empty() && EXCLUDED_KEYS.contains(&&*name)) {
                continue;
            }
            let mut path = prefix.to_vec();
            path.push(name);
            let key = path.join(".");

            let default = path.iter().try_fold(&self.defaults, |value, part| value.get(part));
            if serialized_only && default.is_none() {
                continue;
            }
            // Feature-gated fields depend on how Foundry was built.
            if field.attrs.iter().any(|attr| attr.path().is_ident("cfg")) {
                continue;
            }
            let children = self.table_struct(&field.ty);
            let section = Config::STANDALONE_SECTIONS
                .iter()
                .find(|section| *section == &path[0])
                .map(|section| section.to_string());
            let deprecated = DEPRECATIONS
                .iter()
                .find(|(old, _)| prefix.is_empty() && *old == path[0])
                .map(|(_, new)| new.to_string());

            let mut ty = self.type_name(&field.ty);
            if ty == "table" && default.is_some_and(Value::is_array) {
                ty = "array of table".to_string();
            }
            self.out.push(Setting {
                env: env_vars(&path, section.is_some(), children.is_some()),
                key,
                section,
                ty,
                table: children.is_some(),
                // A table's default is the defaults of its keys.
                default: default
                    .filter(|value| !value.is_null() && children.is_none())
                    .map(toml_value),
                values: self.enum_values(&field.ty),
                aliases: attrs.aliases,
                deprecated,
                description: docs(&field.attrs),
            });

            if let Some(children) = children {
                self.walk(&children, &path, false);
            }
        }
    }
}

/// Environment variables that set a key, following the providers in [`Config::figment`].
fn env_vars(path: &[String], standalone: bool, table: bool) -> Vec<String> {
    if let Some((_, vars)) = SPECIAL_ENV.iter().find(|(key, _)| path.len() == 1 && *key == path[0])
    {
        return vars.iter().map(ToString::to_string).collect();
    }
    let upper = path.iter().map(|part| part.to_uppercase()).collect::<Vec<_>>().join("_");
    match path.len() {
        1 if !standalone && !table => {
            let mut vars = Vec::new();
            if !Config::FOUNDRY_ENV_IGNORED.contains(&&*upper) {
                vars.push(format!("FOUNDRY_{upper}"));
            }
            if !Config::DAPP_ENV_IGNORED.contains(&&*upper) {
                vars.push(format!("DAPP_{upper}"));
            }
            vars
        }
        2 if standalone && !table => vec![format!("FOUNDRY_{upper}")],
        _ => Vec::new(),
    }
}

/// Renders a JSON value as an inline TOML value.
fn toml_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap(),
        Value::Array(items) => {
            format!("[{}]", items.iter().map(toml_value).collect::<Vec<_>>().join(", "))
        }
        Value::Object(map) if map.is_empty() => "{}".to_string(),
        Value::Object(map) => {
            let entries = map
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| {
                    let key =
                        if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                            key.clone()
                        } else {
                            serde_json::to_string(key).unwrap()
                        };
                    format!("{key} = {}", toml_value(value))
                })
                .collect::<Vec<_>>();
            format!("{{ {} }}", entries.join(", "))
        }
    }
}

fn generate() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Flattened network settings are defined in `foundry-evm-networks`.
    let mut defs = Definitions::load(&root.join("src"));
    let networks = Definitions::load(&root.join("../evm/networks/src"));
    for (name, item) in networks.structs {
        defs.structs.entry(name).or_insert(item);
    }
    for (name, item) in networks.enums {
        defs.enums.entry(name).or_insert(item);
    }
    let config = defs.structs.get("Config").expect("Config struct").clone();
    let defaults = serde_json::to_value(Config::default()).unwrap();
    let mut generator = Generator { defs, defaults, out: Vec::new() };
    generator.walk(&config, &[], false);

    let mut seen = BTreeSet::new();
    let mut undocumented = Vec::new();
    for setting in &generator.out {
        assert!(seen.insert(&setting.key), "duplicate key {}", setting.key);
        if setting.description.is_empty() {
            undocumented.push(setting.key.as_str());
        }
    }
    assert!(
        undocumented.is_empty(),
        "document these settings in the doc comments of their fields: {undocumented:?}"
    );

    let reference = Reference { schema_version: 1, settings: generator.out };
    let mut json = serde_json::to_string_pretty(&reference).unwrap();
    json.push('\n');
    json
}

/// Checks that the committed reference is up to date. If not, updates it and fails.
#[test]
#[expect(clippy::disallowed_macros)]
fn reference_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REFERENCE_PATH);
    let contents = generate();
    if fs::read_to_string(&path).is_ok_and(|old| old.replace("\r\n", "\n") == contents) {
        return;
    }

    eprintln!("\n\x1b[31;1merror\x1b[0m: {} was not up-to-date, updating\n", path.display());
    if std::env::var("CI").is_ok() {
        eprintln!("    NOTE: run `cargo config-reference` locally and commit the updated file\n");
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, contents).unwrap();
    panic!("the config reference was not up to date and has been updated, simply re-run the tests");
}
