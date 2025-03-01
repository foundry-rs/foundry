#![cfg_attr(nightly, feature(doc_cfg))]
#![deny(missing_docs)]

//! Semi-hierarchical configuration so con-free, it's unreal.
//!
//! ```rust
//! use serde::Deserialize;
//! use figment::{Figment, providers::{Format, Toml, Json, Env}};
//!
//! #[derive(Deserialize)]
//! struct Package {
//!     name: String,
//!     description: Option<String>,
//!     authors: Vec<String>,
//!     publish: Option<bool>,
//!     // ... and so on ...
//! }
//!
//! #[derive(Deserialize)]
//! struct Config {
//!     package: Package,
//!     rustc: Option<String>,
//!     rustdoc: Option<String>,
//!     // ... and so on ...
//! }
//!
//! # figment::Jail::expect_with(|jail| {
//! # jail.create_file("Cargo.toml", r#"
//! #   [package]
//! #   name = "test"
//! #   authors = ["bob"]
//! #   publish = false
//! # "#)?;
//! let config: Config = Figment::new()
//!     .merge(Toml::file("Cargo.toml"))
//!     .merge(Env::prefixed("CARGO_"))
//!     .merge(Env::raw().only(&["RUSTC", "RUSTDOC"]))
//!     .join(Json::file("Cargo.json"))
//!     .extract()?;
//! # Ok(())
//! # });
//! ```
//!
//! # Table of Contents
//!
//!   * [Overview](#overview) - A brief overview of the entire crate.
//!   * [Metadata](#metadata) - Figment's value metadata tracking.
//!   * [Extracting and Profiles](#extracting-and-profiles) - Semi-hierarchical
//!     "profiles", profile selection, nesting, and extraction.
//!   * [Crate Feature Flags](#crate-feature-flags) - Feature flags and what
//!     they enable.
//!   * [Available Providers](#available-providers) - Table of providers
//!     provided by this and other crates.
//!   * [For `Provider` Authors](#for-provider-authors) - Tips for writing
//!     [`Provider`]s.
//!   * [For Library Authors](#for-library-authors) - Brief guide for authors
//!     wishing to use Figment in their libraries or frameworks.
//!   * [For Application Authors](#for-application-authors) - Brief guide for
//!     authors of applications that use libraries that use Figment.
//!   * [For CLI Application Authors](#for-cli-application-authors) - Brief
//!     guide for authors of applications with a CLI and other configuration
//!     sources.
//!   * [Tips](#tips) - Things to remember when working with Figment.
//!   * [Type Index](#modules) - The real rustdocs.
//!
//! # Overview
//!
//! Figment is a library for declaring and combining configuration sources and
//! extracting typed values from the combined sources. It distinguishes itself
//! from other libraries with similar motives by seamlessly and comprehensively
//! tracking configuration value provenance, even in the face of myriad sources.
//! This means that error values and messages are precise and know exactly where
//! and how misconfiguration arose.
//!
//! There are two prevailing concepts:
//!
//!   * **Providers:** Types implementing the [`Provider`] trait, which
//!     implement a configuration source.
//!   * **Figments:** The [`Figment`] type, which combines providers via
//!     [`merge`](Figment::merge()) or [`join`](Figment::join) and allows
//!     typed [`extraction`](Figment::extract()). Figments are also providers
//!     themselves.
//!
//! Defining a configuration consists of constructing a `Figment` and merging or
//! joining any number of [`Provider`]s. Values for duplicate keys from a
//! _merged_ provider replace those from previous providers, while no
//! replacement occurs for _joined_ providers. Sources are read eagerly,
//! immediately upon merging and joining.
//!
//! The simplest useful figment has one provider. The figment below will use all
//! environment variables prefixed with `MY_APP_` as configuration values, after
//! removing the prefix:
//!
//! ```
//! use figment::{Figment, providers::Env};
//!
//! let figment = Figment::from(Env::prefixed("MY_APP_"));
//! ```
//!
//! Most figments will use more than one provider, merging and joining as
//! necessary. The figment below reads `App.toml`, environment variables
//! prefixed with `APP_` and fills any holes (but does not replace existing
//! values) with values from `App.json`:
//!
//! ```
//! use figment::{Figment, providers::{Format, Toml, Json, Env}};
//!
//! let figment = Figment::new()
//!     .merge(Toml::file("App.toml"))
//!     .merge(Env::prefixed("APP_"))
//!     .join(Json::file("App.json"));
//! ```
//!
//! Values can be [`extracted`](Figment::extract()) into any value that
//! implements [`Deserialize`](serde::Deserialize). The [`Jail`] type allows for
//! semi-sandboxed configuration testing. The example below showcases
//! extraction and testing:
//!
//! ```rust
//! use serde::Deserialize;
//! use figment::{Figment, providers::{Format, Toml, Json, Env}};
//!
//! #[derive(Debug, PartialEq, Deserialize)]
//! struct AppConfig {
//!     name: String,
//!     count: usize,
//!     authors: Vec<String>,
//! }
//!
//! figment::Jail::expect_with(|jail| {
//!     jail.create_file("App.toml", r#"
//!         name = "Just a TOML App!"
//!         count = 100
//!     "#)?;
//!
//!     jail.create_file("App.json", r#"
//!         {
//!             "name": "Just a JSON App",
//!             "authors": ["figment", "developers"]
//!         }
//!     "#)?;
//!
//!     jail.set_env("APP_COUNT", 250);
//!
//!     // Sources are read _eagerly_: sources are read as soon as they are
//!     // merged/joined into a figment.
//!     let figment = Figment::new()
//!         .merge(Toml::file("App.toml"))
//!         .merge(Env::prefixed("APP_"))
//!         .join(Json::file("App.json"));
//!
//!     let config: AppConfig = figment.extract()?;
//!     assert_eq!(config, AppConfig {
//!         name: "Just a TOML App!".into(),
//!         count: 250,
//!         authors: vec!["figment".into(), "developers".into()],
//!     });
//!
//!     Ok(())
//! });
//! ```
//!
//! # Metadata
//!
//! Figment takes _great_ care to propagate as much information as possible
//! about configuration sources. All values extracted from a figment are
//! [tagged](crate::value::Tag) with the originating [`Metadata`] and
//! [`Profile`]. The tag is preserved across merges, joins, and errors, which
//! also include the [`path`](Error::path) of the offending key. Precise
//! tracking allows for rich error messages as well as ["magic"] values like
//! [`RelativePathBuf`], which automatically creates a path relative to the
//! configuration file in which it was declared.
//!
//! A [`Metadata`] consists of:
//!
//!   * The name of the configuration source.
//!   * An ["interpolater"](Metadata::interpolate()) that takes a path to a key
//!     and converts it into a provider-native key.
//!   * A [`Source`] specifying where the value was sourced from.
//!   * A code source [`Location`] where the value's provider was added to a
//!   [`Figment`].
//!
//! Along with the information in an [`Error`], this means figment can produce
//! rich error values and messages:
//!
//! ```text
//! error: invalid type: found string "hi", expected u16
//!  --> key `debug.port` in TOML file App.toml
//! ```
//!
//! [`RelativePathBuf`]: value::magic::RelativePathBuf
//! ["magic"]: value::magic
//! [`Location`]: std::panic::Location
//!
//! # Extracting and Profiles
//!
//! Providers _always_ [produce](Provider::data()) [`Dict`](value::Dict)s nested
//! in [`Profile`]s. A profile is [`selected`](Figment::select()) when
//! extracting, and the dictionary corresponding to that profile is deserialized
//! into the requested type. If no profile is selected, the
//! [`Default`](Profile::Default) profile is used.
//!
//! There are two built-in profiles: the aforementioned default profile and the
//! [`Global`](Profile::Global) profile. As the name implies, the default
//! profile contains default values for all profiles. The global profile _also_
//! contains values that correspond to all profiles, but those values supersede
//! values of any other profile _except_ the global profile, even when another
//! source is merged.
//!
//! Some providers can be configured as `nested`, which allows top-level keys in
//! dictionaries produced by the source to be treated as profiles. The following
//! example showcases profiles and nesting:
//!
//! ```rust
//! use serde::Deserialize;
//! use figment::{Figment, providers::{Format, Toml, Json, Env}};
//!
//! #[derive(Debug, PartialEq, Deserialize)]
//! struct Config {
//!     name: String,
//! }
//!
//! impl Config {
//!     // Note the `nested` option on both `file` providers. This makes each
//!     // top-level dictionary act as a profile.
//!     fn figment() -> Figment {
//!         Figment::new()
//!             .merge(Toml::file("Base.toml").nested())
//!             .merge(Toml::file("App.toml").nested())
//!     }
//! }
//!
//! figment::Jail::expect_with(|jail| {
//!     jail.create_file("Base.toml", r#"
//!         [default]
//!         name = "Base-Default"
//!
//!         [debug]
//!         name = "Base-Debug"
//!     "#)?;
//!
//!     // The default profile is used...by default.
//!     let config: Config = Config::figment().extract()?;
//!     assert_eq!(config, Config { name: "Base-Default".into(), });
//!
//!     // A different profile can be selected with `select`.
//!     let config: Config = Config::figment().select("debug").extract()?;
//!     assert_eq!(config, Config { name: "Base-Debug".into(), });
//!
//!     // Selecting non-existent profiles is okay as long as we have defaults.
//!     let config: Config = Config::figment().select("undefined").extract()?;
//!     assert_eq!(config, Config { name: "Base-Default".into(), });
//!
//!     // Replace the previous `Base.toml`. This one has a `global` profile.
//!     jail.create_file("Base.toml", r#"
//!         [default]
//!         name = "Base-Default"
//!
//!         [debug]
//!         name = "Base-Debug"
//!
//!         [global]
//!         name = "Base-Global"
//!     "#)?;
//!
//!     // Global values override all profile values.
//!     let config_def: Config = Config::figment().extract()?;
//!     let config_deb: Config = Config::figment().select("debug").extract()?;
//!     assert_eq!(config_def, Config { name: "Base-Global".into(), });
//!     assert_eq!(config_deb, Config { name: "Base-Global".into(), });
//!
//!     // Merges from succeeding providers take precedence, even for globals.
//!     jail.create_file("App.toml", r#"
//!         [debug]
//!         name = "App-Debug"
//!
//!         [global]
//!         name = "App-Global"
//!     "#)?;
//!
//!     let config_def: Config = Config::figment().extract()?;
//!     let config_deb: Config = Config::figment().select("debug").extract()?;
//!     assert_eq!(config_def, Config { name: "App-Global".into(), });
//!     assert_eq!(config_deb, Config { name: "App-Global".into(), });
//!
//!     Ok(())
//! });
//! ```
//!
//! # Crate Feature Flags
//!
//! To help with compilation times, types, modules, and providers are gated by
//! features. They are:
//!
//! | feature | gated namespace             | description                               |
//! |---------|-----------------------------|-------------------------------------------|
//! | `test`  | [`Jail`]                    | Semi-sandboxed environment for testing.   |
//! | `env`   | [`providers::Env`]          | Environment variable [`Provider`].        |
//! | `toml`  | [`providers::Toml`]         | TOML file/string [`Provider`].            |
//! | `json`  | [`providers::Json`]         | JSON file/string [`Provider`].            |
//! | `yaml`  | [`providers::Yaml`]         | YAML file/string [`Provider`].            |
//! | `yaml`  | [`providers::YamlExtended`] | [YAML Extended] file/string [`Provider`]. |
//!
//! [YAML Extended]: providers::YamlExtended::from_str()
//!
//! # Available Providers
//!
//! In addition to the four gated providers above, figment provides the
//! following providers out-of-the-box:
//!
//! | provider                              | description                            |
//! |---------------------------------------|----------------------------------------|
//! | [`providers::Serialized`]             | Source from any [`Serialize`] type.    |
//! | [`(impl AsRef<str>, impl Serialize)`] | Global source from a `("key", value)`. |
//! | [`&T` _where_ `T: Provider`]          | Source from `T` as a reference.        |
//!
//! <small>
//!
//! Note: `key` in `(key, value)` is a _key path_, e.g. `"a"` or `"a.b.c"`,
//! where the latter indicates a nested value `c` in `b` in `a`.
//!
//! See [`Figment#extraction`] and [Data
//! (keyed)](providers::Serialized#provider-details) for key path details.
//!
//! </small>
//!
//! [`Serialize`]: serde::Serialize
//! [`(impl AsRef<str>, impl Serialize)`]: Provider#impl-Provider-for-(K%2C%20V)
//! [`&T` _where_ `T: Provider`]: Provider#impl-Provider-for-%26%27_%20T
//!
//! ### Third-Party Providers
//!
//! The following external libraries implement Figment providers:
//!
//!  - [`figment_file_provider_adapter`](https://crates.io/crates/figment_file_provider_adapter)
//!
//!    Wraps existing providers. For any key ending in `_FILE` (configurable),
//!    emits a key without the `_FILE` suffix with a value corresponding to the
//!    contents of the file whose path is the original key's value.
//!
//! # For Provider Authors
//!
//! The [`Provider`] trait documentation details extensively how to implement a
//! provider for Figment. For data format based providers, the [`Format`] trait
//! allows for even simpler implementations.
//!
//! [`Format`]: providers::Format
//!
//! # For Library Authors
//!
//! For libraries and frameworks that wish to expose customizable configuration,
//! we encourage the following structure:
//!
//! ```rust
//! use serde::{Serialize, Deserialize};
//!
//! use figment::{Figment, Provider, Error, Metadata, Profile};
//!
//! // The library's required configuration.
//! #[derive(Debug, Deserialize, Serialize)]
//! struct Config { /* the library's required/expected values */ }
//!
//! // The default configuration.
//! impl Default for Config {
//!     fn default() -> Self {
//!         Config { /* default values */ }
//!     }
//! }
//!
//! impl Config {
//!     // Allow the configuration to be extracted from any `Provider`.
//!     fn from<T: Provider>(provider: T) -> Result<Config, Error> {
//!         Figment::from(provider).extract()
//!     }
//!
//!     // Provide a default provider, a `Figment`.
//!     fn figment() -> Figment {
//!         use figment::providers::Env;
//!
//!         // In reality, whatever the library desires.
//!         Figment::from(Config::default()).merge(Env::prefixed("APP_"))
//!     }
//! }
//!
//! use figment::value::{Map, Dict};
//!
//! // Make `Config` a provider itself for composability.
//! impl Provider for Config {
//!     fn metadata(&self) -> Metadata {
//!         Metadata::named("Library Config")
//!     }
//!
//!     fn data(&self) -> Result<Map<Profile, Dict>, Error>  {
//!         figment::providers::Serialized::defaults(Config::default()).data()
//!     }
//!
//!     fn profile(&self) -> Option<Profile> {
//!         // Optionally, a profile that's selected by default.
//!         # None
//!     }
//! }
//! ```
//!
//! This structure has the following properties:
//!
//!   * The library provides a `Config` structure that clearly indicates which
//!     values the library requires.
//!   * Users can completely customize configuration via their own [`Provider`].
//!   * The library's `Config` is itself a [`Provider`] for composability.
//!   * The library provides a `Figment` which it will use as the default
//!     configuration provider.
//!
//! `Config::from(Config::figment())` can be used as the library default while
//! allowing complete customization of the configuration sources. Developers
//! building on the library can base their figments on `Config::default()`,
//! `Config::figment()`, both or neither.
//!
//! For frameworks, a top-level structure should expose the `Figment` that was
//! used to extract the `Config`, allowing other libraries making use of the
//! framework to also extract values from the same `Figment`:
//!
//! ```rust,no_run
//! use figment::{Figment, Provider, Error};
//! # struct Config;
//! # impl Config {
//! #     fn figment() -> Figment { panic!() }
//! #     fn from<T: Provider>(_: T) -> Result<Config, Error> { panic!() }
//! # }
//!
//! struct App {
//!     /// The configuration.
//!     pub config: Config,
//!     /// The figment used to extract the configuration.
//!     pub figment: Figment,
//! }
//!
//! impl App {
//!     pub fn new() -> Result<App, Error> {
//!         App::custom(Config::figment())
//!     }
//!
//!     pub fn custom<T: Provider>(provider: T) -> Result<App, Error> {
//!         let figment = Figment::from(provider);
//!         Ok(App { config: Config::from(&figment)?, figment })
//!     }
//! }
//! ```
//!
//! # For Application Authors
//!
//! As an application author, you'll need to make at least the following
//! decisions:
//!
//!   1. The sources you'll accept configuration from.
//!   2. The precedence you'll apply to each source.
//!   3. Whether you'll use profiles or not.
//!
//! For special sources, you may find yourself needing to implement a custom
//! [`Provider`]. As with libraries, you'll likely want to provide default
//! values where possible either by providing it to the figment or by using
//! [serde's defaults](https://serde.rs/attr-default.html). Then, it's simply a
//! matter of declaring a figment and extracting the configuration from it.
//!
//! A reasonable starting point might be:
//!
//! ```rust
//! use serde::{Serialize, Deserialize};
//! use figment::{Figment, providers::{Env, Format, Toml, Serialized}};
//!
//! #[derive(Deserialize, Serialize)]
//! struct Config {
//!     key: String,
//!     another: u32
//! }
//!
//! impl Default for Config {
//!     fn default() -> Config {
//!         Config {
//!             key: "default".into(),
//!             another: 100,
//!         }
//!     }
//! }
//!
//! Figment::from(Serialized::defaults(Config::default()))
//!     .merge(Toml::file("App.toml"))
//!     .merge(Env::prefixed("APP_"));
//! ```
//!
//! # For CLI Application Authors
//!
//! As an author of an application with a CLI, you may want to use Figment in
//! combination with a library like [`clap`] if:
//!
//!   * You want to read configuration from sources outside of the CLI.
//!   * You want flexibility in how configuration sources are combined.
//!   * You want great error messages irrespective of how the application is
//!     configured.
//!
//! [`clap`]: https://docs.rs/clap/latest/clap/
//!
//! If any of these conditions apply, Figment is a great choice.
//!
//! If you are already using a library like [`clap`], you'll likely have a
//! configuration structure defined:
//!
//! ```rust
//! use clap::Parser;
//!
//! #[derive(Parser, Debug)]
//! struct Config {
//!    /// Name of the person to greet.
//!    #[clap(short, long, value_parser)]
//!    name: String,
//!
//!    /// Number of times to greet
//!    #[clap(short, long, value_parser, default_value_t = 1)]
//!    count: u8,
//! }
//! ```
//!
//! To enable the structure to be combined with other Figment sources, derive
//! `Serialize` and `Deserialize` for the structure:
//!
//! ```diff
//! + use serde::{Serialize, Deserialize};
//!
//! - #[derive(Parser, Debug)]
//! + #[derive(Parser, Debug, Serialize, Deserialize)]
//! struct Config {
//! ```
//!
//! It can then be combined with other sources via the
//! [`Serialized`](providers::Serialized) provider:
//!
//! ```rust
//! use clap::Parser;
//! use figment::{Figment, providers::{Serialized, Toml, Env, Format}};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Parser, Debug, Serialize, Deserialize)]
//! struct Config {
//!     // ...
//! }
//!
//! # figment::Jail::try_with(|_| {
//! // Parse CLI arguments. Override CLI config values with those in
//! // `Config.toml` and `APP_`-prefixed environment variables.
//! let config: Config = Figment::new()
//!     .merge(Serialized::defaults(Config::parse()))
//!     .merge(Toml::file("Config.toml"))
//!     .merge(Env::prefixed("APP_"))
//!     .extract()?;
//! # Ok(())
//! # });
//! ```
//!
//! See [For Application Authors](#for-application-authors) for further, general
//! guidance on using Figment for application configuration.
//!
//! # Tips
//!
//! Some things to remember when working with Figment:
//!
//!   * Merging and joining are _eager_: sources are read immediately. It's
//!     useful to define a function that returns a `Figment`.
//!   * The [`util`] modules contains helpful serialize and deserialize
//!     implementations for defining `Config` structures.
//!   * The [`Format`] trait makes implementing data-format based [`Provider`]s
//!     straight-forward.
//!   * [`Magic`](value::magic) values can significantly reduce the need to
//!     inspect a `Figment` directly.
//!   * [`Jail`] makes testing configurations straight-forward and much less
//!     error-prone.
//!   * [`Error`] may contain more than one error: iterate over it to retrieve
//!     all errors.
//!   * Using `#[serde(flatten)]` [can break error attribution], so it's best to
//!     avoid using it when possible.
//!
//! [can break error attribution]:
//! https://github.com/SergioBenitez/Figment/issues/80#issuecomment-1701946622

pub mod value;
pub mod providers;
pub mod error;
pub mod util;
mod figment;
mod profile;
mod coalesce;
mod metadata;
mod provider;

#[cfg(any(test, feature = "test"))] mod jail;
#[cfg(any(test, feature = "test"))] pub use jail::Jail;

#[doc(inline)]
pub use error::{Error, Result};
pub use self::figment::Figment;
pub use profile::Profile;
pub use provider::*;
pub use metadata::*;
