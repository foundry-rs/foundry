//! foundry configuration.
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use ethers_core::types::Address;
use ethers_solc::{remappings::Remapping, EvmVersion, ProjectPathsConfig};
use figment::{
    providers::{Env, Format, Serialized, Toml},
    value::{Dict, Map},
    Error, Figment, Metadata, Profile, Provider,
};
use semver::Version;
use serde::{Deserialize, Serialize};

/// Foundry configuration
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// The selected profile. **(default: _default_ `default`)**
    ///
    /// **Note:** This field is never serialized nor deserialized. When a
    /// `Config` is merged into a `Figment` as a `Provider`, this profile is
    /// selected on the `Figment`. When a `Config` is extracted, this field is
    /// set to the extracting Figment's selected `Profile`.
    #[serde(skip)]
    pub profile: Profile,
    /// path of the source contracts dir, like `src` or `contracts`
    pub src: PathBuf,
    /// path of the test dir
    pub test: PathBuf,
    /// path to where artifacts shut be written to
    pub out: PathBuf,
    /// all library folders to include, `lib`, `node_modules`
    pub libs: Vec<PathBuf>,
    /// `Remappings` to use for this repo
    pub remappings: Vec<RelativeRemapping>,
    /// library addresses to link
    pub libraries: Vec<Address>,
    /// whether to enable cache
    pub cache: bool,
    /// evm version to use
    #[serde(with = "from_str_lowercase")]
    pub evm_version: EvmVersion,
    /// Concrete solc version to use if any.
    ///
    /// This takes precedence over `auto_detect_solc`, if a version is set then this overrides
    /// auto-detection.
    pub solc_version: Option<Version>,
    /// whether to autodetect the solc compiler version to use
    pub auto_detect_solc: bool,
    /// Whether to activate optimizer
    pub optimizer: bool,
    /// Sets the optimizer runs
    pub optimizer_runs: usize,
    /// verbosity to use
    pub verbosity: u8,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// list of solidity error codes to always silence
    pub ignored_error_codes: Vec<u64>,
    /// Settings to pass to the `solc` compiler input
    // TODO consider making this more structured https://stackoverflow.com/questions/48998034/does-toml-support-nested-arrays-of-objects-tables
    pub solc_settings: String,

    /// PRIVATE: This structure may grow, As such, constructing this structure should
    /// _always_ be done using a public constructor or update syntax:
    ///
    /// ```rust
    /// use foundry_config::Config;
    ///
    /// let config = Config {
    ///     src: "other".into(),
    ///     ..Default::default()
    /// };
    /// ```
    #[doc(hidden)]
    #[serde(skip)]
    pub __non_exhaustive: (),
}

impl Config {
    /// The default profile: "default"
    pub const DEFAULT_PROFILE: Profile = Profile::const_new("default");

    /// The hardhat profile: "hardhat"
    pub const HARDHAT_PROFILE: Profile = Profile::const_new("hardhat");

    /// File name of config toml file
    pub const FILE_NAME: &'static str = "foundry.toml";

    /// Returns the current `Config`
    ///
    /// See `Config::figment`
    pub fn load() -> Self {
        Config::from(Config::figment())
    }

    /// Extract a `Config` from `provider`, panicking if extraction fails.
    ///
    /// # Panics
    ///
    /// If extraction fails, prints an error message indicating the failure and
    /// panics. For a version that doesn't panic, use [`Config::try_from()`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use figment::providers::{Toml, Format, Env};
    ///
    /// // Use foundry's default `Figment`, but allow values from `other.toml`
    /// // to supersede its values.
    /// let figment = Config::figment()
    ///     .merge(Toml::file("other.toml").nested());
    ///
    /// let config = Config::from(figment);
    /// ```
    pub fn from<T: Provider>(provider: T) -> Self {
        Self::try_from(provider).expect("failed to extract from provider")
    }

    /// Attempts to extract a `Config` from `provider`, returning the result.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use figment::providers::{Toml, Format, Env};
    ///
    /// // Use foundry's default `Figment`, but allow values from `other.toml`
    /// // to supersede its values.
    /// let figment = Config::figment()
    ///     .merge(Toml::file("other.toml").nested());
    ///
    /// let config = Config::try_from(figment);
    /// ```
    pub fn try_from<T: Provider>(provider: T) -> Result<Self, figment::Error> {
        let figment = Figment::from(provider);
        let mut config = figment.extract::<Self>()?;
        config.profile = figment.profile().clone();
        Ok(config)
    }

    /// Returns the default figment
    ///
    /// The default figment reads from the following sources, in ascending
    /// priority order:
    ///
    ///   1. [`Config::default()`] (see [defaults](#defaults))
    ///   2. `foundry.toml` _or_ filename in `FOUNDRY_CONFIG` environment variable
    ///   3. `FOUNDRY_` prefixed environment variables
    ///
    /// The profile selected is the value set in the `FOUNDRY_PROFILE`
    /// environment variable. If it is not set, it defaults to `default`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use serde::Deserialize;
    ///
    /// let my_config = Config::figment().extract::<Config>();
    /// ```
    pub fn figment() -> Figment {
        Config::default().into()
    }

    /// Returns the default figment enhanced with additional context extracted from the provided
    /// root, like remappings and directories.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use serde::Deserialize;
    ///
    /// let my_config = Config::figment_with_root(".").extract::<Config>();
    /// ```
    pub fn figment_with_root(root: impl Into<PathBuf>) -> Figment {
        Self::with_root(root).into()
    }

    /// Creates a new Config that adds additional context extracted from the provided root.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// let my_config = Config::with_root(".");
    /// ```
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        // autodetect paths
        let root = root.into();
        let paths = ProjectPathsConfig::builder().build_with_root(&root);
        Config {
            src: paths.sources.file_name().unwrap().into(),
            out: paths.artifacts.file_name().unwrap().into(),
            libs: paths.libraries.into_iter().map(|lib| lib.file_name().unwrap().into()).collect(),
            remappings: paths
                .remappings
                .into_iter()
                .map(|r| RelativeRemapping::new(r, &root))
                .collect(),
            ..Config::default()
        }
    }

    /// Returns the default config but with hardhat paths
    pub fn hardhat() -> Self {
        Config {
            src: "contracts".into(),
            out: "artifacts".into(),
            libs: vec!["node_modules".into()],
            ..Config::default()
        }
    }

    /// Extracts a basic subset of the config, used for initialisations.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// let my_config = Config::with_root(".").into_basic();
    /// ```
    pub fn into_basic(self) -> BasicConfig {
        BasicConfig {
            profile: self.profile,
            src: self.src,
            out: self.out,
            libs: self.libs,
            remappings: self.remappings,
        }
    }

    /// Serialize the config type as a String of TOML.
    ///
    /// This serializes to a table with the name of the profile
    pub fn to_string_pretty(&self) -> Result<String, toml::ser::Error> {
        let s = toml::to_string_pretty(self)?;
        Ok(format!(
            r#"[{}]
{}"#,
            self.profile, s
        ))
    }

    /// Returns the selected profile
    ///
    /// If the `FOUNDRY_PROFILE` env variable is not set, this returns the `DEFAULT_PROFILE`
    pub fn selected_profile() -> Profile {
        Profile::from_env_or("FOUNDRY_PROFILE", Config::DEFAULT_PROFILE)
    }

    /// Returns the path to the `foundry.toml` file, the file is searched for in
    /// the current working directory and all parent directories until the root,
    /// and the first hit is used.
    pub fn find_config_file() -> Option<PathBuf> {
        fn find(path: &Path) -> Option<PathBuf> {
            if path.is_absolute() {
                return match path.is_file() {
                    true => Some(path.to_path_buf()),
                    false => None,
                }
            }
            let cwd = std::env::current_dir().ok()?;
            let mut cwd = cwd.as_path();
            loop {
                let file_path = cwd.join(path);
                if file_path.is_file() {
                    return Some(file_path)
                }
                cwd = cwd.parent()?;
            }
        }
        find(Env::var_or("FOUNDRY_CONFIG", Config::FILE_NAME).as_ref())
    }
}

impl From<Config> for Figment {
    fn from(c: Config) -> Figment {
        let profile = Config::selected_profile();
        let figment = Figment::default()
            .merge(Toml::file(Env::var_or("FOUNDRY_CONFIG", Config::FILE_NAME)).nested())
            .merge(Env::prefixed("DAPP_").global())
            .merge(Env::prefixed("FOUNDRY_").ignore(&["PROFILE"]).global())
            .select(profile.clone());

        // we try to merge remappings after we've merged all other providers, this prevents
        // redundant fs lookups to determine the default remappings that are eventually updated by
        // other providers, like the toml file
        let remappings = RemappingsProvider::new(&figment, c.libs.clone());
        Figment::from(c).merge(figment.merge(remappings)).select(profile)
    }
}

/// Parses a config profile
///
/// All `Profile` date is ignored by serde, however the `Config::to_string_pretty` includes it and
/// returns a toml table like
///
/// ```toml
/// #[default]
/// src = "..."
/// ```
/// This ignores the `#[default]` part in the toml
pub fn parse_with_profile<T: serde::de::DeserializeOwned>(
    s: &str,
) -> Result<Option<(Profile, T)>, toml::de::Error> {
    let val: Map<Profile, T> = toml::from_str(s)?;
    Ok(val.into_iter().next())
}

impl Provider for Config {
    fn metadata(&self) -> Metadata {
        Metadata::named("Foundry Config")
    }

    #[track_caller]
    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        Serialized::defaults(self).data()
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profile: Self::DEFAULT_PROFILE,
            src: "src".into(),
            test: "test".into(),
            out: "out".into(),
            libs: vec!["lib".into()],
            cache: true,
            evm_version: Default::default(),
            solc_version: None,
            auto_detect_solc: true,
            optimizer: false,
            optimizer_runs: 200,
            solc_settings: r#"{
  "*": {
    "": [
      "ast"
    ],
    "*": [
      "abi",
      "evm.bytecode",
      "evm.deployedBytecode",
      "evm.methodIdentifiers"
    ]
  }
}
"#
            .to_string(),
            eth_rpc_url: None,
            verbosity: 0,
            remappings: vec![],
            libraries: vec![],
            ignored_error_codes: vec![],
            __non_exhaustive: (),
        }
    }
}

/// A figment provider that checks if the remappings were previously set and if they're unset looks
/// up the fs via `Remapping::find_many`.
struct RemappingsProvider {
    lib_paths: Vec<PathBuf>,
    /// the root path used to turn an absolute `Remapping`, as we're getting it from
    /// `Remapping::find_many` into a relative one.
    root: PathBuf,
    /// This contains either:
    ///   - previously set remappings
    ///   - a `MissingField` error, which means previous provider didn't set the "remappings" field
    ///   - other error, like formatting
    remappings: Result<Vec<Remapping>, figment::Error>,
}

impl RemappingsProvider {
    fn new(figment: &Figment, libs: Vec<PathBuf>) -> Self {
        let remappings = figment.extract_inner::<Vec<Remapping>>("remappings");
        let lib_paths = figment.extract_inner::<Vec<PathBuf>>("libs").unwrap_or(libs);
        let root = Config::find_config_file()
            .and_then(|f| f.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap());
        RemappingsProvider { lib_paths, root, remappings }
    }
}

impl Provider for RemappingsProvider {
    fn metadata(&self) -> Metadata {
        Metadata::named("Remapping Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let remappings = match &self.remappings {
            Ok(remappings) => remappings.clone(),
            Err(err) => {
                if let figment::error::Kind::MissingField(_) = err.kind {
                    // only search for the remappings if weren't set before
                    self.lib_paths
                        .iter()
                        .map(|lib| self.root.join(lib))
                        .flat_map(Remapping::find_many)
                        .collect()
                } else {
                    return Err(err.clone())
                }
            }
        };

        // turn the absolute remapping into a relative one by stripping the `root`
        let remappings = remappings
            .into_iter()
            .map(|r| RelativeRemapping::new(r, &self.root).to_string())
            .collect::<Vec<_>>();

        Ok(Map::from([(
            Config::selected_profile(),
            Dict::from([("remappings".to_string(), figment::value::Value::from(remappings))]),
        )]))
    }

    fn profile(&self) -> Option<Profile> {
        Some(Config::selected_profile())
    }
}

/// The path part of the [`Remapping`] that knows the path of the file it was configured in, if any.
///
/// A [`Remapping`] is intended to be absolute, but paths in configuration files are often desired
/// to be relative to the configuration file itself. For example, a path of
/// `weird-erc20/=lib/weird-erc20/src/` configured in a file `/var/foundry.toml` might be desired to
/// resolve as a `weird-erc20/=/var/lib/weird-erc20/src/` remapping.
#[derive(Debug, Clone, PartialEq)]
pub struct RelativeRemappingPathBuf {
    parent: Option<PathBuf>,
    path: PathBuf,
}

impl RelativeRemappingPathBuf {
    /// Creates a new `RelativeRemappingPathBuf` that checks if the `path` is a child path of
    /// `parent`.
    pub fn with_root(parent: &Path, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if let Ok(path) = path.strip_prefix(parent) {
            Self { parent: Some(parent.to_path_buf()), path: path.to_path_buf() }
        } else if path.has_root() {
            Self { parent: None, path: path.to_path_buf() }
        } else {
            Self { parent: Some(parent.to_path_buf()), path: path.to_path_buf() }
        }
    }

    /// Returns the path as it was declared, without modification.
    pub fn original(&self) -> &Path {
        &self.path
    }

    /// Returns this path relative to the file it was delcared in, if any.
    /// Returns the original if this path was not declared in a file or if the
    /// path has a root.
    pub fn relative(&self) -> PathBuf {
        if self.original().has_root() {
            return self.original().into()
        }
        self.parent
            .as_ref()
            .map(|p| p.join(self.original()))
            .unwrap_or_else(|| self.original().into())
    }
}

impl<P: AsRef<Path>> From<P> for RelativeRemappingPathBuf {
    fn from(path: P) -> RelativeRemappingPathBuf {
        Self { parent: None, path: path.as_ref().to_path_buf() }
    }
}

impl Serialize for RelativeRemapping {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RelativeRemapping {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let remapping = String::deserialize(deserializer)?;
        let remapping = Remapping::from_str(&remapping).map_err(serde::de::Error::custom)?;
        Ok(RelativeRemapping { name: remapping.name, path: remapping.path.into() })
    }
}

/// A relative [`Remapping`] that's aware of the current location
///
/// See [`RelativeRemappingPathBuf`]
#[derive(Clone, Debug, PartialEq)]
pub struct RelativeRemapping {
    pub name: String,
    pub path: RelativeRemappingPathBuf,
}

impl RelativeRemapping {
    /// Creates a new `RelativeRemapping` starting prefixed with `root`
    pub fn new(remapping: Remapping, root: &Path) -> Self {
        Self {
            name: remapping.name,
            path: RelativeRemappingPathBuf::with_root(root, remapping.path),
        }
    }

    /// Converts this relative remapping into an absolute remapping
    ///
    /// This sets to root of the remapping to the given `root` path
    pub fn to_remapping(mut self, root: PathBuf) -> Remapping {
        self.path.parent = Some(root);
        self.into()
    }
}

// Remappings are printed as `prefix=target`
impl fmt::Display for RelativeRemapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.name, self.path.original().display())
    }
}

impl From<RelativeRemapping> for Remapping {
    fn from(r: RelativeRemapping) -> Self {
        Remapping { name: r.name, path: r.path.relative().to_string_lossy().to_string() }
    }
}

impl From<Remapping> for RelativeRemapping {
    fn from(r: Remapping) -> Self {
        Self { name: r.name, path: r.path.into() }
    }
}

/// A subset of the foundry `Config`
/// used to initialize a `foundry.toml` file
///
/// # Example
///
/// ```rust
/// use foundry_config::{Config, BasicConfig};
/// use serde::Deserialize;
///
/// let my_config = Config::figment().extract::<BasicConfig>();
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BasicConfig {
    #[serde(skip)]
    pub profile: Profile,
    /// path of the source contracts dir, like `src` or `contracts`
    pub src: PathBuf,
    /// path to where artifacts shut be written to
    pub out: PathBuf,
    /// all library folders to include, `lib`, `node_modules`
    pub libs: Vec<PathBuf>,
    /// `Remappings` to use for this repo
    pub remappings: Vec<RelativeRemapping>,
}

impl BasicConfig {
    /// Serialize the config as a String of TOML.
    ///
    /// This serializes to a table with the name of the profile
    pub fn to_string_pretty(&self) -> Result<String, toml::ser::Error> {
        let s = toml::to_string_pretty(self)?;
        Ok(format!(
            r#"[{}]
{}
# See more config options https://github.com/gakonst/foundry/tree/master/config"#,
            self.profile, s
        ))
    }
}

mod from_str_lowercase {
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: std::fmt::Display,
        S: Serializer,
    {
        serializer.collect_str(&value.to_string().to_lowercase())
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        String::deserialize(deserializer)?.to_lowercase().parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use figment::Figment;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_figment_is_default() {
        figment::Jail::expect_with(|_| {
            let mut default: Config = Config::figment().extract().unwrap();
            default.profile = Config::default().profile;
            assert_eq!(default, Config::default());
            Ok(())
        });
    }

    #[test]
    fn test_default_round_trip() {
        figment::Jail::expect_with(|_| {
            let original = Config::figment();
            let roundtrip = Figment::from(Config::from(&original));
            for figment in &[original, roundtrip] {
                let config = Config::from(figment);
                assert_eq!(config, Config::default());
            }
            Ok(())
        });
    }

    #[test]
    fn test_profile_env() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_PROFILE", "default");
            let figment = Config::figment();
            assert_eq!(figment.profile(), "default");

            jail.set_env("FOUNDRY_PROFILE", "hardhat");
            let figment: Figment = Config::hardhat().into();
            assert_eq!(figment.profile(), "hardhat");
            Ok(())
        });
    }

    #[test]
    fn test_toml_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "some-source"
                out = "some-out"
                cache = true
                eth_rpc_url = "https://example.com/"
                verbosity = 3
                remappings = ["ds-test=lib/ds-test/"]
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "some-source".into(),
                    out: "some-out".into(),
                    cache: true,
                    eth_rpc_url: Some("https://example.com/".to_string()),
                    remappings: vec![Remapping::from_str("ds-test=lib/ds-test/").unwrap().into()],
                    verbosity: 3,
                    ..Config::default()
                }
            );

            Ok(())
        });
    }

    #[test]
    fn test_precedence() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "mysrc"
                out = "myout"
                verbosity = 3
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "mysrc".into(),
                    out: "myout".into(),
                    verbosity: 3,
                    ..Config::default()
                }
            );

            jail.set_env("FOUNDRY_SRC", r#"other-src"#);
            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "other-src".into(),
                    out: "myout".into(),
                    verbosity: 3,
                    ..Config::default()
                }
            );

            jail.set_env("FOUNDRY_PROFILE", "foo");
            let val: Result<String, _> = Config::figment().extract_inner("profile");
            assert!(val.is_err());

            Ok(())
        });
    }

    #[test]
    fn test_extract_basic() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "mysrc"
                out = "myout"
                verbosity = 3
                evm_version = 'berlin'

                [other]
                src = "other-src"
            "#,
            )?;
            let loaded = Config::load();
            assert_eq!(loaded.evm_version, EvmVersion::Berlin);
            let base = loaded.into_basic();
            let default = Config::default();
            assert_eq!(
                base,
                BasicConfig {
                    profile: Config::DEFAULT_PROFILE,
                    src: "mysrc".into(),
                    out: "myout".into(),
                    libs: default.libs.clone(),
                    remappings: default.remappings.clone()
                }
            );
            jail.set_env("FOUNDRY_PROFILE", r#"other"#);
            let base = Config::figment().extract::<BasicConfig>().unwrap();
            assert_eq!(
                base,
                BasicConfig {
                    profile: Config::DEFAULT_PROFILE,
                    src: "other-src".into(),
                    out: "myout".into(),
                    libs: default.libs.clone(),
                    remappings: default.remappings
                }
            );
            Ok(())
        });
    }

    #[test]
    fn config_roundtrip() {
        figment::Jail::expect_with(|jail| {
            let default = Config::default();
            let basic = default.clone().into_basic();
            jail.create_file("foundry.toml", &basic.to_string_pretty().unwrap())?;

            let other = Config::load();
            assert_eq!(default, other);

            let other = other.into_basic();
            assert_eq!(basic, other);

            jail.create_file("foundry.toml", &default.to_string_pretty().unwrap())?;
            let other = Config::load();
            assert_eq!(default, other);

            // println!("{}", default.to_string_pretty().unwrap());
            Ok(())
        });
    }
}
