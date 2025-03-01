use std::panic::Location;

use serde::de::Deserialize;

use crate::{Profile, Provider, Metadata};
use crate::error::{Kind, Result};
use crate::value::{Value, Map, Dict, Tag, ConfiguredValueDe, DefaultInterpreter, LossyInterpreter};
use crate::coalesce::{Coalescible, Order};

/// Combiner of [`Provider`]s for configuration value extraction.
///
/// # Overview
///
/// A `Figment` combines providers by merging or joining their provided data.
/// The combined value or a subset of the combined value can be extracted into
/// any type that implements [`Deserialize`]. Additionally, values can be nested
/// in _profiles_, and a profile can be selected via [`Figment::select()`] for
/// extraction; the profile to be extracted can be retrieved with
/// [`Figment::profile()`] and defaults to [`Profile::Default`]. The [top-level
/// docs](crate) contain a broad overview of these topics.
///
/// ## Conflict Resolution
///
/// Conflicts arising from two providers providing values for the same key are
/// resolved via one of four strategies: [`join`], [`adjoin`], [`merge`], and
/// [`admerge`]. In general, `join` and `adjoin` prefer existing values while
/// `merge` and `admerge` prefer later values. The `ad-` strategies additionally
/// concatenate conflicting arrays whereas the non-`ad-` strategies treat arrays
/// as non-composite values.
///
/// The table below summarizes these strategies and their behavior, with the
/// column label referring to the type of the value pointed to by the
/// conflicting keys:
///
/// | Strategy    | Dictionaries   | Arrays        | All Others    |
/// |-------------|----------------|---------------|---------------|
/// | [`join`]    | Union, Recurse | Keep Existing | Keep Existing |
/// | [`adjoin`]  | Union, Recurse | Concatenate   | Keep Existing |
/// | [`merge`]   | Union, Recurse | Use Incoming  | Use Incoming  |
/// | [`admerge`] | Union, Recurse | Concatenate   | Use Incoming  |
///
/// ### Description
///
/// If both keys point to a **dictionary**, the dictionaries are always unioned,
/// irrespective of the strategy, and conflict resolution proceeds recursively
/// with each key in the union.
///
/// If both keys point to an **array**:
///
///   * `join` uses the existing value
///   * `merge` uses the incoming value
///   * `adjoin` and `admerge` concatenate the arrays
///
/// If both keys point to a **non-composite** (`String`, `Num`, etc.) or values
/// of different kinds (i.e, **array** and **num**):
///
///   * `join` and `adjoin` use the existing value
///   * `merge` and `admerge` use the incoming value
///
/// [`join`]: Figment::join()
/// [`adjoin`]: Figment::adjoin()
/// [`merge`]: Figment::merge()
/// [`admerge`]: Figment::admerge()
///
/// For examples, refer to each strategy's documentation.
///
/// ## Extraction
///
/// The configuration or a subset thereof can be extracted from a `Figment` in
/// one of several ways:
///
///   * [`Figment::extract()`], which extracts the complete value into any `T:
///     Deserialize`.
///   * [`Figment::extract_inner()`], which extracts a subset of the value for a
///     given key path.
///   * [`Figment::find_value()`], which returns the raw, serialized [`Value`]
///     for a given key path.
///
/// A "key path" is a string of the form `a.b.c` (e.g, `item`, `item.fruits`,
/// etc.) where each component delimited by a `.` is a key for the dictionary of
/// the preceding key in the path, or the root dictionary if it is the first key
/// in the path. See [`Value::find()`] for examples.
///
/// ## Metadata
///
/// Every value collected by a `Figment` is accompanied by the metadata produced
/// by the value's provider. Additionally, [`Metadata::provide_location`] is set
/// by `from`, `merge` and `join` to the caller's location. `Metadata` can be
/// retrieved in one of several ways:
///
///   * [`Figment::metadata()`], which returns an iterator over all of the
///     metadata for all values.
///   * [`Figment::find_metadata()`], which returns the metadata for a value at
///     a given key path.
///   * [`Figment::get_metadata()`], which returns the metadata for a given
///     [`Tag`], itself retrieved via [`Tagged`] or [`Value::tag()`].
///
/// [`Tagged`]: crate::value::magic::Tagged
#[derive(Clone, Debug)]
pub struct Figment {
    pub(crate) profile: Profile,
    pub(crate) metadata: Map<Tag, Metadata>,
    pub(crate) value: Result<Map<Profile, Dict>>,
}

impl Figment {
    /// Creates a new `Figment` with the default profile selected and no
    /// providers.
    ///
    /// ```rust
    /// use figment::Figment;
    ///
    /// let figment = Figment::new();
    /// # assert_eq!(figment.profile(), "default");
    /// assert_eq!(figment.metadata().count(), 0);
    /// ```
    pub fn new() -> Self {
        Figment {
            metadata: Map::new(),
            profile: Profile::Default,
            value: Ok(Map::new()),
        }
    }

    /// Creates a new `Figment` with the default profile selected and an initial
    /// `provider`.
    ///
    /// ```rust
    /// use figment::Figment;
    /// use figment::providers::Env;
    ///
    /// let figment = Figment::from(Env::raw());
    /// # assert_eq!(figment.profile(), "default");
    /// assert_eq!(figment.metadata().count(), 1);
    /// ```
    #[track_caller]
    pub fn from<T: Provider>(provider: T) -> Self {
        Figment::new().merge(provider)
    }

    #[track_caller]
    fn provide<T: Provider>(mut self, provider: T, order: Order) -> Self {
        if let Some(map) = provider.__metadata_map() {
            self.metadata.extend(map);
        }

        if let Some(profile) = provider.profile() {
            self.profile = self.profile.coalesce(profile, order);
        }

        let mut metadata = provider.metadata();
        metadata.provide_location = Some(Location::caller());

        let tag = Tag::next();
        self.metadata.insert(tag, metadata);
        self.value = match (provider.data(), self.value) {
            (Ok(_), e@Err(_)) => e,
            (Err(e), Ok(_)) => Err(e.retagged(tag)),
            (Err(e), Err(prev)) => Err(e.retagged(tag).chain(prev)),
            (Ok(mut new), Ok(old)) => {
                new.iter_mut()
                    .map(|(p, map)| std::iter::repeat(p).zip(map.values_mut()))
                    .flatten()
                    .for_each(|(p, v)| v.map_tag(|t| *t = tag.for_profile(p)));

                Ok(old.coalesce(new, order))
            }
        };

        self
    }

    /// Joins `provider` into the current figment.
    /// See [conflict resolution](#conflict-resolution) for details.
    ///
    /// ```rust
    /// use figment::Figment;
    /// use figment::util::map;
    /// use figment::value::{Dict, Map};
    ///
    /// let figment = Figment::new()
    ///     .join(("string", "original"))
    ///     .join(("vec", vec!["item 1"]))
    ///     .join(("map", map!["string" => "inner original"]));
    ///
    /// let new_figment = Figment::new()
    ///     .join(("string", "replaced"))
    ///     .join(("vec", vec!["item 2"]))
    ///     .join(("map", map!["string" => "inner replaced", "new" => "value"]))
    ///     .join(("new", "value"));
    ///
    /// let figment = figment.join(new_figment); // **join**
    ///
    /// let string: String = figment.extract_inner("string").unwrap();
    /// assert_eq!(string, "original"); // existing value retained
    ///
    /// let vec: Vec<String> = figment.extract_inner("vec").unwrap();
    /// assert_eq!(vec, vec!["item 1"]); // existing value retained
    ///
    /// let map: Map<String, String> = figment.extract_inner("map").unwrap();
    /// assert_eq!(map, map! {
    ///     "string".into() => "inner original".into(), // existing value retained
    ///     "new".into() => "value".into(), // new key added
    /// });
    ///
    /// let new: String = figment.extract_inner("new").unwrap();
    /// assert_eq!(new, "value"); // new key added
    /// ```
    #[track_caller]
    pub fn join<T: Provider>(self, provider: T) -> Self {
        self.provide(provider, Order::Join)
    }

    /// Joins `provider` into the current figment while concatenating vectors.
    /// See [conflict resolution](#conflict-resolution) for details.
    ///
    /// ```rust
    /// use figment::Figment;
    /// use figment::util::map;
    /// use figment::value::{Dict, Map};
    ///
    /// let figment = Figment::new()
    ///     .join(("string", "original"))
    ///     .join(("vec", vec!["item 1"]))
    ///     .join(("map", map!["vec" => vec!["inner item 1"]]));
    ///
    /// let new_figment = Figment::new()
    ///     .join(("string", "replaced"))
    ///     .join(("vec", vec!["item 2"]))
    ///     .join(("map", map!["vec" => vec!["inner item 2"], "new" => vec!["value"]]))
    ///     .join(("new", "value"));
    ///
    /// let figment = figment.adjoin(new_figment); // **adjoin**
    ///
    /// let string: String = figment.extract_inner("string").unwrap();
    /// assert_eq!(string, "original"); // existing value retained
    ///
    /// let vec: Vec<String> = figment.extract_inner("vec").unwrap();
    /// assert_eq!(vec, vec!["item 1", "item 2"]); // arrays concatenated
    ///
    /// let map: Map<String, Vec<String>> = figment.extract_inner("map").unwrap();
    /// assert_eq!(map, map! {
    ///     "vec".into() => vec!["inner item 1".into(), "inner item 2".into()], // arrays concatenated
    ///     "new".into() => vec!["value".into()], // new key added
    /// });
    ///
    /// let new: String = figment.extract_inner("new").unwrap();
    /// assert_eq!(new, "value"); // new key added
    /// ```
    #[track_caller]
    pub fn adjoin<T: Provider>(self, provider: T) -> Self {
        self.provide(provider, Order::Adjoin)
    }

    /// Merges `provider` into the current figment.
    /// See [conflict resolution](#conflict-resolution) for details.
    ///
    /// ```rust
    /// use figment::Figment;
    /// use figment::util::map;
    /// use figment::value::{Dict, Map};
    ///
    /// let figment = Figment::new()
    ///     .join(("string", "original"))
    ///     .join(("vec", vec!["item 1"]))
    ///     .join(("map", map!["string" => "inner original"]));
    ///
    /// let new_figment = Figment::new()
    ///     .join(("string", "replaced"))
    ///     .join(("vec", vec!["item 2"]))
    ///     .join(("map", map!["string" => "inner replaced", "new" => "value"]))
    ///     .join(("new", "value"));
    ///
    /// let figment = figment.merge(new_figment); // **merge**
    ///
    /// let string: String = figment.extract_inner("string").unwrap();
    /// assert_eq!(string, "replaced"); // incoming value replaced existing
    ///
    /// let vec: Vec<String> = figment.extract_inner("vec").unwrap();
    /// assert_eq!(vec, vec!["item 2"]); // incoming value replaced existing
    ///
    /// let map: Map<String, String> = figment.extract_inner("map").unwrap();
    /// assert_eq!(map, map! {
    ///     "string".into() => "inner replaced".into(), // incoming value replaced existing
    ///     "new".into() => "value".into(), // new key added
    /// });
    ///
    /// let new: String = figment.extract_inner("new").unwrap();
    /// assert_eq!(new, "value"); // new key added
    /// ```
    #[track_caller]
    pub fn merge<T: Provider>(self, provider: T) -> Self {
        self.provide(provider, Order::Merge)
    }

    /// Merges `provider` into the current figment while concatenating vectors.
    /// See [conflict resolution](#conflict-resolution) for details.
    ///
    /// ```rust
    /// use figment::Figment;
    /// use figment::util::map;
    /// use figment::value::{Dict, Map};
    ///
    /// let figment = Figment::new()
    ///     .join(("string", "original"))
    ///     .join(("vec", vec!["item 1"]))
    ///     .join(("map", map!["vec" => vec!["inner item 1"]]));
    ///
    /// let new_figment = Figment::new()
    ///     .join(("string", "replaced"))
    ///     .join(("vec", vec!["item 2"]))
    ///     .join(("map", map!["vec" => vec!["inner item 2"], "new" => vec!["value"]]))
    ///     .join(("new", "value"));
    ///
    /// let figment = figment.admerge(new_figment); // **admerge**
    ///
    /// let string: String = figment.extract_inner("string").unwrap();
    /// assert_eq!(string, "replaced"); // incoming value replaced existing
    ///
    /// let vec: Vec<String> = figment.extract_inner("vec").unwrap();
    /// assert_eq!(vec, vec!["item 1", "item 2"]); // arrays concatenated
    ///
    /// let map: Map<String, Vec<String>> = figment.extract_inner("map").unwrap();
    /// assert_eq!(map, map! {
    ///     "vec".into() => vec!["inner item 1".into(), "inner item 2".into()], // arrays concatenated
    ///     "new".into() => vec!["value".into()], // new key added
    /// });
    ///
    /// let new: String = figment.extract_inner("new").unwrap();
    /// assert_eq!(new, "value"); // new key added
    /// ```
    #[track_caller]
    pub fn admerge<T: Provider>(self, provider: T) -> Self {
        self.provide(provider, Order::Admerge)
    }

    /// Sets the profile to extract from to `profile`.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::Figment;
    ///
    /// let figment = Figment::new().select("staging");
    /// assert_eq!(figment.profile(), "staging");
    /// ```
    pub fn select<P: Into<Profile>>(mut self, profile: P) -> Self {
        self.profile = profile.into();
        self
    }

    /// Merges the selected profile with the default and global profiles.
    fn merged(&self) -> Result<Value> {
        let mut map = self.value.clone().map_err(|e| e.resolved(self))?;
        let def = map.remove(&Profile::Default).unwrap_or_default();
        let global = map.remove(&Profile::Global).unwrap_or_default();

        let map = match map.remove(&self.profile) {
            Some(v) if self.profile.is_custom() => def.merge(v).merge(global),
            _ => def.merge(global)
        };

        Ok(Value::Dict(Tag::Default, map))
    }

    /// Returns a new `Figment` containing only the sub-dictionaries at `key`.
    ///
    /// This "sub-figment" is a _focusing_ of `self` with the property that:
    ///
    ///   * `self.find(key + ".sub")` <=> `focused.find("sub")`
    ///
    /// In other words, all values in `self` with a key starting with `key` are
    /// in `focused` _without_ the prefix and vice-versa.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, providers::{Format, Toml}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         cat = [1, 2, 3]
    ///         dog = [4, 5, 6]
    ///
    ///         [subtree]
    ///         cat = "meow"
    ///         dog = "woof!"
    ///
    ///         [subtree.bark]
    ///         dog = true
    ///         cat = false
    ///     "#)?;
    ///
    ///     let root = Figment::from(Toml::file("Config.toml"));
    ///     assert_eq!(root.extract_inner::<Vec<u8>>("cat").unwrap(), vec![1, 2, 3]);
    ///     assert_eq!(root.extract_inner::<Vec<u8>>("dog").unwrap(), vec![4, 5, 6]);
    ///     assert_eq!(root.extract_inner::<String>("subtree.cat").unwrap(), "meow");
    ///     assert_eq!(root.extract_inner::<String>("subtree.dog").unwrap(), "woof!");
    ///
    ///     let subtree = root.focus("subtree");
    ///     assert_eq!(subtree.extract_inner::<String>("cat").unwrap(), "meow");
    ///     assert_eq!(subtree.extract_inner::<String>("dog").unwrap(), "woof!");
    ///     assert_eq!(subtree.extract_inner::<bool>("bark.cat").unwrap(), false);
    ///     assert_eq!(subtree.extract_inner::<bool>("bark.dog").unwrap(), true);
    ///
    ///     let bark = subtree.focus("bark");
    ///     assert_eq!(bark.extract_inner::<bool>("cat").unwrap(), false);
    ///     assert_eq!(bark.extract_inner::<bool>("dog").unwrap(), true);
    ///
    ///     let not_a_dict = root.focus("cat");
    ///     assert!(not_a_dict.extract_inner::<bool>("cat").is_err());
    ///     assert!(not_a_dict.extract_inner::<bool>("dog").is_err());
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn focus(&self, key: &str) -> Self {
        fn try_focus(figment: &Figment, key: &str) -> Result<Map<Profile, Dict>> {
            let map = figment.value.clone().map_err(|e| e.resolved(figment))?;
            let new_map = map.into_iter()
                .filter_map(|(k, v)| {
                    let focused = Value::Dict(Tag::Default, v).find(key)?;
                    let dict = focused.into_dict()?;
                    Some((k, dict))
                })
                .collect();

            Ok(new_map)
        }

        Figment {
            profile: self.profile.clone(),
            metadata: self.metadata.clone(),
            value: try_focus(self, key)
        }
    }

    /// Deserializes the collected value into `T`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    ///
    /// use figment::{Figment, providers::{Format, Toml, Json, Env}};
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Config {
    ///     name: String,
    ///     numbers: Option<Vec<usize>>,
    ///     debug: bool,
    /// }
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         name = "test"
    ///         numbers = [1, 2, 3, 10]
    ///     "#)?;
    ///
    ///     jail.set_env("config_name", "env-test");
    ///
    ///     jail.create_file("Config.json", r#"
    ///         {
    ///             "name": "json-test",
    ///             "debug": true
    ///         }
    ///     "#)?;
    ///
    ///     let config: Config = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .merge(Env::prefixed("CONFIG_"))
    ///         .join(Json::file("Config.json"))
    ///         .extract()?;
    ///
    ///     assert_eq!(config, Config {
    ///         name: "env-test".into(),
    ///         numbers: vec![1, 2, 3, 10].into(),
    ///         debug: true
    ///     });
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn extract<'a, T: Deserialize<'a>>(&self) -> Result<T> {
        let value = self.merged()?;
        T::deserialize(ConfiguredValueDe::<'_, DefaultInterpreter>::from(self, &value))
    }

    /// As [`extract`](Figment::extract_lossy), but interpret numbers and
    /// booleans more flexibly.
    ///
    /// See [`Value::to_bool_lossy`] and [`Value::to_num_lossy`] for a full
    /// explanation of the imputs accepted.
    ///
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    ///
    /// use figment::{Figment, providers::{Format, Toml, Json, Env}};
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Config {
    ///     name: String,
    ///     numbers: Option<Vec<usize>>,
    ///     debug: bool,
    /// }
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         name = "test"
    ///         numbers = ["1", "2", "3", "10"]
    ///     "#)?;
    ///
    ///     jail.set_env("config_name", "env-test");
    ///
    ///     jail.create_file("Config.json", r#"
    ///         {
    ///             "name": "json-test",
    ///             "debug": "yes"
    ///         }
    ///     "#)?;
    ///
    ///     let config: Config = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .merge(Env::prefixed("CONFIG_"))
    ///         .join(Json::file("Config.json"))
    ///         .extract_lossy()?;
    ///
    ///     assert_eq!(config, Config {
    ///         name: "env-test".into(),
    ///         numbers: vec![1, 2, 3, 10].into(),
    ///         debug: true
    ///     });
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn extract_lossy<'a, T: Deserialize<'a>>(&self) -> Result<T> {
        let value = self.merged()?;
        T::deserialize(ConfiguredValueDe::<'_, LossyInterpreter>::from(self, &value))
    }

    /// Deserializes the value at the `key` path in the collected value into
    /// `T`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, providers::{Format, Toml, Json}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         numbers = [1, 2, 3, 10]
    ///     "#)?;
    ///
    ///     jail.create_file("Config.json", r#"{ "debug": true } "#)?;
    ///
    ///     let numbers: Vec<usize> = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .join(Json::file("Config.json"))
    ///         .extract_inner("numbers")?;
    ///
    ///     assert_eq!(numbers, vec![1, 2, 3, 10]);
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn extract_inner<'a, T: Deserialize<'a>>(&self, path: &str) -> Result<T> {
        let value = self.find_value(path)?;
        let de = ConfiguredValueDe::<'_, DefaultInterpreter>::from(self, &value);
        T::deserialize(de).map_err(|e| e.with_path(path))
    }

    /// As [`extract`](Figment::extract_lossy), but interpret numbers and
    /// booleans more flexibly.
    ///
    /// See [`Value::to_bool_lossy`] and [`Value::to_num_lossy`] for a full
    /// explanation of the imputs accepted.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, providers::{Format, Toml, Json}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         numbers = ["1", "2", "3", "10"]
    ///     "#)?;
    ///
    ///     jail.create_file("Config.json", r#"{ "debug": true } "#)?;
    ///
    ///     let numbers: Vec<usize> = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .join(Json::file("Config.json"))
    ///         .extract_inner_lossy("numbers")?;
    ///
    ///     assert_eq!(numbers, vec![1, 2, 3, 10]);
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn extract_inner_lossy<'a, T: Deserialize<'a>>(&self, path: &str) -> Result<T> {
        let value = self.find_value(path)?;
        let de = ConfiguredValueDe::<'_, LossyInterpreter>::from(self, &value);
        T::deserialize(de).map_err(|e| e.with_path(path))
    }

    /// Returns an iterator over the metadata for all of the collected values in
    /// the order in which they were added to `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, providers::{Format, Toml, Json}};
    ///
    /// let figment = Figment::new()
    ///     .merge(Toml::file("Config.toml"))
    ///     .join(Json::file("Config.json"));
    ///
    /// assert_eq!(figment.metadata().count(), 2);
    /// for (i, md) in figment.metadata().enumerate() {
    ///     match i {
    ///         0 => assert!(md.name.starts_with("TOML")),
    ///         1 => assert!(md.name.starts_with("JSON")),
    ///         _ => unreachable!(),
    ///     }
    /// }
    /// ```
    // In fact, the order in which they were added globally. Why? Because
    // `BTreeMap` returns values in order of keys, and we generate a new ID,
    // monotonically greater than the previous, each time a new item is
    // provided. It's important that the IDs are unique globally since we can
    // allow combining `Figment`s.
    pub fn metadata(&self) -> impl Iterator<Item = &Metadata> {
        self.metadata.values()
    }

    /// Returns the selected profile.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::Figment;
    ///
    /// let figment = Figment::new();
    /// assert_eq!(figment.profile(), "default");
    ///
    /// let figment = figment.select("staging");
    /// assert_eq!(figment.profile(), "staging");
    /// ```
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Returns an iterator over profiles with valid configurations in this
    /// figment. **Note:** this may not include the selected profile if the
    /// selected profile has no configured values.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::{Figment, providers::Serialized};
    ///
    /// let figment = Figment::new();
    /// let profiles = figment.profiles().collect::<Vec<_>>();
    /// assert_eq!(profiles.len(), 0);
    ///
    /// let figment = Figment::new()
    ///     .join(Serialized::default("key", "hi"))
    ///     .join(Serialized::default("key", "hey").profile("debug"));
    ///
    /// let mut profiles = figment.profiles().collect::<Vec<_>>();
    /// profiles.sort();
    /// assert_eq!(profiles, &["debug", "default"]);
    ///
    /// let figment = Figment::new()
    ///     .join(Serialized::default("key", "hi").profile("release"))
    ///     .join(Serialized::default("key", "hi").profile("testing"))
    ///     .join(Serialized::default("key", "hey").profile("staging"))
    ///     .select("debug");
    ///
    /// let mut profiles = figment.profiles().collect::<Vec<_>>();
    /// profiles.sort();
    /// assert_eq!(profiles, &["release", "staging", "testing"]);
    /// ```
    pub fn profiles(&self) -> impl Iterator<Item = &Profile> {
        self.value.as_ref()
            .ok()
            .map(|v| v.keys())
            .into_iter()
            .flatten()
    }

    /// Finds the value at `path` in the combined value.
    ///
    /// If there is an error evaluating the combined figment, that error is
    /// returned. Otherwise if there is a value at `path`, returns `Ok(value)`,
    /// and if there is no value at `path`, returns `Err` of kind
    /// `MissingField`.
    ///
    /// See [`Value::find()`] for details on the syntax for `path`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    ///
    /// use figment::{Figment, providers::{Format, Toml, Json, Env}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         name = "test"
    ///
    ///         [package]
    ///         name = "my-package"
    ///     "#)?;
    ///
    ///     jail.create_file("Config.json", r#"
    ///         {
    ///             "author": { "name": "Bob" }
    ///         }
    ///     "#)?;
    ///
    ///     let figment = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .join(Json::file("Config.json"));
    ///
    ///     let name = figment.find_value("name")?;
    ///     assert_eq!(name.as_str(), Some("test"));
    ///
    ///     let package_name = figment.find_value("package.name")?;
    ///     assert_eq!(package_name.as_str(), Some("my-package"));
    ///
    ///     let author_name = figment.find_value("author.name")?;
    ///     assert_eq!(author_name.as_str(), Some("Bob"));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn find_value(&self, path: &str) -> Result<Value> {
        self.merged()?
            .find(path)
            .ok_or_else(|| Kind::MissingField(path.to_string().into()).into())
    }

    /// Returns `true` if the combined figment evaluates successfully and
    /// contains a value at `path`.
    ///
    /// See [`Value::find()`] for details on the syntax for `path`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    ///
    /// use figment::{Figment, providers::{Format, Toml, Json, Env}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"
    ///         name = "test"
    ///
    ///         [package]
    ///         name = "my-package"
    ///     "#)?;
    ///
    ///     jail.create_file("Config.json", r#"
    ///         {
    ///             "author": { "name": "Bob" }
    ///         }
    ///     "#)?;
    ///
    ///     let figment = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .join(Json::file("Config.json"));
    ///
    ///     assert!(figment.contains("name"));
    ///     assert!(figment.contains("package"));
    ///     assert!(figment.contains("package.name"));
    ///     assert!(figment.contains("author"));
    ///     assert!(figment.contains("author.name"));
    ///     assert!(!figment.contains("author.title"));
    ///     Ok(())
    /// });
    /// ```
    pub fn contains(&self, path: &str) -> bool {
        self.merged().map_or(false, |v| v.find_ref(path).is_some())
    }

    /// Finds the metadata for the value at `key` path. See [`Value::find()`]
    /// for details on the syntax for `key`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    ///
    /// use figment::{Figment, providers::{Format, Toml, Json, Env}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#" name = "test" "#)?;
    ///     jail.set_env("CONF_AUTHOR", "Bob");
    ///
    ///     let figment = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .join(Env::prefixed("CONF_").only(&["author"]));
    ///
    ///     let name_md = figment.find_metadata("name").unwrap();
    ///     assert!(name_md.name.starts_with("TOML"));
    ///
    ///     let author_md = figment.find_metadata("author").unwrap();
    ///     assert!(author_md.name.contains("CONF_"));
    ///     assert!(author_md.name.contains("environment"));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn find_metadata(&self, key: &str) -> Option<&Metadata> {
        self.metadata.get(&self.find_value(key).ok()?.tag())
    }

    /// Returns the metadata with the given `tag` if this figment contains a
    /// value with said metadata.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    ///
    /// use figment::{Figment, providers::{Format, Toml, Json, Env}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#" name = "test" "#)?;
    ///     jail.create_file("Config.json", r#" { "author": "Bob" } "#)?;
    ///
    ///     let figment = Figment::new()
    ///         .merge(Toml::file("Config.toml"))
    ///         .join(Json::file("Config.json"));
    ///
    ///     let name = figment.find_value("name").unwrap();
    ///     let metadata = figment.get_metadata(name.tag()).unwrap();
    ///     assert!(metadata.name.starts_with("TOML"));
    ///
    ///     let author = figment.find_value("author").unwrap();
    ///     let metadata = figment.get_metadata(author.tag()).unwrap();
    ///     assert!(metadata.name.starts_with("JSON"));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn get_metadata(&self, tag: Tag) -> Option<&Metadata> {
        self.metadata.get(&tag)
    }
}

impl Provider for Figment {
    fn metadata(&self) -> Metadata { Metadata::default() }

    fn data(&self) -> Result<Map<Profile, Dict>> { self.value.clone() }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }

    fn __metadata_map(&self) -> Option<Map<Tag, Metadata>> {
        Some(self.metadata.clone())
    }
}

impl Default for Figment {
    fn default() -> Self {
        Figment::new()
    }
}

#[test]
#[cfg(test)]
fn is_send_sync() {
    fn check_for_send_sync<T: Send + Sync>() {}
    check_for_send_sync::<Figment>();
}
