//! Where each config value was set, for `forge config --show-origin`.

use crate::Config;
use figment::{Figment, Metadata, Profile, Source};
use heck::ToSnakeCase;
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    path::{Path, PathBuf},
};

/// The name of the provider of Foundry's defaults.
const DEFAULT_PROVIDER: &str = "Foundry Config";

/// Keys that a config file may set under another name, as `(key, name)`.
const LEGACY_KEYS: &[(&str, &str)] =
    &[("chain_id", "chain"), ("solc", "solc_version"), ("deny", "deny_warnings")];

/// The variables `DappEnvCompatProvider` reads, as `(key, variable)`.
const DAPP_ENV_COMPAT: &[(&str, &str)] = &[
    ("block_number", "DAPP_TEST_NUMBER"),
    ("sender", "DAPP_TEST_ADDRESS"),
    ("fork_block_number", "DAPP_FORK_BLOCK"),
    ("fork_block_number", "DAPP_TEST_NUMBER"),
    ("block_timestamp", "DAPP_TEST_TIMESTAMP"),
    ("optimizer_runs", "DAPP_BUILD_OPTIMIZE_RUNS"),
    ("optimizer", "DAPP_BUILD_OPTIMIZE"),
];

/// Where a config value was set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueOrigin {
    /// Foundry's default.
    Default,
    /// A config file, and the table that holds the key when it could be found.
    File {
        /// The file.
        path: PathBuf,
        /// The table that sets the key, such as `profile.ci` or `fuzz`.
        table: Option<String>,
    },
    /// An environment variable, by name when it could be determined.
    Env(Option<String>),
    /// A command-line flag.
    CommandLine,
    /// Detected from the project, such as remappings or the source directory.
    Detected,
    /// Computed by forge from other values.
    Derived,
}

impl fmt::Display for ValueOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::File { path, table: Some(table) } => write!(f, "{} [{table}]", path.display()),
            Self::File { path, table: None } => write!(f, "{}", path.display()),
            Self::Env(Some(var)) => write!(f, "env {var}"),
            Self::Env(None) => f.write_str("environment"),
            Self::CommandLine => f.write_str("command line"),
            Self::Detected => f.write_str("detected"),
            Self::Derived => f.write_str("derived"),
        }
    }
}

/// Finds the origin of config values in a [`Figment`].
#[derive(Debug)]
pub struct OriginResolver<'a> {
    figment: &'a Figment,
    root: PathBuf,
    /// Parsed config files, by path.
    files: HashMap<PathBuf, Option<toml::Table>>,
}

impl<'a> OriginResolver<'a> {
    /// Creates a resolver for the values of `figment`, showing paths relative to `root`.
    pub fn new(figment: &'a Figment, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = dunce::canonicalize(&root).unwrap_or(root);
        Self { figment, root, files: HashMap::new() }
    }

    /// Returns the origin of the value at `key`, a dotted path such as `fuzz.runs`, or `None`
    /// if the figment has no such value.
    pub fn origin(&mut self, key: &str) -> Option<ValueOrigin> {
        // `BackwardsCompatTomlProvider` writes these variables into the file's data.
        if key == "solc"
            && let Some(var) = ["FOUNDRY_SOLC_VERSION", "DAPP_SOLC_VERSION"]
                .into_iter()
                .find(|var| std::env::var_os(var).is_some())
        {
            return Some(ValueOrigin::Env(Some(var.to_string())));
        }
        let mut metadata = self.figment.find_metadata(key)?;
        // A value set under an alias is stored under the alias.
        if metadata.name == DEFAULT_PROVIDER {
            for (_, alias) in LEGACY_KEYS.iter().filter(|(k, _)| *k == key) {
                if let Some(aliased) = self.figment.find_metadata(alias)
                    && aliased.name != DEFAULT_PROVIDER
                {
                    metadata = aliased;
                }
            }
        }
        Some(self.origin_of(key, metadata))
    }

    fn origin_of(&mut self, key: &str, metadata: &Metadata) -> ValueOrigin {
        if let Some(Source::File(path)) = &metadata.source {
            let table = self.table_of(path, key);
            return ValueOrigin::File { path: self.display_path(path), table };
        }
        let name = metadata.name.as_ref();
        match name {
            DEFAULT_PROVIDER => ValueOrigin::Default,
            "Remapping Provider" | "Dapp Hardhat dir compat" => ValueOrigin::Detected,
            "Dapp env compat" => ValueOrigin::Env(
                DAPP_ENV_COMPAT
                    .iter()
                    .filter(|(k, _)| *k == key)
                    .map(|(_, var)| *var)
                    .find(|var| std::env::var_os(var).is_some())
                    .map(String::from),
            ),
            _ if name.contains("environment variable") => ValueOrigin::Env(env_var_for(key)),
            // Values that forge merges itself, e.g. `evm_version` normalized for `solc`.
            _ if metadata.source.is_none() && name.contains("::") => ValueOrigin::Derived,
            _ => ValueOrigin::CommandLine,
        }
    }

    /// Returns the table of the config file at `path` that sets `key`.
    fn table_of(&mut self, path: &Path, key: &str) -> Option<String> {
        let file = self
            .files
            .entry(path.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(path).ok()?.parse::<toml::Table>().ok())
            .as_ref()?;
        let profile = self.figment.profile();
        let (first, rest) = key.split_once('.').unwrap_or((key, ""));
        let mut candidates = Vec::new();
        if Config::STANDALONE_SECTIONS.contains(&first) && !rest.is_empty() {
            let sections = std::iter::once(first).chain(
                crate::STANDALONE_FALLBACK_SECTIONS
                    .iter()
                    .filter(|(section, _)| *section == first)
                    .map(|(_, fallback)| *fallback),
            );
            for section in sections {
                for p in profiles(profile) {
                    candidates.push((format!("profile.{p}.{section}"), rest));
                }
                candidates.push((section.to_string(), rest));
            }
        } else {
            let names = std::iter::once(key)
                .chain(LEGACY_KEYS.iter().filter(|(k, _)| *k == key).map(|(_, alias)| *alias));
            for name in names {
                for p in profiles(profile) {
                    candidates.push((format!("profile.{p}"), name));
                }
            }
        }
        candidates.into_iter().find(|(table, key)| contains(file, table, key)).map(|(t, _)| t)
    }

    /// Adds the origin of each value to `toml`, the output of [`Config::to_string_pretty`], as a
    /// trailing comment, and returns the origins by key.
    ///
    /// `loaded` is the same output before forge adjusted any values, e.g. with
    /// [`Config::normalized_evm_version`]. Values that differ from it, and values attributed to
    /// the default that differ from Foundry's default, are marked as derived.
    pub fn annotate(
        &mut self,
        toml: &str,
        loaded: &str,
    ) -> Result<(String, BTreeMap<String, String>), toml_edit::TomlError> {
        let mut loaded = loaded.parse::<toml_edit::DocumentMut>()?;
        let mut loaded_values = BTreeMap::new();
        for_each_value(&mut loaded, &mut |path, item| {
            loaded_values.insert(path.to_string(), value_string(item));
        });
        let defaults = Config { profile: self.figment.profile().clone(), ..Default::default() };
        let mut defaults = defaults
            .to_string_pretty()
            .ok()
            .and_then(|defaults| defaults.parse::<toml_edit::DocumentMut>().ok())
            .unwrap_or_default();
        let mut default_values = BTreeMap::new();
        for_each_value(&mut defaults, &mut |path, item| {
            default_values.insert(path.to_string(), value_string(item));
        });
        let mut doc = toml.parse::<toml_edit::DocumentMut>()?;
        let mut origins = BTreeMap::new();
        for_each_value(&mut doc, &mut |path, item| {
            let value = value_string(item);
            let origin = if loaded_values.get(path) == Some(&value) {
                match self.origin(path) {
                    // Set while loading, e.g. `optimizer` turned on by `optimizer_runs`.
                    Some(ValueOrigin::Default) if default_values.get(path) != Some(&value) => {
                        ValueOrigin::Derived
                    }
                    Some(origin) => origin,
                    None => return,
                }
            } else {
                ValueOrigin::Derived
            };
            let origin = origin.to_string();
            match item {
                toml_edit::Item::Value(value) => {
                    value.decor_mut().set_suffix(format!(" # {origin}"));
                }
                toml_edit::Item::ArrayOfTables(tables) => {
                    if let Some(first) = tables.get_mut(0) {
                        first.decor_mut().set_suffix(format!(" # {origin}"));
                    }
                }
                _ => {}
            }
            origins.insert(path.to_string(), origin);
        });
        Ok((doc.to_string(), origins))
    }

    fn display_path(&self, path: &Path) -> PathBuf {
        let path = &dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Ok(relative) = path.strip_prefix(&self.root) {
            return relative.to_path_buf();
        }
        if let Some(home) = dirs::home_dir()
            && let Ok(relative) = path.strip_prefix(home)
        {
            return Path::new("~").join(relative);
        }
        path.clone()
    }
}

/// Calls `f` with the key path and item of each value in the output of
/// [`Config::to_string_pretty`]: keys of `[profile.<name>]` without the prefix, keys of
/// standalone sections with theirs.
fn for_each_value(
    doc: &mut toml_edit::DocumentMut,
    f: &mut impl FnMut(&str, &mut toml_edit::Item),
) {
    fn walk(
        table: &mut toml_edit::Table,
        prefix: &str,
        f: &mut impl FnMut(&str, &mut toml_edit::Item),
    ) {
        for (key, item) in table.iter_mut() {
            let path = format!("{prefix}{key}");
            match item {
                toml_edit::Item::Table(inner) => walk(inner, &format!("{path}."), f),
                toml_edit::Item::None => {}
                item => f(&path, item),
            }
        }
    }
    for (key, item) in doc.as_table_mut().iter_mut() {
        let Some(table) = item.as_table_mut() else { continue };
        if key == Config::PROFILE_SECTION {
            for (_, profile) in table.iter_mut() {
                if let Some(profile) = profile.as_table_mut() {
                    walk(profile, "", f);
                }
            }
        } else {
            walk(table, &format!("{key}."), f);
        }
    }
}

/// The value of `item` without comments or whitespace.
fn value_string(item: &toml_edit::Item) -> String {
    let mut item = item.clone();
    match &mut item {
        toml_edit::Item::Value(value) => value.decor_mut().clear(),
        toml_edit::Item::ArrayOfTables(tables) => {
            tables.iter_mut().for_each(|table| table.decor_mut().clear())
        }
        _ => {}
    }
    item.to_string().trim().to_string()
}

/// The selected profile first, then the default profile.
fn profiles(selected: &Profile) -> Vec<&str> {
    let mut profiles = vec![selected.as_str().as_str()];
    if *selected != Config::DEFAULT_PROFILE {
        profiles.push("default");
    }
    profiles
}

/// Whether `file` has `key` in `table`, both dotted paths.
fn contains(file: &toml::Table, table: &str, key: &str) -> bool {
    let mut current = file;
    let mut segments = table.split('.').chain(key.split('.')).peekable();
    while let Some(segment) = segments.next() {
        // Profile names and map keys are kept as written; config keys are read in snake_case.
        let Some((_, value)) =
            current.iter().find(|(k, _)| *k == segment || k.to_snake_case() == segment)
        else {
            return false;
        };
        if segments.peek().is_none() {
            return true;
        }
        let Some(inner) = value.as_table() else { return false };
        current = inner;
    }
    false
}

/// Returns the environment variable that sets `key`, if one of the names forge reads is set.
fn env_var_for(key: &str) -> Option<String> {
    if key == "etherscan_api_key" && std::env::var_os("ETHERSCAN_API_KEY").is_some() {
        return Some("ETHERSCAN_API_KEY".to_string());
    }
    let name = key.replace('.', "_").to_uppercase();
    ["FOUNDRY_", "DAPP_", "DAPP_TEST_"]
        .iter()
        .map(|prefix| format!("{prefix}{name}"))
        .find(|var| std::env::var_os(var).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origins(keys: &[&str]) -> Vec<String> {
        let figment = Config::figment();
        let root = std::env::current_dir().unwrap();
        let mut resolver = OriginResolver::new(&figment, root);
        keys.iter()
            .map(|key| resolver.origin(key).map(|o| o.to_string()).unwrap_or_default())
            .collect()
    }

    #[test]
    fn origins_of_aliases_legacy_keys_and_names_as_written() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [profile.default]
                chain = 1
                solc_version = "0.8.30"

                [profile.ci-venom]
                via_ir = true

                [rpc_endpoints]
                baseSepolia = "https://sepolia.base.org"
                "#,
            )?;
            jail.set_env("FOUNDRY_PROFILE", "ci-venom");
            jail.set_env("DAPP_BUILD_OPTIMIZE_RUNS", "7");

            assert_eq!(
                origins(&[
                    "chain_id",
                    "solc",
                    "via_ir",
                    "rpc_endpoints.baseSepolia",
                    "optimizer_runs"
                ]),
                [
                    "foundry.toml [profile.default]",
                    "foundry.toml [profile.default]",
                    "foundry.toml [profile.ci-venom]",
                    "foundry.toml [rpc_endpoints]",
                    "env DAPP_BUILD_OPTIMIZE_RUNS",
                ]
            );

            jail.set_env("FOUNDRY_SOLC_VERSION", "0.8.28");
            assert_eq!(origins(&["solc"]), ["env FOUNDRY_SOLC_VERSION"]);
            Ok(())
        });
    }

    #[test]
    fn origins_of_file_env_and_default_values() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [profile.default]
                optimizer_runs = 1
                src = "contracts"

                [profile.ci]
                optimizer_runs = 999
                viaIr = true

                [profile.ci.fmt]
                line_length = 100

                [fuzz]
                runs = 77
                "#,
            )?;
            jail.set_env("FOUNDRY_PROFILE", "ci");
            jail.set_env("FOUNDRY_VERBOSITY", "3");
            jail.set_env("FOUNDRY_FUZZ_SEED", "0x1");

            assert_eq!(
                origins(&[
                    "optimizer_runs",
                    "src",
                    "via_ir",
                    "fmt.line_length",
                    "fuzz.runs",
                    "invariant.runs",
                    "fuzz.seed",
                    "verbosity",
                    "ffi",
                ]),
                [
                    "foundry.toml [profile.ci]",
                    "foundry.toml [profile.default]",
                    "foundry.toml [profile.ci]",
                    "foundry.toml [profile.ci.fmt]",
                    "foundry.toml [fuzz]",
                    // `[invariant]` falls back to `[fuzz]`.
                    "foundry.toml [fuzz]",
                    "env FOUNDRY_FUZZ_SEED",
                    "env FOUNDRY_VERBOSITY",
                    "default",
                ]
            );
            Ok(())
        });
    }
}
