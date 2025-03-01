use std::fmt;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::panic::Location;

use crate::Profile;

/// Metadata about a configuration value: its source's name and location.
///
/// # Overview
///
/// Every [`Value`] produced by a [`Figment`] is [`Tag`]ed with `Metadata`
/// by its producing [`Provider`]. The metadata consists of:
///
///   * A name for the source, e.g. "TOML File".
///   * The [`Source`] itself, if it is known.
///   * A default or custom [interpolater](#interpolation).
///   * A source [`Location`] where a value's provider was added to the
///   containing figment, if it is known.
///
/// This information is used to produce insightful error messages as well as to
/// generate values like [`RelativePathBuf`] that know about their configuration
/// source.
///
/// [`Location`]: std::panic::Location
///
/// ## Errors
///
/// [`Error`]s produced by [`Figment`]s contain the `Metadata` for the value
/// that caused the error. The `Display` implementation for `Error` uses the
/// metadata's interpolater to display the path to the key for the value that
/// caused the error.
///
/// ## Interpolation
///
/// Interpolation takes a figment profile and key path (`a.b.c`) and turns it
/// into a source-native path. The default interpolater returns a figment key
/// path prefixed with the profile if the profile is custom:
///
/// ```text
/// ${profile}.${a}.${b}.${c}
/// ```
///
/// Providers are free to implement any interpolater for their metadata. For
/// example, the interpolater for [`Env`] uppercases each path key:
///
/// ```rust
/// use figment::Metadata;
///
/// let metadata = Metadata::named("environment variable(s)")
///     .interpolater(|profile, path| {
///         let keys: Vec<_> = path.iter()
///             .map(|k| k.to_ascii_uppercase())
///             .collect();
///
///         format!("{}", keys.join("."))
///     });
///
/// let profile = figment::Profile::Default;
/// let interpolated = metadata.interpolate(&profile, &["key", "path"]);
/// assert_eq!(interpolated, "KEY.PATH");
/// ```
///
/// [`Provider`]: crate::Provider
/// [`Error`]: crate::Error
/// [`Figment`]: crate::Figment
/// [`RelativePathBuf`]: crate::value::magic::RelativePathBuf
/// [`value`]: crate::value::Value
/// [`Tag`]: crate::value::Tag
/// [`Env`]: crate::providers::Env
#[derive(Debug, Clone)]
pub struct Metadata {
    /// The name of the configuration source for a given value.
    pub name: Cow<'static, str>,
    /// The source of the configuration value, if it is known.
    pub source: Option<Source>,
    /// The source location where this value's provider was added to the
    /// containing figment, if it is known.
    pub provide_location: Option<&'static Location<'static>>,
    interpolater: Box<dyn Interpolator>,
}

impl Metadata {
    /// Creates a new `Metadata` with the given `name` and `source`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Metadata;
    ///
    /// let metadata = Metadata::from("AWS Config Store", "path/to/value");
    /// assert_eq!(metadata.name, "AWS Config Store");
    /// assert_eq!(metadata.source.unwrap().custom(), Some("path/to/value"));
    /// ```
    #[inline(always)]
    pub fn from<N, S>(name: N, source: S) -> Self
        where N: Into<Cow<'static, str>>, S: Into<Source>
    {
        Metadata::named(name).source(source)
    }

    /// Creates a new `Metadata` with the given `name` and no source.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Metadata;
    ///
    /// let metadata = Metadata::named("AWS Config Store");
    /// assert_eq!(metadata.name, "AWS Config Store");
    /// assert!(metadata.source.is_none());
    /// ```
    #[inline]
    pub fn named<T: Into<Cow<'static, str>>>(name: T) -> Self {
        Metadata { name: name.into(), ..Metadata::default() }
    }

    /// Sets the `source` of `self` to `Some(source)`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Metadata;
    ///
    /// let metadata = Metadata::named("AWS Config Store").source("config/path");
    /// assert_eq!(metadata.name, "AWS Config Store");
    /// assert_eq!(metadata.source.unwrap().custom(), Some("config/path"));
    /// ```
    #[inline(always)]
    pub fn source<S: Into<Source>>(mut self, source: S) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Sets the `interpolater` of `self` to the function `f`. The interpolater
    /// can be invoked via [`Metadata::interpolate()`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Metadata;
    ///
    /// let metadata = Metadata::named("environment variable(s)")
    ///     .interpolater(|profile, path| {
    ///         let keys: Vec<_> = path.iter()
    ///             .map(|k| k.to_ascii_uppercase())
    ///             .collect();
    ///
    ///         format!("{}", keys.join("."))
    ///     });
    ///
    /// let profile = figment::Profile::Default;
    /// let interpolated = metadata.interpolate(&profile, &["key", "path"]);
    /// assert_eq!(interpolated, "KEY.PATH");
    /// ```
    #[inline(always)]
    pub fn interpolater<I: Clone + Send + Sync + 'static>(mut self, f: I) -> Self
        where I: Fn(&Profile, &[&str]) -> String
    {
        self.interpolater = Box::new(f);
        self
    }

    /// Runs the interpolater in `self` on `profile` and `keys`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Metadata, Profile};
    ///
    /// let url = "ftp://config.dev";
    /// let md = Metadata::named("Network").source(url)
    ///     .interpolater(move |profile, keys| match profile.is_custom() {
    ///         true => format!("{}/{}/{}", url, profile, keys.join("/")),
    ///         false => format!("{}/{}", url, keys.join("/")),
    ///     });
    ///
    /// let interpolated = md.interpolate(&Profile::Default, &["key", "path"]);
    /// assert_eq!(interpolated, "ftp://config.dev/key/path");
    ///
    /// let profile = Profile::new("static");
    /// let interpolated = md.interpolate(&profile, &["key", "path"]);
    /// assert_eq!(interpolated, "ftp://config.dev/static/key/path");
    /// ```
    pub fn interpolate<K: AsRef<str>>(&self, profile: &Profile, keys: &[K]) -> String {
        let keys: Vec<_> = keys.iter().map(|k| k.as_ref()).collect();
        (self.interpolater)(profile, &keys)
    }
}

impl PartialEq for Metadata {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.source == other.source
    }
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            name: "Default".into(),
            source: None,
            provide_location: None,
            interpolater: Box::new(default_interpolater),
        }
    }
}

/// The source for a configuration value.
///
/// The `Source` of a given value can be determined via that value's
/// [`Metadata.source`](Metadata#structfield.source) retrievable via the value's
/// [`Tag`] (via [`Value::tag()`] or via the magic value [`Tagged`]) and
/// [`Figment::get_metadata()`].
///
/// [`Tag`]: crate::value::Tag
/// [`Value::tag()`]: crate::value::Value::tag()
/// [`Tagged`]: crate::value::magic::Tagged
/// [`Figment::get_metadata()`]: crate::Figment::get_metadata()
#[non_exhaustive]
#[derive(PartialEq, Debug, Clone)]
pub enum Source {
    /// A file: the path to the file.
    File(PathBuf),
    /// Some programatic value: the source location.
    Code(&'static Location<'static>),
    /// A custom source all-together.
    Custom(String),
}

impl Source {
    /// Returns the path to the source file if `self.kind` is `Kind::File`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    /// use figment::Source;
    ///
    /// let source = Source::from(Path::new("a/b/c.txt"));
    /// assert_eq!(source.file_path(), Some(Path::new("a/b/c.txt")));
    /// ```
    pub fn file_path(&self) -> Option<&Path> {
        match self {
            Source::File(ref p) => Some(p),
            _ => None,
        }
    }

    /// Returns the location to the source code if `self` is `Source::Code`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::panic::Location;
    ///
    /// use figment::Source;
    ///
    /// let location = Location::caller();
    /// let source = Source::Code(location);
    /// assert_eq!(source.code_location(), Some(location));
    /// ```
    pub fn code_location(&self) -> Option<&'static Location<'static>> {
        match self {
            Source::Code(s) => Some(s),
            _ => None
        }
    }
    /// Returns the custom source location if `self` is `Source::Custom`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Source;
    ///
    /// let source = Source::Custom("ftp://foo".into());
    /// assert_eq!(source.custom(), Some("ftp://foo"));
    /// ```
    pub fn custom(&self) -> Option<&str> {
        match self {
            Source::Custom(ref c) => Some(c),
            _ => None,
        }
    }
}

/// Displays the source. Location and custom sources are displayed directly.
/// File paths are displayed relative to the current working directory if the
/// relative path is shorter than the complete path.
impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::File(p) => {
                use {std::env::current_dir, crate::util::diff_paths};

                match current_dir().ok().and_then(|cwd| diff_paths(p, &cwd)) {
                    Some(r) if r.iter().count() < p.iter().count() => r.display().fmt(f),
                    Some(_) | None => p.display().fmt(f)
                }
            }
            Source::Code(l) => l.fmt(f),
            Source::Custom(c) => c.fmt(f),
        }
    }
}

impl From<&Path> for Source {
    fn from(path: &Path) -> Source {
        Source::File(path.into())
    }
}

impl From<&'static Location<'static>> for Source {
    fn from(location: &'static Location<'static>) -> Source {
        Source::Code(location)
    }
}

impl From<&str> for Source {
    fn from(string: &str) -> Source {
        Source::Custom(string.into())
    }
}

impl From<String> for Source {
    fn from(string: String) -> Source {
        Source::Custom(string)
    }
}

crate::util::cloneable_fn_trait!(
    Interpolator: Fn(&Profile, &[&str]) -> String + Send + Sync + 'static
);

fn default_interpolater(profile: &Profile, keys: &[&str]) -> String {
    format!("{}.{}", profile, keys.join("."))
}
