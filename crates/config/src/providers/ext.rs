use crate::{utils, Config};
use figment::{
    providers::{Env, Format, Toml},
    value::{Dict, Map, Value},
    Error, Figment, Metadata, Profile, Provider,
};
use foundry_compilers::ProjectPathsConfig;
use inflector::Inflector;
use std::path::{Path, PathBuf};

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
    pub env_var: Option<&'static str>,
    pub default: PathBuf,
    pub cache: Option<Result<Map<Profile, Dict>, Error>>,
}

impl TomlFileProvider {
    pub(crate) fn new(env_var: Option<&'static str>, default: impl Into<PathBuf>) -> Self {
        Self { env_var, default: default.into(), cache: None }
    }

    fn env_val(&self) -> Option<String> {
        self.env_var.and_then(Env::var)
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

    pub(crate) fn cached(mut self) -> Self {
        self.cache = Some(self.read());
        self
    }

    fn read(&self) -> Result<Map<Profile, Dict>, Error> {
        use serde::de::Error as _;
        if let Some(file) = self.env_val() {
            let path = Path::new(&file);
            if !path.exists() {
                return Err(Error::custom(format!(
                    "Config file `{}` set in env var `{}` does not exist",
                    file,
                    self.env_var.unwrap()
                )));
            }
            Toml::file(file)
        } else {
            Toml::file(&self.default)
        }
        .nested()
        .data()
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
        if let Some(cache) = self.cache.as_ref() {
            cache.clone()
        } else {
            self.read()
        }
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
        let mut map = Map::new();
        for (profile, dict) in self.0.data()? {
            if Config::STANDALONE_SECTIONS.contains(&profile.as_ref()) {
                // don't force snake case for keys in standalone sections
                map.insert(profile, dict);
                continue;
            }
            map.insert(profile, dict.into_iter().map(|(k, v)| (k.to_snake_case(), v)).collect());
        }
        Ok(map)
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

            if let Some(v) = dict.remove("odyssey") {
                dict.insert("odyssey".to_string(), v);
            }
            map.insert(profile, dict);
        }
        Ok(map)
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
        self.provider.data().and_then(|mut data| {
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
        })
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
            let dict = [(self.profile.to_string().to_snake_case(), value)].into_iter().collect();
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
        let data = self.provider.data()?;
        if let Some(fallback) = data.get(&self.fallback) {
            let mut inner = data.get(&self.profile).cloned().unwrap_or_default();
            for (k, v) in fallback.iter() {
                if !inner.contains_key(k) {
                    inner.insert(k.to_owned(), v.clone());
                }
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
