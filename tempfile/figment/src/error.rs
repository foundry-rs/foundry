//! Error values produces when extracting configurations.

use std::fmt::{self, Display};
use std::borrow::Cow;

use serde::{ser, de};

use crate::{Figment, Profile, Metadata, value::Tag};

/// A simple alias to `Result` with an error type of [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// An error that occured while producing data or extracting a configuration.
///
/// # Constructing Errors
///
/// An `Error` will generally be constructed indirectly via its implementations
/// of serde's [`de::Error`] and [`ser::Error`], that is, as a result of
/// serialization or deserialization errors. When implementing [`Provider`],
/// however, it may be necessary to construct an `Error` directly.
///
/// [`Provider`]: crate::Provider
///
/// Broadly, there are two ways to construct an `Error`:
///
///   * With an error message, as `Error` impls `From<String>` and `From<&str>`:
///
///     ```
///     use figment::Error;
///
///     Error::from(format!("{} is invalid", 1));
///
///     Error::from("whoops, something went wrong!");
///     ```
///
///   * With a [`Kind`], as `Error` impls `From<Kind>`:
///
///     ```
///     use figment::{error::{Error, Kind}, value::Value};
///
///     let value = Value::serialize(&100).unwrap();
///     if !value.as_str().is_some() {
///         let kind = Kind::InvalidType(value.to_actual(), "string".into());
///         let error = Error::from(kind);
///     }
///     ```
///
/// As always, `?` can be used to automatically convert into an `Error` using
/// the available `From` implementations:
///
/// ```
/// use std::fs::File;
///
/// fn try_read() -> Result<(), figment::Error> {
///     let x = File::open("/tmp/foo.boo").map_err(|e| e.to_string())?;
///     Ok(())
/// }
/// ```
///
/// # Display
///
/// By default, `Error` uses all of the available information about the error,
/// including the `Metadata`, `path`, and `profile` to display a message that
/// resembles the following, where `$` is `error.` for some `error: Error`:
///
/// ```text
/// $kind: `$metadata.interpolate($path)` in $($metadata.sources())*
/// ```
///
/// Concretely, such an error may look like:
///
/// ```text
/// invalid type: found sequence, expected u16: `staging.port` in TOML file Config.toml
/// ```
///
/// # Iterator
///
/// An `Error` may contain more than one error. To process all errors, iterate
/// over an `Error`:
///
/// ```rust
/// fn with_error(error: figment::Error) {
///     for error in error {
///         println!("error: {}", error);
///     }
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    /// The tag of the value that errored. We use this to lookup the `metadata`.
    tag: Tag,
    /// The profile that was selected when the error occured, if any.
    pub profile: Option<Profile>,
    /// The metadata for the provider of the value that errored, if known.
    pub metadata: Option<Metadata>,
    /// The path to the configuration key that errored, if known.
    pub path: Vec<String>,
    /// The error kind.
    pub kind: Kind,
    prev: Option<Box<Error>>,
}

/// An error kind, encapsulating serde's [`serde::de::Error`].
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A custom error message.
    Message(String),

    /// An invalid type: (actual, expected). See
    /// [`serde::de::Error::invalid_type()`].
    InvalidType(Actual, String),
    /// An invalid value: (actual, expected). See
    /// [`serde::de::Error::invalid_value()`].
    InvalidValue(Actual, String),
    /// Too many or too few items: (actual, expected). See
    /// [`serde::de::Error::invalid_length()`].
    InvalidLength(usize, String),

    /// A variant with an unrecognized name: (actual, expected). See
    /// [`serde::de::Error::unknown_variant()`].
    UnknownVariant(String, &'static [&'static str]),
    /// A field with an unrecognized name: (actual, expected). See
    /// [`serde::de::Error::unknown_field()`].
    UnknownField(String, &'static [&'static str]),
    /// A field was missing: (name). See [`serde::de::Error::missing_field()`].
    MissingField(Cow<'static, str>),
    /// A field appeared more than once: (name). See
    /// [`serde::de::Error::duplicate_field()`].
    DuplicateField(&'static str),

    /// The `isize` was not in range of any known sized signed integer.
    ISizeOutOfRange(isize),
    /// The `usize` was not in range of any known sized unsigned integer.
    USizeOutOfRange(usize),

    /// The serializer or deserializer does not support the `Actual` type.
    Unsupported(Actual),

    /// The type `.0` cannot be used for keys, need a `.1`.
    UnsupportedKey(Actual, Cow<'static, str>),
}

impl Error {
    pub(crate) fn prefixed(mut self, key: &str) -> Self {
        self.path.insert(0, key.into());
        self
    }

    pub(crate) fn retagged(mut self, tag: Tag) -> Self {
        if self.tag.is_default() {
            self.tag = tag;
        }

        self
    }

    pub(crate) fn resolved(mut self, config: &Figment) -> Self {
        let mut error = Some(&mut self);
        while let Some(e) = error {
            e.metadata = config.get_metadata(e.tag).cloned();
            e.profile = e.tag.profile()
                .or_else(|| Some(config.profile().clone()));

            error = e.prev.as_deref_mut();
        }

        self
    }
}

impl Error {
    /// Returns `true` if the error's kind is `MissingField`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::error::{Error, Kind};
    ///
    /// let error = Error::from(Kind::MissingField("path".into()));
    /// assert!(error.missing());
    /// ```
    pub fn missing(&self) -> bool {
        matches!(self.kind, Kind::MissingField(..))
    }

    /// Append the string `path` to the error's path.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Error;
    ///
    /// let error = Error::from("an error message").with_path("some_path");
    /// assert_eq!(error.path, vec!["some_path"]);
    ///
    /// let error = Error::from("an error message").with_path("some.path");
    /// assert_eq!(error.path, vec!["some", "path"]);
    /// ```
    pub fn with_path(mut self, path: &str) -> Self {
        let paths = path.split('.')
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());

        self.path.extend(paths);
        self
    }

    /// Prepends `self` to `error` and returns `error`.
    ///
    /// ```rust
    /// use figment::error::Error;
    ///
    /// let e1 = Error::from("1");
    /// let e2 = Error::from("2");
    /// let e3 = Error::from("3");
    ///
    /// let error = e1.chain(e2).chain(e3);
    /// assert_eq!(error.count(), 3);
    ///
    /// let unchained = error.into_iter()
    ///     .map(|e| e.to_string())
    ///     .collect::<Vec<_>>();
    /// assert_eq!(unchained, vec!["3", "2", "1"]);
    ///
    /// let e1 = Error::from("1");
    /// let e2 = Error::from("2");
    /// let e3 = Error::from("3");
    /// let error = e3.chain(e2).chain(e1);
    /// assert_eq!(error.count(), 3);
    ///
    /// let unchained = error.into_iter()
    ///     .map(|e| e.to_string())
    ///     .collect::<Vec<_>>();
    /// assert_eq!(unchained, vec!["1", "2", "3"]);
    /// ```
    pub fn chain(self, mut error: Error) -> Self {
        error.prev = Some(Box::new(self));
        error
    }

    /// Returns the number of errors represented by `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Figment, providers::{Format, Toml}};
    ///
    /// figment::Jail::expect_with(|jail| {
    ///     jail.create_file("Base.toml", r#"
    ///         # oh no, an unclosed array!
    ///         cat = [1
    ///     "#)?;
    ///
    ///     jail.create_file("Release.toml", r#"
    ///         # and now an unclosed string!?
    ///         cat = "
    ///     "#)?;
    ///
    ///     let figment = Figment::from(Toml::file("Base.toml"))
    ///         .merge(Toml::file("Release.toml"));
    ///
    ///     let error = figment.extract_inner::<String>("cat").unwrap_err();
    ///     assert_eq!(error.count(), 2);
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn count(&self) -> usize {
        1 + self.prev.as_ref().map_or(0, |e| e.count())
    }
}

/// An iterator over all errors in an [`Error`].
pub struct IntoIter(Option<Error>);

impl Iterator for IntoIter {
    type Item = Error;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(mut error) = self.0.take() {
            self.0 = error.prev.take().map(|e| *e);
            Some(error)
        } else {
            None
        }
    }
}

impl IntoIterator for Error {
    type Item = Error;
    type IntoIter = IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter(Some(self))
    }
}

/// A type that enumerates all of serde's types, used to indicate that a value
/// of the given type was received.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq)]
pub enum Actual {
    Bool(bool),
    Unsigned(u128),
    Signed(i128),
    Float(f64),
    Char(char),
    Str(String),
    Bytes(Vec<u8>),
    Unit,
    Option,
    NewtypeStruct,
    Seq,
    Map,
    Enum,
    UnitVariant,
    NewtypeVariant,
    TupleVariant,
    StructVariant,
    Other(String),
}

impl fmt::Display for Actual {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actual::Bool(v) => write!(f, "bool {}", v),
            Actual::Unsigned(v) => write!(f, "unsigned int `{}`", v),
            Actual::Signed(v) => write!(f, "signed int `{}`", v),
            Actual::Float(v) => write!(f, "float `{}`", v),
            Actual::Char(v) => write!(f, "char {:?}", v),
            Actual::Str(v) => write!(f, "string {:?}", v),
            Actual::Bytes(v) => write!(f, "bytes {:?}", v),
            Actual::Unit => write!(f, "unit"),
            Actual::Option => write!(f, "option"),
            Actual::NewtypeStruct => write!(f, "new-type struct"),
            Actual::Seq => write!(f, "sequence"),
            Actual::Map => write!(f, "map"),
            Actual::Enum => write!(f, "enum"),
            Actual::UnitVariant => write!(f, "unit variant"),
            Actual::NewtypeVariant => write!(f, "new-type variant"),
            Actual::TupleVariant => write!(f, "tuple variant"),
            Actual::StructVariant => write!(f, "struct variant"),
            Actual::Other(v) => v.fmt(f),
        }
    }
}

impl From<de::Unexpected<'_>> for Actual {
    fn from(value: de::Unexpected<'_>) -> Actual {
        match value {
            de::Unexpected::Bool(v) => Actual::Bool(v),
            de::Unexpected::Unsigned(v) => Actual::Unsigned(v as u128),
            de::Unexpected::Signed(v) => Actual::Signed(v as i128),
            de::Unexpected::Float(v) => Actual::Float(v),
            de::Unexpected::Char(v) => Actual::Char(v),
            de::Unexpected::Str(v) => Actual::Str(v.into()),
            de::Unexpected::Bytes(v) => Actual::Bytes(v.into()),
            de::Unexpected::Unit => Actual::Unit,
            de::Unexpected::Option => Actual::Option,
            de::Unexpected::NewtypeStruct => Actual::NewtypeStruct,
            de::Unexpected::Seq => Actual::Seq,
            de::Unexpected::Map => Actual::Map,
            de::Unexpected::Enum => Actual::Enum,
            de::Unexpected::UnitVariant => Actual::UnitVariant,
            de::Unexpected::NewtypeVariant => Actual::NewtypeVariant,
            de::Unexpected::TupleVariant => Actual::TupleVariant,
            de::Unexpected::StructVariant => Actual::StructVariant,
            de::Unexpected::Other(v) => Actual::Other(v.into())
        }
    }
}

impl de::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Kind::Message(msg.to_string()).into()
    }

    fn invalid_type(unexp: de::Unexpected, exp: &dyn de::Expected) -> Self {
        Kind::InvalidType(unexp.into(), exp.to_string()).into()
    }

    fn invalid_value(unexp: de::Unexpected, exp: &dyn de::Expected) -> Self {
        Kind::InvalidValue(unexp.into(), exp.to_string()).into()
    }

    fn invalid_length(len: usize, exp: &dyn de::Expected) -> Self {
        Kind::InvalidLength(len, exp.to_string()).into()
    }

    fn unknown_variant(variant: &str, expected: &'static [&'static str]) -> Self {
        Kind::UnknownVariant(variant.into(), expected).into()
    }

    fn unknown_field(field: &str, expected: &'static [&'static str]) -> Self {
        Kind::UnknownField(field.into(), expected).into()
    }

    fn missing_field(field: &'static str) -> Self {
        Kind::MissingField(field.into()).into()
    }

    fn duplicate_field(field: &'static str) -> Self {
        Kind::DuplicateField(field).into()
    }
}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Kind::Message(msg.to_string()).into()
    }
}

impl From<Kind> for Error {
    fn from(kind: Kind) -> Error {
        Error {
            tag: Tag::Default,
            path: vec![],
            profile: None,
            metadata: None,
            prev: None,
            kind,
        }
    }
}

impl From<&str> for Error {
    fn from(string: &str) -> Error {
        Kind::Message(string.into()).into()
    }
}

impl From<String> for Error {
    fn from(string: String) -> Error {
        Kind::Message(string).into()
    }
}

impl Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Kind::Message(msg) => f.write_str(&msg),
            Kind::InvalidType(v, exp) => {
                write!(f, "invalid type: found {}, expected {}", v, exp)
            }
            Kind::InvalidValue(v, exp) => {
                write!(f, "invalid value {}, expected {}", v, exp)
            },
            Kind::InvalidLength(v, exp) => {
                write!(f, "invalid length {}, expected {}", v, exp)
            },
            Kind::UnknownVariant(v, exp) => {
                write!(f, "unknown variant: found `{}`, expected `{}`", v, OneOf(exp))
            }
            Kind::UnknownField(v, exp) => {
                write!(f, "unknown field: found `{}`, expected `{}`", v, OneOf(exp))
            }
            Kind::MissingField(v) => {
                write!(f, "missing field `{}`", v)
            }
            Kind::DuplicateField(v) => {
                write!(f, "duplicate field `{}`", v)
            }
            Kind::ISizeOutOfRange(v) => {
                write!(f, "signed integer `{}` is out of range", v)
            }
            Kind::USizeOutOfRange(v) => {
                write!(f, "unsigned integer `{}` is out of range", v)
            }
            Kind::Unsupported(v) => {
                write!(f, "unsupported type `{}`", v)
            }
            Kind::UnsupportedKey(a, e) => {
                write!(f, "unsupported type `{}` for key: must be `{}`", a, e)
            }
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.kind.fmt(f)?;

        if let (Some(profile), Some(md)) = (&self.profile, &self.metadata) {
            if !self.path.is_empty() {
                let key = md.interpolate(profile, &self.path);
                write!(f, " for key {:?}", key)?;
            }
        }

        if let Some(md) = &self.metadata {
            if let Some(source) = &md.source {
                write!(f, " in {} {}", source, md.name)?;
            } else {
                write!(f, " in {}", md.name)?;
            }
        }

        if let Some(prev) = &self.prev {
            write!(f, "\n{}", prev)?;
        }

        Ok(())
    }
}

impl std::error::Error for Error {}

/// A structure that implements [`de::Expected`] signaling that one of the types
/// in the slice was expected.
pub struct OneOf(pub &'static [&'static str]);

impl fmt::Display for OneOf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0.len() {
            0 => write!(f, "none"),
            1 => write!(f, "`{}`", self.0[0]),
            2 => write!(f, "`{}` or `{}`", self.0[0], self.0[1]),
            _ => {
                write!(f, "one of ")?;
                for (i, alt) in self.0.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "`{}`", alt)?;
                }

                Ok(())
            }
        }
    }
}

impl de::Expected for OneOf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}
