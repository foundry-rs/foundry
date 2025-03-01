//! (De)serializable values that "magically" use information from the extracing
//! [`Figment`](crate::Figment).

use std::ops::Deref;
use std::path::{PathBuf, Path};

use serde::{Deserialize, Serialize, de};

use crate::{Error, value::{ConfiguredValueDe, Interpreter, MapDe, Tag}};

/// Marker trait for "magic" values. Primarily for use with [`Either`].
pub trait Magic: for<'de> Deserialize<'de> {
    /// The name of the deserialization pseudo-strucure.
    #[doc(hidden)] const NAME: &'static str;

    /// The fields of the pseudo-structure. The last one should be the value.
    #[doc(hidden)] const FIELDS: &'static [&'static str];

    #[doc(hidden)] fn deserialize_from<'de: 'c, 'c, V: de::Visitor<'de>, I: Interpreter>(
        de: ConfiguredValueDe<'c, I>,
        visitor: V
    ) -> Result<V::Value, Error>;
}

/// A [`PathBuf`] that knows the path of the file it was configured in, if any.
///
/// Paths in configuration files are often desired to be relative to the
/// configuration file itself. For example, a path of `a/b.html` configured in a
/// file `/var/config.toml` might be desired to resolve as `/var/a/b.html`. This
/// type makes this possible by simply delcaring the configuration value's type
/// as [`RelativePathBuf`].
///
/// # Example
///
/// ```rust
/// use std::path::Path;
///
/// use serde::{Deserialize, Serialize};
/// use figment::{Figment, value::magic::RelativePathBuf, Jail};
/// use figment::providers::{Env, Format, Toml, Serialized};
///
/// #[derive(Debug, PartialEq, Deserialize, Serialize)]
/// struct Config {
///     path: RelativePathBuf,
/// }
///
/// Jail::expect_with(|jail| {
///     // Note that `jail.directory()` is some non-empty path:
///     assert_ne!(jail.directory(), Path::new("/"));
///
///     // When a path is declared in a file and deserialized as
///     // `RelativePathBuf`, `relative()` will be relative to the file.
///     jail.create_file("Config.toml", r#"path = "a/b/c.html""#)?;
///     let c: Config = Figment::from(Toml::file("Config.toml")).extract()?;
///     assert_eq!(c.path.original(), Path::new("a/b/c.html"));
///     assert_eq!(c.path.relative(), jail.directory().join("a/b/c.html"));
///     assert_ne!(c.path.relative(), Path::new("a/b/c.html"));
///
///     // Round-tripping a `RelativePathBuf` preserves path-relativity.
///     let c: Config = Figment::from(Serialized::defaults(&c)).extract()?;
///     assert_eq!(c.path.original(), Path::new("a/b/c.html"));
///     assert_eq!(c.path.relative(), jail.directory().join("a/b/c.html"));
///     assert_ne!(c.path.relative(), Path::new("a/b/c.html"));
///
///     // If a path is declared elsewhere, the "relative" path is the original.
///     jail.set_env("PATH", "a/b/c.html");
///     let c: Config = Figment::from(Toml::file("Config.toml"))
///         .merge(Env::raw().only(&["PATH"]))
///         .extract()?;
///
///     assert_eq!(c.path.original(), Path::new("a/b/c.html"));
///     assert_eq!(c.path.relative(), Path::new("a/b/c.html"));
///
///     // Absolute paths remain unchanged.
///     jail.create_file("Config.toml", r#"path = "/var/c.html""#);
///     let c: Config = Figment::from(Toml::file("Config.toml")).extract()?;
///     assert_eq!(c.path.original(), Path::new("/var/c.html"));
///     assert_eq!(c.path.relative(), Path::new("/var/c.html"));
///
///     // You can use the `From<P: AsRef<Path>>` impl to set defaults:
///     let figment = Figment::from(Serialized::defaults(Config {
///         path: "some/default/path".into()
///     }));
///
///     let default: Config = figment.extract()?;
///     assert_eq!(default.path.original(), Path::new("some/default/path"));
///     assert_eq!(default.path.relative(), Path::new("some/default/path"));
///
///     jail.create_file("Config.toml", r#"path = "an/override""#)?;
///     let overriden: Config = figment.merge(Toml::file("Config.toml")).extract()?;
///     assert_eq!(overriden.path.original(), Path::new("an/override"));
///     assert_eq!(overriden.path.relative(), jail.directory().join("an/override"));
///
///     Ok(())
/// });
/// ```
///
/// # Serialization
///
/// By default, a `RelativePathBuf` serializes into a structure that can only
/// deserialize as a `RelativePathBuf`. In particular, a `RelativePathBuf` does
/// not serialize into a value compatible with `PathBuf`. To serialize into a
/// `Path`, use [`RelativePathBuf::serialize_original()`] or
/// [`RelativePathBuf::serialize_relative()`] together with serde's
/// `serialize_with` field attribute:
///
/// ```rust
/// use std::path::PathBuf;
///
/// use serde::{Deserialize, Serialize};
/// use figment::{Figment, value::magic::RelativePathBuf, Jail};
/// use figment::providers::{Format, Toml, Serialized};
///
/// #[derive(Deserialize, Serialize)]
/// struct Config {
///     relative: RelativePathBuf,
///     #[serde(serialize_with = "RelativePathBuf::serialize_original")]
///     root: RelativePathBuf,
///     #[serde(serialize_with = "RelativePathBuf::serialize_relative")]
///     temp: RelativePathBuf,
/// }
///
/// Jail::expect_with(|jail| {
///     jail.create_file("Config.toml", r#"
///         relative = "relative/path"
///         root = "root/path"
///         temp = "temp/path"
///     "#)?;
///
///     // Create a figment with a serialize `Config`.
///     let figment = Figment::from(Toml::file("Config.toml"));
///     let config = figment.extract::<Config>()?;
///     let figment = Figment::from(Serialized::defaults(config));
///
///     // This fails, as expected.
///     let relative = figment.extract_inner::<PathBuf>("relative");
///     assert!(relative.is_err());
///
///     // These succeed. This one uses the originally written path.
///     let root = figment.extract_inner::<PathBuf>("root")?;
///     assert_eq!(root, PathBuf::from("root/path"));
///
///     // This one the magic relative path.
///     let temp = figment.extract_inner::<PathBuf>("temp")?;
///     assert_eq!(temp, jail.directory().join("temp/path"));
///
///     Ok(())
/// })
/// ```
#[derive(Debug, Clone)]
// #[derive(Deserialize, Serialize)]
// #[serde(rename = "___figment_relative_path_buf")]
pub struct RelativePathBuf {
    // #[serde(rename = "___figment_relative_metadata_path")]
    metadata_path: Option<PathBuf>,
    // #[serde(rename = "___figment_relative_path")]
    path: PathBuf,
}

impl PartialEq for RelativePathBuf {
    fn eq(&self, other: &Self) -> bool {
        self.relative() == other.relative()
    }
}

impl<P: AsRef<Path>> From<P> for RelativePathBuf {
    fn from(path: P) -> RelativePathBuf {
        Self { metadata_path: None, path: path.as_ref().into() }
    }
}

impl Magic for RelativePathBuf {
    const NAME: &'static str = "___figment_relative_path_buf";

    const FIELDS: &'static [&'static str] = &[
        "___figment_relative_metadata_path",
        "___figment_relative_path"
    ];

    fn deserialize_from<'de: 'c, 'c, V: de::Visitor<'de>, I: Interpreter>(
        de: ConfiguredValueDe<'c, I>,
        visitor: V
    ) -> Result<V::Value, Error> {
        // If we have this struct with a non-empty metadata_path, use it.
        let config = de.config;
        if let Some(d) = de.value.as_dict() {
            if let Some(mpv) = d.get(Self::FIELDS[0]) {
                if mpv.to_empty().is_none() {
                    let map_de = MapDe::new(d, |v| ConfiguredValueDe::<I>::from(config, v));
                    return visitor.visit_map(map_de);
                }
            }
        }

        let metadata_path = config.get_metadata(de.value.tag())
            .and_then(|metadata| metadata.source.as_ref()
                .and_then(|s| s.file_path())
                .map(|path| path.display().to_string()));

        let mut map = crate::value::Map::new();
        if let Some(path) = metadata_path {
            map.insert(Self::FIELDS[0].into(), path.into());
        }

        // If we have this struct with no metadata_path, still use the value.
        let value = de.value.find_ref(Self::FIELDS[1]).unwrap_or(&de.value);
        map.insert(Self::FIELDS[1].into(), value.clone());
        visitor.visit_map(MapDe::new(&map, |v| ConfiguredValueDe::<I>::from(config, v)))
    }
}

impl RelativePathBuf {
    /// Returns the path as it was declared, without modification.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    ///
    /// use figment::{Figment, value::magic::RelativePathBuf, Jail};
    /// use figment::providers::{Format, Toml};
    ///
    /// #[derive(Debug, PartialEq, serde::Deserialize)]
    /// struct Config {
    ///     path: RelativePathBuf,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"path = "hello.html""#)?;
    ///     let c: Config = Figment::from(Toml::file("Config.toml")).extract()?;
    ///     assert_eq!(c.path.original(), Path::new("hello.html"));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn original(&self) -> &Path {
        &self.path
    }

    /// Returns this path resolved relative to the file it was declared in, if any.
    ///
    /// If the configured path was relative and it was configured from a file,
    /// this function returns that path prefixed with that file's parent
    /// directory. Otherwise it returns the original path. Where
    /// `config_file_path` is the location of the configuration file, this
    /// corresponds to:
    ///
    /// ```rust
    /// # use figment::{Figment, value::magic::RelativePathBuf, Jail};
    /// # use figment::providers::{Format, Toml};
    /// # use serde::Deserialize;
    /// #
    /// # #[derive(Debug, PartialEq, Deserialize)]
    /// # struct Config {
    /// #     path: RelativePathBuf,
    /// # }
    /// # Jail::expect_with(|jail| {
    /// # let config_file_path = jail.directory().join("Config.toml");
    /// # let config_file = jail.create_file("Config.toml", r#"path = "hello.html""#)?;
    /// # let config: Config = Figment::from(Toml::file("Config.toml")).extract()?;
    /// # let relative_path_buf = config.path;
    /// let relative = config_file_path
    ///     .parent()
    ///     .unwrap()
    ///     .join(relative_path_buf.original());
    /// # assert_eq!(relative_path_buf.relative(), relative);
    /// # Ok(())
    /// # });
    /// ```
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    ///
    /// use figment::{Figment, value::magic::RelativePathBuf, Jail};
    /// use figment::providers::{Env, Format, Toml};
    ///
    /// #[derive(Debug, PartialEq, serde::Deserialize)]
    /// struct Config {
    ///     path: RelativePathBuf,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"path = "hello.html""#)?;
    ///     let c: Config = Figment::from(Toml::file("Config.toml")).extract()?;
    ///     assert_eq!(c.path.relative(), jail.directory().join("hello.html"));
    ///
    ///     jail.set_env("PATH", r#"hello.html"#);
    ///     let c: Config = Figment::from(Env::raw().only(&["PATH"])).extract()?;
    ///     assert_eq!(c.path.relative(), Path::new("hello.html"));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn relative(&self) -> PathBuf {
        if self.original().has_root() {
            return self.original().into();
        }

        self.metadata_path()
            .and_then(|root| match root.is_dir() {
                true => Some(root),
                false => root.parent(),
            })
            .map(|root| root.join(self.original()))
            .unwrap_or_else(|| self.original().into())
    }

    /// Returns the path to the file this path was declared in, if any.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    ///
    /// use figment::{Figment, value::magic::RelativePathBuf, Jail};
    /// use figment::providers::{Env, Format, Toml};
    ///
    /// #[derive(Debug, PartialEq, serde::Deserialize)]
    /// struct Config {
    ///     path: RelativePathBuf,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.create_file("Config.toml", r#"path = "hello.html""#)?;
    ///     let c: Config = Figment::from(Toml::file("Config.toml")).extract()?;
    ///     assert_eq!(c.path.metadata_path().unwrap(), jail.directory().join("Config.toml"));
    ///
    ///     jail.set_env("PATH", r#"hello.html"#);
    ///     let c: Config = Figment::from(Env::raw().only(&["PATH"])).extract()?;
    ///     assert_eq!(c.path.metadata_path(), None);
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn metadata_path(&self) -> Option<&Path> {
        self.metadata_path.as_ref().map(|p| p.as_ref())
    }

    /// Serialize `self` as the [`original`](Self::original()) path.
    ///
    /// See [serialization](Self#serialization) for more.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::PathBuf;
    /// use figment::value::magic::RelativePathBuf;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct Config {
    ///     #[serde(serialize_with = "RelativePathBuf::serialize_original")]
    ///     path: RelativePathBuf,
    /// }
    /// ```
    pub fn serialize_original<S>(&self, ser: S) -> Result<S::Ok, S::Error>
        where S: serde::Serializer
    {
        self.original().serialize(ser)
    }

    /// Serialize `self` as the [`relative`](Self::relative()) path.
    ///
    /// See [serialization](Self#serialization) for more.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::PathBuf;
    /// use figment::value::magic::RelativePathBuf;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct Config {
    ///     #[serde(serialize_with = "RelativePathBuf::serialize_relative")]
    ///     path: RelativePathBuf,
    /// }
    /// ```
    // FIXME: Make this the default? We need a breaking change for this.
    pub fn serialize_relative<S>(&self, ser: S) -> Result<S::Ok, S::Error>
        where S: serde::Serializer
    {
        self.relative().serialize(ser)
    }
}

// /// MAGIC
// #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
// #[serde(rename = "___figment_selected_profile")]
// pub struct SelectedProfile {
//     profile: crate::Profile,
// }
//
// /// TODO: This doesn't work when it's in a map and the config doesn't contain a
// /// value for the corresponding field; we never get to call `deserialize` on the
// /// field's value. We can't fabricate this from no value. We either need to fake
// /// the field name, somehow, or just not have this.
// impl Magic for SelectedProfile {
//     const NAME: &'static str = "___figment_selected_profile";
//     const FIELDS: &'static [&'static str] = &["profile"];
//
//     fn deserialize_from<'de: 'c, 'c, V: de::Visitor<'de>>(
//         de: ConfiguredValueDe<'c>,
//         visitor: V
//     ) -> Result<V::Value, Error>{
//         let mut map = crate::value::Map::new();
//         map.insert(Self::FIELDS[0].into(), de.config.profile().to_string().into());
//         visitor.visit_map(MapDe::new(&map, |v| ConfiguredValueDe::<I>::from(de.config, v)))
//     }
// }
//
// impl Deref for SelectedProfile {
//     type Target = crate::Profile;
//
//     fn deref(&self) -> &Self::Target {
//         &self.profile
//     }
// }

/// (De)serializes as either a magic value `A` or any other deserializable value
/// `B`.
///
/// An `Either<A, B>` deserializes as either an `A` or `B`, whichever succeeds
/// first.
///
/// The usual `Either` implementation or an "untagged" enum does not allow its
/// internal values to provide hints to the deserializer. These hints are
/// required for magic values to work. By contrast, this `Either` _does_ provide
/// the appropriate hints.
///
/// # Example
///
/// ```
/// use serde::{Serialize, Deserialize};
/// use figment::{Figment, value::magic::{Either, RelativePathBuf, Tagged}};
///
/// #[derive(Debug, PartialEq, Deserialize, Serialize)]
/// struct Config {
///     int_or_str: Either<Tagged<usize>, String>,
///     path_or_bytes: Either<RelativePathBuf, Vec<u8>>,
/// }
///
/// fn figment<A: Serialize, B: Serialize>(a: A, b: B) -> Figment {
///     Figment::from(("int_or_str", a)).merge(("path_or_bytes", b))
/// }
///
/// let config: Config = figment(10, "/a/b").extract().unwrap();
/// assert_eq!(config.int_or_str, Either::Left(10.into()));
/// assert_eq!(config.path_or_bytes, Either::Left("/a/b".into()));
///
/// let config: Config = figment("hi", "c/d").extract().unwrap();
/// assert_eq!(config.int_or_str, Either::Right("hi".into()));
/// assert_eq!(config.path_or_bytes, Either::Left("c/d".into()));
///
/// let config: Config = figment(123, &[1, 2, 3]).extract().unwrap();
/// assert_eq!(config.int_or_str, Either::Left(123.into()));
/// assert_eq!(config.path_or_bytes, Either::Right(vec![1, 2, 3].into()));
///
/// let config: Config = figment("boo!", &[4, 5, 6]).extract().unwrap();
/// assert_eq!(config.int_or_str, Either::Right("boo!".into()));
/// assert_eq!(config.path_or_bytes, Either::Right(vec![4, 5, 6].into()));
///
/// let config: Config = Figment::from(figment::providers::Serialized::defaults(Config {
///     int_or_str: Either::Left(10.into()),
///     path_or_bytes: Either::Left("a/b/c".into()),
/// })).extract().unwrap();
///
/// assert_eq!(config.int_or_str, Either::Left(10.into()));
/// assert_eq!(config.path_or_bytes, Either::Left("a/b/c".into()));
///
/// let config: Config = Figment::from(figment::providers::Serialized::defaults(Config {
///     int_or_str: Either::Right("hi".into()),
///     path_or_bytes: Either::Right(vec![3, 7, 13]),
/// })).extract().unwrap();
///
/// assert_eq!(config.int_or_str, Either::Right("hi".into()));
/// assert_eq!(config.path_or_bytes, Either::Right(vec![3, 7, 13]));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
// #[serde(untagged)]
// #[derive(Serialize)]
pub enum Either<A, B> {
    /// The "left" variant.
    Left(A),
    /// The "right" variant.
    Right(B),
}

impl<'de: 'b, 'b, A, B> Deserialize<'de> for Either<A, B>
    where A: Magic, B: Deserialize<'b>
{
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
        where D: de::Deserializer<'de>
    {
        use crate::value::ValueVisitor;

        // FIXME: propogate the error properly
        let value = de.deserialize_struct(A::NAME, A::FIELDS, ValueVisitor)?;
        match A::deserialize(&value) {
            Ok(value) => Ok(Either::Left(value)),
            Err(a_err) => {
                let value = value.as_dict()
                    .and_then(|d| d.get(A::FIELDS[A::FIELDS.len() - 1]))
                    .unwrap_or(&value);

                match B::deserialize(value) {
                    Ok(value) => Ok(Either::Right(value)),
                    Err(b_err) => Err(de::Error::custom(format!("{}; {}", a_err, b_err)))
                }
            }
        }

        // use crate::error::Kind::*;
        // result.map_err(|e| match e.kind {
        //     InvalidType(Actual, String) => de::Error::invalid_type()
        //     InvalidValue(Actual, String),
        //     UnknownField(String, &'static [&'static str]),
        //     MissingField(Cow<'static, str>),
        //     DuplicateField(&'static str),
        //     InvalidLength(usize, String),
        //     UnknownVariant(String, &'static [&'static str]),
        //     kind => de::Error::custom(kind.to_string()),
        // })
    }
}

/// A wrapper around any value of type `T` and its [`Tag`].
///
/// ```rust
/// use figment::{Figment, value::magic::Tagged, Jail};
/// use figment::providers::{Format, Toml};
///
/// #[derive(Debug, PartialEq, serde::Deserialize)]
/// struct Config {
///     number: Tagged<usize>,
/// }
///
/// Jail::expect_with(|jail| {
///     jail.create_file("Config.toml", r#"number = 10"#)?;
///     let figment = Figment::from(Toml::file("Config.toml"));
///     let c: Config = figment.extract()?;
///     assert_eq!(*c.number, 10);
///
///     let tag = c.number.tag();
///     let metadata = figment.get_metadata(tag).expect("number has tag");
///
///     assert!(!tag.is_default());
///     assert_eq!(metadata.name, "TOML file");
///     Ok(())
/// });
/// ```
#[derive(Debug, Clone)]
// #[derive(Deserialize, Serialize)]
// #[serde(rename = "___figment_tagged_item")]
pub struct Tagged<T> {
    // #[serde(rename = "___figment_tagged_tag")]
    tag: Tag,
    // #[serde(rename = "___figment_tagged_value")]
    value: T,
}

impl<T: PartialEq> PartialEq for Tagged<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: for<'de> Deserialize<'de>> Magic for Tagged<T> {
    const NAME: &'static str = "___figment_tagged_item";
    const FIELDS: &'static [&'static str] = &[
        "___figment_tagged_tag" , "___figment_tagged_value"
    ];

    fn deserialize_from<'de: 'c, 'c, V: de::Visitor<'de>, I: Interpreter>(
        de: ConfiguredValueDe<'c, I>,
        visitor: V
    ) -> Result<V::Value, Error>{
        let config = de.config;
        let mut map = crate::value::Map::new();

        // If we have this struct with a non-default tag, use it.
        if let Some(dict) = de.value.as_dict() {
            if let Some(tagv) = dict.get(Self::FIELDS[0]) {
                if let Ok(false) = tagv.deserialize::<Tag>().map(|t| t.is_default()) {
                    return visitor.visit_map(MapDe::new(dict, |v| {
                        ConfiguredValueDe::<I>::from(config, v)
                    }));
                }
            }
        }

        // If we have this struct with default tag, use the value.
        let value = de.value.find_ref(Self::FIELDS[1]).unwrap_or(&de.value);
        map.insert(Self::FIELDS[0].into(), de.value.tag().into());
        map.insert(Self::FIELDS[1].into(), value.clone());
        visitor.visit_map(MapDe::new(&map, |v| ConfiguredValueDe::<I>::from(config, v)))
    }
}

impl<T> Tagged<T> {
    /// Returns the tag of the inner value if it is known. As long `self` is a
    /// leaf and was extracted from a [`Figment`](crate::Figment), the returned
    /// value is expected to be `Some`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, Profile, value::magic::Tagged};
    ///
    /// let figment = Figment::from(("key", "value"));
    /// let tagged = figment.extract_inner::<Tagged<String>>("key").unwrap();
    ///
    /// assert!(!tagged.tag().is_default());
    /// assert_eq!(tagged.tag().profile(), Some(Profile::Global));
    /// ```
    pub fn tag(&self) -> Tag {
        self.tag
    }

    /// Consumes `self` and returns the inner value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, value::magic::Tagged};
    ///
    /// let tagged = Figment::from(("key", "value"))
    ///     .extract_inner::<Tagged<String>>("key")
    ///     .unwrap();
    ///
    /// let value = tagged.into_inner();
    /// assert_eq!(value, "value");
    /// ```
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for Tagged<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> From<T> for Tagged<T> {
    fn from(value: T) -> Self {
        Tagged { tag: Tag::Default, value, }
    }
}

/// These were generated by serde's derive. We don't want to depend on the
/// 'derive' feature, so we simply expand it and copy the impls here.
mod _serde {
    use super::*;

    #[allow(unused_imports)]
    pub mod export {
        // These are re-reexports used by serde's codegen.
        pub use std::clone::Clone;
        pub use std::convert::{From, Into};
        pub use std::default::Default;
        pub use std::fmt::{self, Formatter};
        pub use std::marker::PhantomData;
        pub use std::option::Option::{self, None, Some};
        pub use std::result::Result::{self, Err, Ok};

        pub fn missing_field<'de, V, E>(field: &'static str) -> Result<V, E>
            where V: serde::de::Deserialize<'de>,
                  E: serde::de::Error,
        {
            struct MissingFieldDeserializer<E>(&'static str, PhantomData<E>);

            impl<'de, E> serde::de::Deserializer<'de> for MissingFieldDeserializer<E>
                where E: serde::de::Error
            {
                    type Error = E;

                    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, E>
                        where V: serde::de::Visitor<'de>,
                    {
                        Err(serde::de::Error::missing_field(self.0))
                    }

                    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, E>
                        where V: serde::de::Visitor<'de>,
                    {
                        visitor.visit_none()
                    }

                    serde::forward_to_deserialize_any! {
                        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str
                        string bytes byte_buf unit unit_struct newtype_struct seq tuple
                        tuple_struct map struct enum identifier ignored_any
                    }
                }

            let deserializer = MissingFieldDeserializer(field, PhantomData);
            serde::de::Deserialize::deserialize(deserializer)
        }
    }

    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;

        #[automatically_derived]
        impl<'de> _serde::Deserialize<'de> for RelativePathBuf {
            fn deserialize<__D>(__deserializer: __D) -> export::Result<Self, __D::Error>
            where
                __D: _serde::Deserializer<'de>,
            {
                #[allow(non_camel_case_types)]
                enum __Field {
                    __field0,
                    __field1,
                    __ignore,
                }
                struct __FieldVisitor;
                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                    type Value = __Field;
                    fn expecting(
                        &self,
                        __formatter: &mut export::Formatter,
                    ) -> export::fmt::Result {
                        export::Formatter::write_str(__formatter, "field identifier")
                    }
                    fn visit_u64<__E>(
                        self,
                        __value: u64,
                    ) -> export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            0u64 => export::Ok(__Field::__field0),
                            1u64 => export::Ok(__Field::__field1),
                            _ => export::Err(_serde::de::Error::invalid_value(
                                _serde::de::Unexpected::Unsigned(__value),
                                &"field index 0 <= i < 2",
                            )),
                        }
                    }
                    fn visit_str<__E>(
                        self,
                        __value: &str,
                    ) -> export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            "___figment_relative_metadata_path" => {
                                export::Ok(__Field::__field0)
                            }
                            "___figment_relative_path" => export::Ok(__Field::__field1),
                            _ => export::Ok(__Field::__ignore),
                        }
                    }
                    fn visit_bytes<__E>(
                        self,
                        __value: &[u8],
                    ) -> export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            b"___figment_relative_metadata_path" => {
                                export::Ok(__Field::__field0)
                            }
                            b"___figment_relative_path" => {
                                export::Ok(__Field::__field1)
                            }
                            _ => export::Ok(__Field::__ignore),
                        }
                    }
                }
                impl<'de> _serde::Deserialize<'de> for __Field {
                    #[inline]
                    fn deserialize<__D>(
                        __deserializer: __D,
                    ) -> export::Result<Self, __D::Error>
                    where
                        __D: _serde::Deserializer<'de>,
                    {
                        _serde::Deserializer::deserialize_identifier(
                            __deserializer,
                            __FieldVisitor,
                        )
                    }
                }
                struct __Visitor<'de> {
                    marker: export::PhantomData<RelativePathBuf>,
                    lifetime: export::PhantomData<&'de ()>,
                }
                impl<'de> _serde::de::Visitor<'de> for __Visitor<'de> {
                    type Value = RelativePathBuf;
                    fn expecting(
                        &self,
                        __formatter: &mut export::Formatter,
                    ) -> export::fmt::Result {
                        export::Formatter::write_str(
                            __formatter,
                            "struct RelativePathBuf",
                        )
                    }
                    #[inline]
                    fn visit_seq<__A>(
                        self,
                        mut __seq: __A,
                    ) -> export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::SeqAccess<'de>,
                    {
                        let __field0 = match match _serde::de::SeqAccess::next_element::<
                            Option<PathBuf>,
                        >(&mut __seq)
                        {
                            export::Ok(__val) => __val,
                            export::Err(__err) => {
                                return export::Err(__err);
                            }
                        } {
                            export::Some(__value) => __value,
                            export::None => {
                                return export::Err(_serde::de::Error::invalid_length(
                                    0usize,
                                    &"struct RelativePathBuf with 2 elements",
                                ));
                            }
                        };
                        let __field1 = match match _serde::de::SeqAccess::next_element::<PathBuf>(
                            &mut __seq,
                        ) {
                            export::Ok(__val) => __val,
                            export::Err(__err) => {
                                return export::Err(__err);
                            }
                        } {
                            export::Some(__value) => __value,
                            export::None => {
                                return export::Err(_serde::de::Error::invalid_length(
                                    1usize,
                                    &"struct RelativePathBuf with 2 elements",
                                ));
                            }
                        };
                        export::Ok(RelativePathBuf {
                            metadata_path: __field0,
                            path: __field1,
                        })
                    }
                    #[inline]
                    fn visit_map<__A>(
                        self,
                        mut __map: __A,
                    ) -> export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::MapAccess<'de>,
                    {
                        let mut __field0: export::Option<Option<PathBuf>> =
                            export::None;
                        let mut __field1: export::Option<PathBuf> =
                            export::None;
                        while let export::Some(__key) =
                            match _serde::de::MapAccess::next_key::<__Field>(&mut __map) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            }
                        {
                            match __key {
                                __Field::__field0 => {
                                    if export::Option::is_some(&__field0) {
                                        return export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "___figment_relative_metadata_path",
                                            ),
                                        );
                                    }
                                    __field0 = export::Some(
                                        match _serde::de::MapAccess::next_value::<Option<PathBuf>>(
                                            &mut __map,
                                        ) {
                                            export::Ok(__val) => __val,
                                            export::Err(__err) => {
                                                return export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                __Field::__field1 => {
                                    if export::Option::is_some(&__field1) {
                                        return export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "___figment_relative_path",
                                            ),
                                        );
                                    }
                                    __field1 = export::Some(
                                        match _serde::de::MapAccess::next_value::<PathBuf>(
                                            &mut __map,
                                        ) {
                                            export::Ok(__val) => __val,
                                            export::Err(__err) => {
                                                return export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                _ => {
                                    let _ = match _serde::de::MapAccess::next_value::<
                                        _serde::de::IgnoredAny,
                                    >(
                                        &mut __map
                                    ) {
                                        export::Ok(__val) => __val,
                                        export::Err(__err) => {
                                            return export::Err(__err);
                                        }
                                    };
                                }
                            }
                        }
                        let __field0 = match __field0 {
                            export::Some(__field0) => __field0,
                            export::None => match export::missing_field(
                                "___figment_relative_metadata_path",
                            ) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            },
                        };
                        let __field1 = match __field1 {
                            export::Some(__field1) => __field1,
                            export::None => match export::missing_field(
                                "___figment_relative_path",
                            ) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            },
                        };
                        export::Ok(RelativePathBuf {
                            metadata_path: __field0,
                            path: __field1,
                        })
                    }
                }
                const FIELDS: &[&str] = &[
                    "___figment_relative_metadata_path",
                    "___figment_relative_path",
                ];
                _serde::Deserializer::deserialize_struct(
                    __deserializer,
                    "___figment_relative_path_buf",
                    FIELDS,
                    __Visitor {
                        marker: export::PhantomData::<RelativePathBuf>,
                        lifetime: export::PhantomData,
                    },
                )
            }
        }
    };

    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for RelativePathBuf {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = match _serde::Serializer::serialize_struct(
                    __serializer,
                    "___figment_relative_path_buf",
                    false as usize + 1 + 1,
                ) {
                    export::Ok(__val) => __val,
                    export::Err(__err) => {
                        return export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "___figment_relative_metadata_path",
                    &self.metadata_path,
                ) {
                    export::Ok(__val) => __val,
                    export::Err(__err) => {
                        return export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "___figment_relative_path",
                    &self.path,
                ) {
                    export::Ok(__val) => __val,
                    export::Err(__err) => {
                        return export::Err(__err);
                    }
                };
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };

    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<A, B> _serde::Serialize for Either<A, B>
        where
            A: _serde::Serialize,
            B: _serde::Serialize,
        {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    Either::Left(ref __field0) => {
                        _serde::Serialize::serialize(__field0, __serializer)
                    }
                    Either::Right(ref __field0) => {
                        _serde::Serialize::serialize(__field0, __serializer)
                    }
                }
            }
        }
    };

    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<'de, T> _serde::Deserialize<'de> for Tagged<T>
        where
            T: _serde::Deserialize<'de>,
        {
            fn deserialize<__D>(__deserializer: __D) -> export::Result<Self, __D::Error>
            where
                __D: _serde::Deserializer<'de>,
            {
                #[allow(non_camel_case_types)]
                enum __Field {
                    __field0,
                    __field1,
                    __ignore,
                }
                struct __FieldVisitor;
                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                    type Value = __Field;
                    fn expecting(
                        &self,
                        __formatter: &mut export::Formatter,
                    ) -> export::fmt::Result {
                        export::Formatter::write_str(__formatter, "field identifier")
                    }
                    fn visit_u64<__E>(
                        self,
                        __value: u64,
                    ) -> export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            0u64 => export::Ok(__Field::__field0),
                            1u64 => export::Ok(__Field::__field1),
                            _ => export::Err(_serde::de::Error::invalid_value(
                                _serde::de::Unexpected::Unsigned(__value),
                                &"field index 0 <= i < 2",
                            )),
                        }
                    }
                    fn visit_str<__E>(
                        self,
                        __value: &str,
                    ) -> export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            "___figment_tagged_tag" => export::Ok(__Field::__field0),
                            "___figment_tagged_value" => export::Ok(__Field::__field1),
                            _ => export::Ok(__Field::__ignore),
                        }
                    }
                    fn visit_bytes<__E>(
                        self,
                        __value: &[u8],
                    ) -> export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            b"___figment_tagged_tag" => export::Ok(__Field::__field0),
                            b"___figment_tagged_value" => export::Ok(__Field::__field1),
                            _ => export::Ok(__Field::__ignore),
                        }
                    }
                }
                impl<'de> _serde::Deserialize<'de> for __Field {
                    #[inline]
                    fn deserialize<__D>(
                        __deserializer: __D,
                    ) -> export::Result<Self, __D::Error>
                    where
                        __D: _serde::Deserializer<'de>,
                    {
                        _serde::Deserializer::deserialize_identifier(
                            __deserializer,
                            __FieldVisitor,
                        )
                    }
                }
                struct __Visitor<'de, T>
                where
                    T: _serde::Deserialize<'de>,
                {
                    marker: export::PhantomData<Tagged<T>>,
                    lifetime: export::PhantomData<&'de ()>,
                }
                impl<'de, T> _serde::de::Visitor<'de> for __Visitor<'de, T>
                where
                    T: _serde::Deserialize<'de>,
                {
                    type Value = Tagged<T>;
                    fn expecting(
                        &self,
                        __formatter: &mut export::Formatter,
                    ) -> export::fmt::Result {
                        export::Formatter::write_str(__formatter, "struct Tagged")
                    }
                    #[inline]
                    fn visit_seq<__A>(
                        self,
                        mut __seq: __A,
                    ) -> export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::SeqAccess<'de>,
                    {
                        let __field0 = match match _serde::de::SeqAccess::next_element::<Tag>(
                            &mut __seq,
                        ) {
                            export::Ok(__val) => __val,
                            export::Err(__err) => {
                                return export::Err(__err);
                            }
                        } {
                            export::Some(__value) => __value,
                            export::None => {
                                return export::Err(_serde::de::Error::invalid_length(
                                    0usize,
                                    &"struct Tagged with 2 elements",
                                ));
                            }
                        };
                        let __field1 =
                            match match _serde::de::SeqAccess::next_element::<T>(&mut __seq) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            } {
                                export::Some(__value) => __value,
                                export::None => {
                                    return export::Err(
                                        _serde::de::Error::invalid_length(
                                            1usize,
                                            &"struct Tagged with 2 elements",
                                        ),
                                    );
                                }
                            };
                        export::Ok(Tagged {
                            tag: __field0,
                            value: __field1,
                        })
                    }
                    #[inline]
                    fn visit_map<__A>(
                        self,
                        mut __map: __A,
                    ) -> export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::MapAccess<'de>,
                    {
                        let mut __field0: export::Option<Tag> = export::None;
                        let mut __field1: export::Option<T> = export::None;
                        while let export::Some(__key) =
                            match _serde::de::MapAccess::next_key::<__Field>(&mut __map) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            }
                        {
                            match __key {
                                __Field::__field0 => {
                                    if export::Option::is_some(&__field0) {
                                        return export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "___figment_tagged_tag",
                                            ),
                                        );
                                    }
                                    __field0 = export::Some(
                                        match _serde::de::MapAccess::next_value::<Tag>(
                                            &mut __map,
                                        ) {
                                            export::Ok(__val) => __val,
                                            export::Err(__err) => {
                                                return export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                __Field::__field1 => {
                                    if export::Option::is_some(&__field1) {
                                        return export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "___figment_tagged_value",
                                            ),
                                        );
                                    }
                                    __field1 = export::Some(
                                        match _serde::de::MapAccess::next_value::<T>(&mut __map)
                                        {
                                            export::Ok(__val) => __val,
                                            export::Err(__err) => {
                                                return export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                _ => {
                                    let _ = match _serde::de::MapAccess::next_value::<
                                        _serde::de::IgnoredAny,
                                    >(
                                        &mut __map
                                    ) {
                                        export::Ok(__val) => __val,
                                        export::Err(__err) => {
                                            return export::Err(__err);
                                        }
                                    };
                                }
                            }
                        }
                        let __field0 = match __field0 {
                            export::Some(__field0) => __field0,
                            export::None => match export::missing_field(
                                "___figment_tagged_tag",
                            ) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            },
                        };
                        let __field1 = match __field1 {
                            export::Some(__field1) => __field1,
                            export::None => match export::missing_field(
                                "___figment_tagged_value",
                            ) {
                                export::Ok(__val) => __val,
                                export::Err(__err) => {
                                    return export::Err(__err);
                                }
                            },
                        };
                        export::Ok(Tagged {
                            tag: __field0,
                            value: __field1,
                        })
                    }
                }
                const FIELDS: &[&str] =
                    &["___figment_tagged_tag", "___figment_tagged_value"];
                _serde::Deserializer::deserialize_struct(
                    __deserializer,
                    "___figment_tagged_item",
                    FIELDS,
                    __Visitor {
                        marker: export::PhantomData::<Tagged<T>>,
                        lifetime: export::PhantomData,
                    },
                )
            }
        }
    };

    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<T> _serde::Serialize for Tagged<T>
        where
            T: _serde::Serialize,
        {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = match _serde::Serializer::serialize_struct(
                    __serializer,
                    "___figment_tagged_item",
                    false as usize + 1 + 1,
                ) {
                    export::Ok(__val) => __val,
                    export::Err(__err) => {
                        return export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "___figment_tagged_tag",
                    &self.tag,
                ) {
                    export::Ok(__val) => __val,
                    export::Err(__err) => {
                        return export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "___figment_tagged_value",
                    &self.value,
                ) {
                    export::Ok(__val) => __val,
                    export::Err(__err) => {
                        return export::Err(__err);
                    }
                };
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::Figment;

    #[test]
    fn test_relative_path_buf() {
        use super::RelativePathBuf;
        use crate::providers::{Format, Toml};
        use std::path::Path;

        crate::Jail::expect_with(|jail| {
            jail.set_env("foo", "bar");
            jail.create_file("Config.toml", r###"
                [debug]
                file_path = "hello.js"
                another = "whoops/hi/there"
                absolute = "/tmp/foo"
            "###)?;

            let path: RelativePathBuf = Figment::new()
                .merge(Toml::file("Config.toml").nested())
                .select("debug")
                .extract_inner("file_path")?;

            assert_eq!(path.original(), Path::new("hello.js"));
            assert_eq!(path.metadata_path().unwrap(), jail.directory().join("Config.toml"));
            assert_eq!(path.relative(), jail.directory().join("hello.js"));

            let path: RelativePathBuf = Figment::new()
                .merge(Toml::file("Config.toml").nested())
                .select("debug")
                .extract_inner("another")?;

            assert_eq!(path.original(), Path::new("whoops/hi/there"));
            assert_eq!(path.metadata_path().unwrap(), jail.directory().join("Config.toml"));
            assert_eq!(path.relative(), jail.directory().join("whoops/hi/there"));

            let path: RelativePathBuf = Figment::new()
                .merge(Toml::file("Config.toml").nested())
                .select("debug")
                .extract_inner("absolute")?;

            assert_eq!(path.original(), Path::new("/tmp/foo"));
            assert_eq!(path.metadata_path().unwrap(), jail.directory().join("Config.toml"));
            assert_eq!(path.relative(), Path::new("/tmp/foo"));

            jail.create_file("Config.toml", r###"
                [debug.inner.container]
                inside = "inside_path/a.html"
            "###)?;

            #[derive(serde::Deserialize)]
            struct Testing { inner: Container, }

            #[derive(serde::Deserialize)]
            struct Container { container: Inside, }

            #[derive(serde::Deserialize)]
            struct Inside { inside: RelativePathBuf, }

            let testing: Testing = Figment::new()
                .merge(Toml::file("Config.toml").nested())
                .select("debug")
                .extract()?;

            let path = testing.inner.container.inside;
            assert_eq!(path.original(), Path::new("inside_path/a.html"));
            assert_eq!(path.metadata_path().unwrap(), jail.directory().join("Config.toml"));
            assert_eq!(path.relative(), jail.directory().join("inside_path/a.html"));

            Ok(())
        })
    }

    // #[test]
    // fn test_selected_profile() {
    //     use super::SelectedProfile;
    //
    //     let profile: SelectedProfile = Figment::new().extract().unwrap();
    //     assert_eq!(&*profile, crate::Profile::default());
    //
    //     let profile: SelectedProfile = Figment::new().select("foo").extract().unwrap();
    //     assert_eq!(&*profile, "foo");
    //
    //     let profile: SelectedProfile = Figment::new().select("bar").extract().unwrap();
    //     assert_eq!(&*profile, "bar");
    //
    //     #[derive(serde::Deserialize)]
    //     struct Testing {
    //         #[serde(alias = "other")]
    //         profile: SelectedProfile,
    //         value: usize
    //     }
    //
    //     let testing: Testing = Figment::from(("value", 123))
    //         .merge(("other", "hi"))
    //         .select("with-value").extract().unwrap();
    //
    //     assert_eq!(&*testing.profile, "with-value");
    //     assert_eq!(testing.value, 123);
    // }

    // #[test]
    // fn test_selected_profile_kink() {
    //     use super::SelectedProfile;
    //
    //     #[derive(serde::Deserialize)]
    //     struct Base {
    //         profile: SelectedProfile,
    //     }
    //
    //     #[derive(serde::Deserialize)]
    //     struct Testing {
    //         base: Base,
    //         value: usize
    //     }
    //
    //     let testing: Testing = Figment::from(("value", 123)).extract().unwrap();
    //
    //     assert_eq!(&*testing.base.profile, "with-value");
    //     assert_eq!(testing.value, 123);
    // }

    #[test]
    fn test_tagged() {
        use super::Tagged;

        let val = Figment::from(("foo", "hello"))
            .extract_inner::<Tagged<String>>("foo")
            .expect("extraction");

        let first_tag = val.tag();
        assert_eq!(val.value, "hello");

        let val = Figment::from(("bar", "hi"))
            .extract_inner::<Tagged<String>>("bar")
            .expect("extraction");

        let second_tag = val.tag();
        assert_eq!(val.value, "hi");
        assert!(second_tag != first_tag);

        #[derive(serde::Deserialize)]
        struct TwoVals {
            foo: Tagged<String>,
            bar: Tagged<u16>,
        }

        let two = Figment::new()
            .merge(("foo", "hey"))
            .merge(("bar", 10))
            .extract::<TwoVals>()
            .expect("extraction");

        let tag3 = two.foo.tag();
        assert_eq!(two.foo.value, "hey");
        assert!(tag3 != second_tag);

        let tag4 = two.bar.tag();
        assert_eq!(two.bar.value, 10);
        assert!(tag4 != tag3);

        let val = Figment::new()
            .merge(("foo", "hey"))
            .merge(("bar", 10))
            .extract::<Tagged<TwoVals>>()
            .expect("extraction");

        assert!(val.tag().is_default());

        let tag5 = val.value.foo.tag();
        assert_eq!(val.value.foo.value, "hey");
        assert!(tag4 != tag5);

        let tag6 = val.value.bar.tag();
        assert_eq!(val.value.bar.value, 10);
        assert!(tag6 != tag5)
    }
}
