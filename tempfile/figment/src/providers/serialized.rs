use std::panic::Location;

use serde::Serialize;

use crate::{Profile, Provider, Metadata};
use crate::error::{Error, Kind::InvalidType};
use crate::value::{Value, Map, Dict};

/// A `Provider` that sources values directly from a serialize type.
///
/// # Provider Details
///
///   * **Profile**
///
///     This provider does not set a profile.
///
///   * **Metadata**
///
///     This provider is named `T` (via [`std::any::type_name`]). The source
///     location is set to the call site of the constructor.
///
///   * **Data (Unkeyed)**
///
///     When data is not keyed, `T` is expected to serialize to a [`Dict`] and
///     is emitted directly as the value for the configured profile.
///
///   * **Data (Keyed)**
///
///     When keyed ([`Serialized::default()`], [`Serialized::global()`],
///     [`Serialized::key()`]), `T` can serialize to any [`Value`] and is
///     emitted as the value of the configured `key` key path. Nested
///     dictionaries are created for every path component delimited by `.` in
///     the `key` string, each dictionary mapping the path component to the
///     child, with the leaf mapping to the serialized `T`. For instance,
///     `a.b.c` results in `{ a: { b: { c: T }}}`.
#[derive(Debug, Clone)]
pub struct Serialized<T> {
    /// The value to be serialized and used as the provided data.
    pub value: T,
    /// The key path (`a.b.c`) to emit the value to or the root if `None`.
    pub key: Option<String>,
    /// The profile to emit the value to. Defaults to [`Profile::Default`].
    pub profile: Profile,
    loc: &'static Location<'static>,
}

impl<T> Serialized<T> {
    /// Constructs an (unkeyed) provider that emits `value`, which must
    /// serialize to a `dict`, to the `profile`.
    ///
    /// ```rust
    /// use serde::Deserialize;
    /// use figment::{Figment, Jail, providers::Serialized, util::map};
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Config {
    ///     numbers: Vec<usize>,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     let map = map!["numbers" => &[1, 2, 3]];
    ///
    ///     // This is also `Serialized::defaults(&map)`;
    ///     let figment = Figment::from(Serialized::from(&map, "default"));
    ///     let config: Config = figment.extract()?;
    ///     assert_eq!(config, Config { numbers: vec![1, 2, 3] });
    ///
    ///     // This is also `Serialized::defaults(&map).profile("debug")`;
    ///     let figment = Figment::from(Serialized::from(&map, "debug"));
    ///     let config: Config = figment.select("debug").extract()?;
    ///     assert_eq!(config, Config { numbers: vec![1, 2, 3] });
    ///
    ///     Ok(())
    /// });
    /// ```
    #[track_caller]
    pub fn from<P: Into<Profile>>(value: T, profile: P) -> Serialized<T> {
        Serialized {
            value,
            key: None,
            profile: profile.into(),
            loc: Location::caller()
        }
    }

    /// Emits `value`, which must serialize to a [`Dict`], to the `Default`
    /// profile.
    ///
    /// Equivalent to `Serialized::from(value, Profile::Default)`.
    ///
    /// See [`Serialized::from()`].
    #[track_caller]
    pub fn defaults(value: T) -> Serialized<T> {
        Self::from(value, Profile::Default)
    }

    /// Emits `value`, which must serialize to a [`Dict`], to the `Global`
    /// profile.
    ///
    /// Equivalent to `Serialized::from(value, Profile::Global)`.
    ///
    /// See [`Serialized::from()`].
    #[track_caller]
    pub fn globals(value: T) -> Serialized<T> {
        Self::from(value, Profile::Global)
    }

    /// Emits a nested dictionary to the `Default` profile keyed by `key`
    /// key path with the final key mapping to `value`.
    ///
    /// See [Data (keyed)](#provider-details) for key path details.
    ///
    /// Equivalent to `Serialized::from(value, Profile::Default).key(key)`.
    ///
    /// See [`Serialized::from()`] and [`Serialized::key()`].
    #[track_caller]
    pub fn default(key: &str, value: T) -> Serialized<T> {
        Self::from(value, Profile::Default).key(key)
    }

    /// Emits a nested dictionary to the `Global` profile keyed by `key` with
    /// the final key mapping to `value`.
    ///
    /// See [Data (keyed)](#provider-details) for key path details.
    ///
    /// Equivalent to `Serialized::from(value, Profile::Global).key(key)`.
    ///
    /// See [`Serialized::from()`] and [`Serialized::key()`].
    #[track_caller]
    pub fn global(key: &str, value: T) -> Serialized<T> {
        Self::from(value, Profile::Global).key(key)
    }

    /// Sets the profile to emit the serialized value to.
    ///
    /// ```rust
    /// use figment::{Figment, Jail, providers::Serialized};
    ///
    /// Jail::expect_with(|jail| {
    ///     // This is also `Serialized::defaults(&map)`;
    ///     let figment = Figment::new()
    ///         .join(Serialized::default("key", "hey").profile("debug"))
    ///         .join(Serialized::default("key", "hi"));
    ///
    ///     let value: String = figment.extract_inner("key")?;
    ///     assert_eq!(value, "hi");
    ///
    ///     let value: String = figment.select("debug").extract_inner("key")?;
    ///     assert_eq!(value, "hey");
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn profile<P: Into<Profile>>(mut self, profile: P) -> Self {
        self.profile = profile.into();
        self
    }

    /// Sets the key path to emit the serialized value to.
    ///
    /// See [Data (keyed)](#provider-details) for key path details.
    ///
    /// ```rust
    /// use figment::{Figment, Jail, providers::Serialized};
    ///
    /// Jail::expect_with(|jail| {
    ///     // This is also `Serialized::defaults(&map)`;
    ///     let figment = Figment::new()
    ///         .join(Serialized::default("key", "hey").key("other"))
    ///         .join(Serialized::default("key", "hi"));
    ///
    ///     let value: String = figment.extract_inner("key")?;
    ///     assert_eq!(value, "hi");
    ///
    ///     let value: String = figment.extract_inner("other")?;
    ///     assert_eq!(value, "hey");
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl<T: Serialize> Provider for Serialized<T> {
    fn metadata(&self) -> Metadata {
        Metadata::from(std::any::type_name::<T>(), self.loc)
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let value = Value::serialize(&self.value)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let dict = match &self.key {
            Some(key) => crate::util::nest(key, value).into_dict().ok_or(error)?,
            None => value.into_dict().ok_or(error)?,
        };

        Ok(self.profile.clone().collect(dict))
    }
}
