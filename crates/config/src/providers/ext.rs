use crate::{Config, extend, utils};
use figment::{
    Error, Figment, Metadata, Profile, Provider,
    providers::{Env, Format, Toml},
    value::{Dict, Map, Value},
};
use foundry_compilers::ProjectPathsConfig;
use heck::ToSnakeCase;
use std::{
    cell::OnceCell,
    path::{Path, PathBuf},
};

pub(crate) trait ProviderExt: Provider + Sized {
    fn rename(
        self,
        from: impl Into<Profile>,
        to: impl Into<Profile>,
    ) -> RenameProfileProvider<Self> {
        RenameProfileProvider::new(self, from, to)
    }

    fn wrap(
        self,
        wrapping_key: impl Into<Profile>,
        profile: impl Into<Profile>,
    ) -> WrapProfileProvider<Self> {
        WrapProfileProvider::new(self, wrapping_key, profile)
    }

    fn strict_select(
        self,
        profiles: impl IntoIterator<Item = impl Into<Profile>>,
    ) -> OptionalStrictProfileProvider<Self> {
        OptionalStrictProfileProvider::new(self, profiles)
    }

    fn fallback(
        self,
        profile: impl Into<Profile>,
        fallback: impl Into<Profile>,
    ) -> FallbackProfileProvider<Self> {
        FallbackProfileProvider::new(self, profile, fallback)
    }
}

impl<P: Provider> ProviderExt for P {}

/// A convenience provider to retrieve a toml file.
/// This will return an error if the env var is set but the file does not exist
pub(crate) struct TomlFileProvider {
    env_var: Option<&'static str>,
    env_val: OnceCell<Option<String>>,
    default: PathBuf,
    cache: OnceCell<Result<Map<Profile, Dict>, Error>>,
}

impl TomlFileProvider {
    pub(crate) fn new(env_var: Option<&'static str>, default: PathBuf) -> Self {
        Self { env_var, env_val: OnceCell::new(), default, cache: OnceCell::new() }
    }

    fn env_val(&self) -> Option<&str> {
        self.env_val.get_or_init(|| self.env_var.and_then(Env::var)).as_deref()
    }

    fn file(&self) -> PathBuf {
        self.env_val().map(PathBuf::from).unwrap_or_else(|| self.default.clone())
    }

    fn is_missing(&self) -> bool {
        if let Some(file) = self.env_val() {
            let path = Path::new(&file);
            if !path.exists() {
                return true;
            }
        }
        false
    }

    /// Reads and processes the TOML configuration file, handling inheritance if configured.
    fn read(&self) -> Result<Map<Profile, Dict>, Error> {
        use serde::de::Error as _;

        // Get the config file path and validate it exists
        let local_path = self.file();
        if !local_path.exists() {
            if let Some(file) = self.env_val() {
                return Err(Error::custom(format!(
                    "Config file `{}` set in env var `{}` does not exist",
                    file,
                    self.env_var.unwrap()
                )));
            }
            return Ok(Map::new());
        }

        // Create a provider for the local config file
        let local_provider = Toml::file(local_path.clone()).nested();

        // Parse the local config to check for extends field
        let local_path_str = local_path.to_string_lossy();
        let local_content = std::fs::read_to_string(&local_path)
            .map_err(|e| Error::custom(e.to_string()).with_path(&local_path_str))?;
        let partial_config: extend::ExtendsPartialConfig = toml::from_str(&local_content)
            .map_err(|e| Error::custom(e.to_string()).with_path(&local_path_str))?;

        // Check if the currently active profile has an 'extends' field
        let selected_profile = Config::selected_profile();
        let extends_config = partial_config.profile.as_ref().and_then(|profiles| {
            let profile_str = selected_profile.to_string();
            profiles.get(&profile_str).and_then(|cfg| cfg.extends.as_ref())
        });

        // If inheritance is configured, load and merge the base config
        if let Some(extends_config) = extends_config {
            let extends_path = extends_config.path();
            let extends_strategy = extends_config.strategy();
            let relative_base_path = PathBuf::from(extends_path);
            let local_dir = local_path.parent().ok_or_else(|| {
                Error::custom(format!(
                    "Could not determine parent directory of config file: {}",
                    local_path.display()
                ))
            })?;

            let base_path =
                foundry_compilers::utils::canonicalize(local_dir.join(&relative_base_path))
                    .map_err(|e| {
                        Error::custom(format!(
                            "Failed to resolve inherited config path: {}: {e}",
                            relative_base_path.display()
                        ))
                    })?;

            // Validate the base config file exists
            if !base_path.is_file() {
                return Err(Error::custom(format!(
                    "Inherited config file does not exist or is not a file: {}",
                    base_path.display()
                )));
            }

            // Prevent self-inheritance which would cause infinite recursion
            if foundry_compilers::utils::canonicalize(&local_path).ok().as_ref() == Some(&base_path)
            {
                return Err(Error::custom(format!(
                    "Config file {} cannot inherit from itself.",
                    local_path.display()
                )));
            }

            // Parse the base config to check for nested inheritance
            let base_path_str = base_path.to_string_lossy();
            let base_content = std::fs::read_to_string(&base_path)
                .map_err(|e| Error::custom(e.to_string()).with_path(&base_path_str))?;
            let base_partial: extend::ExtendsPartialConfig = toml::from_str(&base_content)
                .map_err(|e| Error::custom(e.to_string()).with_path(&base_path_str))?;

            // Check if the base file's same profile also has extends (nested inheritance)
            let base_extends = base_partial
                .profile
                .as_ref()
                .and_then(|profiles| {
                    let profile_str = selected_profile.to_string();
                    profiles.get(&profile_str)
                })
                .and_then(|profile| profile.extends.as_ref());

            // Prevent nested inheritance to avoid complexity and potential cycles
            if base_extends.is_some() {
                return Err(Error::custom(format!(
                    "Nested inheritance is not allowed. Base file '{}' cannot have an 'extends' field in profile '{selected_profile}'.",
                    base_path.display()
                )));
            }

            // Load base configuration as a Figment provider
            let base_provider = Toml::file(base_path).nested();

            // Apply the selected merge strategy
            match extends_strategy {
                extend::ExtendStrategy::ExtendArrays => {
                    // Using 'admerge' strategy:
                    // - Arrays are concatenated (base elements + local elements)
                    // - Other values are replaced (local values override base values)
                    // - The extends field is preserved in the final configuration
                    Figment::new().merge(base_provider).admerge(local_provider).data()
                }
                extend::ExtendStrategy::ReplaceArrays => {
                    // Using 'merge' strategy:
                    // - Arrays are replaced entirely (local arrays replace base arrays)
                    // - Other values are replaced (local values override base values)
                    Figment::new().merge(base_provider).merge(local_provider).data()
                }
                extend::ExtendStrategy::NoCollision => {
                    // Check for key collisions between base and local configs
                    let base_data = base_provider.data()?;
                    let local_data = local_provider.data()?;

                    let profile_key = Profile::new("profile");
                    if let (Some(local_profiles), Some(base_profiles)) =
                        (local_data.get(&profile_key), base_data.get(&profile_key))
                    {
                        // Extract dicts for the selected profile
                        let profile_str = selected_profile.to_string();
                        let base_dict = base_profiles.get(&profile_str).and_then(|v| v.as_dict());
                        let local_dict = local_profiles.get(&profile_str).and_then(|v| v.as_dict());

                        // Find colliding keys
                        if let (Some(local_dict), Some(base_dict)) = (local_dict, base_dict) {
                            let collisions: Vec<&String> = local_dict
                                .keys()
                                .filter(|key| {
                                    // Ignore the "extends" key as it's expected
                                    *key != "extends" && base_dict.contains_key(*key)
                                })
                                .collect();

                            if !collisions.is_empty() {
                                return Err(Error::custom(format!(
                                    "Key collision detected in profile '{profile_str}' when extending '{extends_path}'. \
                                    Conflicting keys: {collisions:?}. Use 'extends.strategy' or 'extends_strategy' to specify how to handle conflicts."
                                )));
                            }
                        }
                    }

                    // Safe to merge the configs without collisions
                    Figment::new().merge(base_provider).merge(local_provider).data()
                }
            }
        } else {
            // No inheritance - return the local config as-is
            local_provider.data()
        }
    }
}

impl Provider for TomlFileProvider {
    fn metadata(&self) -> Metadata {
        if self.is_missing() {
            Metadata::named("TOML file provider")
        } else {
            Toml::file(self.file()).nested().metadata()
        }
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        self.cache.get_or_init(|| self.read()).clone()
    }
}

/// A Provider that ensures all keys are snake case if they're not standalone sections, See
/// `Config::STANDALONE_SECTIONS`
pub(crate) struct ForcedSnakeCaseData<P>(pub(crate) P);

impl<P: Provider> Provider for ForcedSnakeCaseData<P> {
    fn metadata(&self) -> Metadata {
        self.0.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut map = self.0.data()?;
        for (profile, dict) in &mut map {
            if Config::STANDALONE_SECTIONS.contains(&profile.as_ref()) {
                // don't force snake case for keys in standalone sections
                continue;
            }
            let dict2 = std::mem::take(dict);
            *dict = dict2.into_iter().map(|(k, v)| (k.to_snake_case(), v)).collect();
        }
        Ok(map)
    }

    fn profile(&self) -> Option<Profile> {
        self.0.profile()
    }
}

/// A Provider that handles breaking changes in toml files
pub(crate) struct BackwardsCompatTomlProvider<P>(pub(crate) P);

impl<P: Provider> Provider for BackwardsCompatTomlProvider<P> {
    fn metadata(&self) -> Metadata {
        self.0.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut map = Map::new();
        let solc_env = std::env::var("FOUNDRY_SOLC_VERSION")
            .or_else(|_| std::env::var("DAPP_SOLC_VERSION"))
            .map(Value::from)
            .ok();
        for (profile, mut dict) in self.0.data()? {
            if let Some(v) = solc_env.clone() {
                // ENV var takes precedence over config file
                dict.insert("solc".to_string(), v);
            } else if let Some(v) = dict.remove("solc_version") {
                // only insert older variant if not already included
                if !dict.contains_key("solc") {
                    dict.insert("solc".to_string(), v);
                }
            }
            if let Some(v) = dict.remove("deny_warnings")
                && !dict.contains_key("deny")
            {
                dict.insert("deny".to_string(), v);
            }

            map.insert(profile, dict);
        }
        Ok(map)
    }

    fn profile(&self) -> Option<Profile> {
        self.0.profile()
    }
}

/// A provider that sets the `src` and `output` path depending on their existence.
pub(crate) struct DappHardhatDirProvider<'a>(pub(crate) &'a Path);

impl Provider for DappHardhatDirProvider<'_> {
    fn metadata(&self) -> Metadata {
        Metadata::named("Dapp Hardhat dir compat")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = Dict::new();
        dict.insert(
            "src".to_string(),
            ProjectPathsConfig::find_source_dir(self.0)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
                .into(),
        );
        dict.insert(
            "out".to_string(),
            ProjectPathsConfig::find_artifacts_dir(self.0)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
                .into(),
        );

        // detect libs folders:
        //   if `lib` _and_ `node_modules` exists: include both
        //   if only `node_modules` exists: include `node_modules`
        //   include `lib` otherwise
        let mut libs = vec![];
        let node_modules = self.0.join("node_modules");
        let lib = self.0.join("lib");
        if node_modules.exists() {
            if lib.exists() {
                libs.push(lib.file_name().unwrap().to_string_lossy().to_string());
            }
            libs.push(node_modules.file_name().unwrap().to_string_lossy().to_string());
        } else {
            libs.push(lib.file_name().unwrap().to_string_lossy().to_string());
        }

        dict.insert("libs".to_string(), libs.into());

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// A provider that checks for DAPP_ env vars that are named differently than FOUNDRY_
pub(crate) struct DappEnvCompatProvider;

impl Provider for DappEnvCompatProvider {
    fn metadata(&self) -> Metadata {
        Metadata::named("Dapp env compat")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        use serde::de::Error as _;
        use std::env;

        let mut dict = Dict::new();
        if let Ok(val) = env::var("DAPP_TEST_NUMBER") {
            dict.insert(
                "block_number".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_TEST_ADDRESS") {
            dict.insert("sender".to_string(), val.into());
        }
        if let Ok(val) = env::var("DAPP_FORK_BLOCK") {
            dict.insert(
                "fork_block_number".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        } else if let Ok(val) = env::var("DAPP_TEST_NUMBER") {
            dict.insert(
                "fork_block_number".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_TEST_TIMESTAMP") {
            dict.insert(
                "block_timestamp".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_BUILD_OPTIMIZE_RUNS") {
            dict.insert(
                "optimizer_runs".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_BUILD_OPTIMIZE") {
            // Activate Solidity optimizer (0 or 1)
            let val = val.parse::<u8>().map_err(figment::Error::custom)?;
            if val > 1 {
                return Err(
                    format!("Invalid $DAPP_BUILD_OPTIMIZE value `{val}`, expected 0 or 1").into()
                );
            }
            dict.insert("optimizer".to_string(), (val == 1).into());
        }

        // libraries in env vars either as `[..]` or single string separated by comma
        if let Ok(val) = env::var("DAPP_LIBRARIES").or_else(|_| env::var("FOUNDRY_LIBRARIES")) {
            dict.insert("libraries".to_string(), utils::to_array_value(&val)?);
        }

        let mut fuzz_dict = Dict::new();
        if let Ok(val) = env::var("DAPP_TEST_FUZZ_RUNS") {
            fuzz_dict.insert(
                "runs".to_string(),
                val.parse::<u32>().map_err(figment::Error::custom)?.into(),
            );
        }
        dict.insert("fuzz".to_string(), fuzz_dict.into());

        let mut invariant_dict = Dict::new();
        if let Ok(val) = env::var("DAPP_TEST_DEPTH") {
            invariant_dict.insert(
                "depth".to_string(),
                val.parse::<u32>().map_err(figment::Error::custom)?.into(),
            );
        }
        dict.insert("invariant".to_string(), invariant_dict.into());

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// Renames a profile from `from` to `to`.
///
/// For example given:
///
/// ```toml
/// [from]
/// key = "value"
/// ```
///
/// RenameProfileProvider will output
///
/// ```toml
/// [to]
/// key = "value"
/// ```
pub(crate) struct RenameProfileProvider<P> {
    provider: P,
    from: Profile,
    to: Profile,
}

impl<P> RenameProfileProvider<P> {
    pub(crate) fn new(provider: P, from: impl Into<Profile>, to: impl Into<Profile>) -> Self {
        Self { provider, from: from.into(), to: to.into() }
    }
}

impl<P: Provider> Provider for RenameProfileProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut data = self.provider.data()?;
        if let Some(data) = data.remove(&self.from) {
            return Ok(Map::from([(self.to.clone(), data)]));
        }
        Ok(Default::default())
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.to.clone())
    }
}

/// Unwraps a profile reducing the key depth
///
/// For example given:
///
/// ```toml
/// [wrapping_key.profile]
/// key = "value"
/// ```
///
/// UnwrapProfileProvider will output:
///
/// ```toml
/// [profile]
/// key = "value"
/// ```
struct UnwrapProfileProvider<P> {
    provider: P,
    wrapping_key: Profile,
    profile: Profile,
}

impl<P> UnwrapProfileProvider<P> {
    pub fn new(provider: P, wrapping_key: impl Into<Profile>, profile: impl Into<Profile>) -> Self {
        Self { provider, wrapping_key: wrapping_key.into(), profile: profile.into() }
    }
}

impl<P: Provider> Provider for UnwrapProfileProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut data = self.provider.data()?;
        if let Some(profiles) = data.remove(&self.wrapping_key) {
            for (profile_str, profile_val) in profiles {
                let profile = Profile::new(&profile_str);
                if profile != self.profile {
                    continue;
                }
                match profile_val {
                    Value::Dict(_, dict) => return Ok(profile.collect(dict)),
                    bad_val => {
                        let mut err = Error::from(figment::error::Kind::InvalidType(
                            bad_val.to_actual(),
                            "dict".into(),
                        ));
                        err.metadata = Some(self.provider.metadata());
                        err.profile = Some(self.profile.clone());
                        return Err(err);
                    }
                }
            }
        }
        Ok(Default::default())
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}

/// Wraps a profile in another profile
///
/// For example given:
///
/// ```toml
/// [profile]
/// key = "value"
/// ```
///
/// WrapProfileProvider will output:
///
/// ```toml
/// [wrapping_key.profile]
/// key = "value"
/// ```
pub(crate) struct WrapProfileProvider<P> {
    provider: P,
    wrapping_key: Profile,
    profile: Profile,
}

impl<P> WrapProfileProvider<P> {
    pub fn new(provider: P, wrapping_key: impl Into<Profile>, profile: impl Into<Profile>) -> Self {
        Self { provider, wrapping_key: wrapping_key.into(), profile: profile.into() }
    }
}

impl<P: Provider> Provider for WrapProfileProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        if let Some(inner) = self.provider.data()?.remove(&self.profile) {
            let value = Value::from(inner);
            let mut dict = Dict::new();
            dict.insert(self.profile.as_str().as_str().to_snake_case(), value);
            Ok(self.wrapping_key.collect(dict))
        } else {
            Ok(Default::default())
        }
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}

/// Extracts the profile from the `profile` key and using the original key as backup, merging
/// values where necessary
///
/// For example given:
///
/// ```toml
/// [profile.cool]
/// key = "value"
///
/// [cool]
/// key2 = "value2"
/// ```
///
/// OptionalStrictProfileProvider will output:
///
/// ```toml
/// [cool]
/// key = "value"
/// key2 = "value2"
/// ```
///
/// And emit a deprecation warning
pub(crate) struct OptionalStrictProfileProvider<P> {
    provider: P,
    profiles: Vec<Profile>,
}

impl<P> OptionalStrictProfileProvider<P> {
    pub const PROFILE_PROFILE: Profile = Profile::const_new("profile");

    pub fn new(provider: P, profiles: impl IntoIterator<Item = impl Into<Profile>>) -> Self {
        Self { provider, profiles: profiles.into_iter().map(|profile| profile.into()).collect() }
    }
}

impl<P: Provider> Provider for OptionalStrictProfileProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut figment = Figment::from(&self.provider);
        for profile in &self.profiles {
            figment = figment.merge(UnwrapProfileProvider::new(
                &self.provider,
                Self::PROFILE_PROFILE,
                profile.clone(),
            ));
        }
        figment.data().map_err(|err| {
            // figment does tag metadata and tries to map metadata to an error, since we use a new
            // figment in this provider this new figment does not know about the metadata of the
            // provider and can't map the metadata to the error. Therefore we return the root error
            // if this error originated in the provider's data.
            if let Err(root_err) = self.provider.data() {
                return root_err;
            }
            err
        })
    }

    fn profile(&self) -> Option<Profile> {
        self.profiles.last().cloned()
    }
}

/// Extracts the profile from the `profile` key and sets unset values according to the fallback
/// provider
pub struct FallbackProfileProvider<P> {
    provider: P,
    profile: Profile,
    fallback: Profile,
}

impl<P> FallbackProfileProvider<P> {
    /// Creates a new fallback profile provider.
    pub fn new(provider: P, profile: impl Into<Profile>, fallback: impl Into<Profile>) -> Self {
        Self { provider, profile: profile.into(), fallback: fallback.into() }
    }
}

impl<P: Provider> Provider for FallbackProfileProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut data = self.provider.data()?;
        if let Some(fallback) = data.remove(&self.fallback) {
            let mut inner = data.remove(&self.profile).unwrap_or_default();
            for (k, v) in fallback {
                inner.entry(k).or_insert(v);
            }
            Ok(self.profile.collect(inner))
        } else {
            Ok(data)
        }
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}
