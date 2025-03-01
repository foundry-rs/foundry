use std::fmt;

use crate::{Profile, Provider, Metadata};
use crate::coalesce::Coalescible;
use crate::value::{Map, Dict};
use crate::error::Error;
use crate::util::nest;

use uncased::{Uncased, UncasedStr};

crate::util::cloneable_fn_trait!(
    FilterMap: for<'a> Fn(&'a UncasedStr) -> Option<Uncased<'a>> + 'static
);

/// A [`Provider`] that sources its values from environment variables.
///
/// All key-lookups and comparisons are case insensitive, facilitated by the
/// [`UncasedStr`] and [`Uncased`] types. By default, environment variable names
/// are lowercased before being emitted as [key paths] in the provided data, but
/// this default can be changed with [`Env::lowercase()`]. Environment variable
/// values can contain structured data, parsed as a [`Value`], with syntax
/// resembling TOML:
///
///   * [`Bool`]: `true`, `false` (e.g, `APP_VAR=true`)
///   * [`Num::F64`]: any float containing `.`: (e.g, `APP_VAR=1.2`, `APP_VAR=-0.002`)
///   * [`Num::USize`]: any unsigned integer (e.g, `APP_VAR=10`)
///   * [`Num::Isize`]: any negative integer (e.g, `APP_VAR=-10`)
///   * [`Array`]: delimited by `[]` (e.g, `APP_VAR=[true, 1.0, -1]`)
///   * [`Dict`]: in the form `{key=value}` (e.g, `APP_VAR={key="value",num=10}`)
///   * [`String`]: delimited by `"` (e.g, `APP_VAR=\"hi\"`)
///   * [`String`]: anything else (e.g, `APP_VAR=hi`, `APP_VAR=[hi`)
///
/// Additionally, keys and strings delimited with `"` can contain the following
/// escaped characters:
///
/// ```text
/// \b         - backspace       (U+0008)
/// \t         - tab             (U+0009)
/// \n         - linefeed        (U+000A)
/// \f         - form feed       (U+000C)
/// \r         - carriage return (U+000D)
/// \"         - quote           (U+0022)
/// \\         - backslash       (U+005C)
/// \uXXXX     - unicode         (U+XXXX)
/// \UXXXXXXXX - unicode         (U+XXXXXXXX)
/// ```
///
/// For example:
///
/// ```sh
/// APP_VAR=\"hello\\nthere\"  => (what in Rust is) "hello\nthere"
/// APP_VAR=\"hi\\u1234there\" => (what in Rust is) "hi\u{1234}there"
/// APP_VAR=\"abc\\td\\n\"     => (what in Rust is) "abc\td\n"
///
/// APP_VAR={\"key\\nkey\"=123}`)
/// APP_VAR={\"key.key\"=123}`)
/// ```
///
/// Undelimited strings, or strings with invalid escape sequences, are
/// interpreted exactly as written without any escaping.
///
/// [key paths]: crate::Figment#extraction
/// [`Value`]: crate::value::Value
/// [`Bool`]: crate::value::Value::Bool
/// [`Num::F64`]: crate::value::Num::F64
/// [`Num::USize`]: crate::value::Num::USize
/// [`Num::ISize`]: crate::value::Num::ISize
/// [`Array`]: crate::value::Value::Array
/// [`String`]: crate::value::Value::String
///
/// # Key Paths (nesting)
///
/// Because environment variables names are emitted as [key paths] in the
/// provided data, a nested dictionary is created for every component of the
/// name delimited by `.`, each a parent of the next, with the leaf mapping to
/// environment variable `Value`. For example, the environment variable
/// `a.b.c=3` creates the mapping `a -> b -> c -> 3` in the emitted data.
///
/// Environment variable names cannot typically contain the `.` character, but
/// another character can be used in its place by replacing that character in
/// the name with `.` with [`Env::map()`]. The [`Env::split()`] method is a
/// convenience method that does exactly this.
///
/// # Provider Details
///
///   * **Profile**
///
///     This provider does not set a profile.
///
///   * **Metadata**
///
///     This provider is named `environment variable(s)`. It does not specify a
///     [`Source`](crate::Source). Interpolation makes path parts uppercase and
///     delimited with a `.`.
///
///   * **Data**
///
///     The data emitted by this provider is single-level dictionary with the
///     keys and values returned by [`Env::iter()`], which reads from the
///     currently set environment variables and is customizable via the various
///     inherent methods. The dictionary is emitted to the profile
///     [`profile`](#structfield.profile), configurable via [`Env::profile()`].
#[derive(Clone)]
#[cfg_attr(nightly, doc(cfg(feature = "env")))]
pub struct Env {
    filter_map: Box<dyn FilterMap>,
    /// The profile config data will be emitted to. Defaults to
    /// [`Profile::Default`].
    pub profile: Profile,
    /// We use this to generate better metadata when available.
    prefix: Option<String>,
    /// We use this to generate better metadata when available.
    lowercase: bool,
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl Env {
    fn new() -> Self {
        Env {
            filter_map: Box::new(|key| Some(key.into())),
            profile: Profile::Default,
            prefix: None,
            lowercase: true,
        }
    }

    fn chain<F: Clone + 'static>(self, f: F) -> Self
        where F: for<'a> Fn(Option<Uncased<'a>>) -> Option<Uncased<'a>>
    {
        let filter_map = self.filter_map;
        Env {
            filter_map: Box::new(move |key| f(filter_map(key))),
            profile: self.profile,
            prefix: self.prefix,
            lowercase: true,
        }
    }

    /// Constructs and `Env` provider that does not filter or map any
    /// environment variables.
    ///
    /// ```rust
    /// use serde::Deserialize;
    /// use figment::{Figment, Jail, providers::Env};
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Config {
    ///     numbers: Vec<usize>,
    ///     app_bar: String,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("NUMBERS", "[1, 2, 3]");
    ///     jail.set_env("APP_BAR", "hi");
    ///
    ///     let config: Config = Figment::from(Env::raw()).extract()?;
    ///     assert_eq!(config, Config {
    ///         numbers: vec![1, 2, 3],
    ///         app_bar: "hi".into(),
    ///     });
    ///
    ///     Ok(())
    /// });
    /// ```
    #[inline(always)]
    pub fn raw() -> Self {
        Env::new()
    }

    /// Return an `Env` provider that filters environment variables to those
    /// with the prefix `prefix` and maps to one without the prefix.
    ///
    /// ```rust
    /// use serde::Deserialize;
    /// use figment::{Figment, Jail, providers::Env};
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Config {
    ///     foo: usize,
    ///     bar: String,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("APP_FOO", 100);
    ///     jail.set_env("APP_BAR", "hi");
    ///
    ///     let config: Config = Figment::from(Env::prefixed("APP_")).extract()?;
    ///     assert_eq!(config, Config { foo: 100, bar: "hi".into() });
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn prefixed(prefix: &str) -> Self {
        let owned_prefix = prefix.to_string();
        let mut env = Env::new()
            .filter_map(move |key| match key.starts_with(&owned_prefix) {
                true => Some(key[owned_prefix.len()..].into()),
                false => None
            });

        env.prefix = Some(prefix.into());
        env
    }

    /// Applys an additional filter to the keys of environment variables being
    /// considered.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("FOO_FOO", 100);
    ///     jail.set_env("BAR_BAR", "hi");
    ///     jail.set_env("foobar", "hi");
    ///
    ///     // We'll be left with `FOO_FOO=100` and `foobar=hi`.
    ///     let env = Env::raw().filter(|k| k.starts_with("foo"));
    ///     assert_eq!(env.iter().count(), 2);
    ///
    ///     // Filters chain, like iterator adapters. `FOO_FOO=100` remains.
    ///     let env = env.filter(|k| k.as_str().contains('_'));
    ///     assert_eq!(env.iter().count(), 1);
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn filter<F: Clone + 'static>(self, filter: F) -> Self
        where F: Fn(&UncasedStr) -> bool
    {
        self.chain(move |prev| prev.filter(|v| filter(&v)))
    }

    /// Applys an additional mapping to the keys of environment variables being
    /// considered.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("FOO_FOO", 100);
    ///     jail.set_env("BAR_FOO", "hi");
    ///     jail.set_env("foobar", "hi");
    ///
    ///     // This is like `prefixed("foo_")` without the filtering.
    ///     let env = Env::raw().map(|k| match k.starts_with("foo_") {
    ///         true => k["foo_".len()..].into(),
    ///         false => k.into()
    ///     });
    ///
    ///     // We now have `FOO=100`, `BAR_FOO=hi`, and `bar=hi`.
    ///     assert_eq!(env.clone().filter(|k| k == "foo").iter().count(), 1);
    ///
    ///     // Mappings chain, like iterator adapters.
    ///     let env = env.map(|k| match k.starts_with("bar_") {
    ///         true => k["bar_".len()..].into(),
    ///         false => k.into()
    ///     });
    ///
    ///     // We now have `FOO=100`, `FOO=hi`, and `bar=hi`.
    ///     assert_eq!(env.filter(|k| k == "foo").iter().count(), 2);
    ///     Ok(())
    /// });
    /// ```
    pub fn map<F: Clone + 'static>(self, mapper: F) -> Self
        where F: Fn(&UncasedStr) -> Uncased<'_>
    {
        self.chain(move |prev| prev.map(|v| mapper(&v).into_owned()))
    }

    /// Simultanously filters and maps the keys of environment variables being
    /// considered.
    ///
    /// The returned `Env` only yields values for which `f` returns `Some`.
    ///
    /// ```rust
    /// use std::collections::HashMap;
    /// use figment::{Jail, providers::Env};
    /// use uncased::AsUncased;
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.clear_env();
    ///     jail.set_env("FOO_FOO", 100);
    ///     jail.set_env("BAR_BAR", "hi");
    ///     jail.set_env("BAZ_BAZ", "200");
    ///
    ///     // We starts with all three variables in `Env::raw();
    ///     let env = Env::raw();
    ///     assert_eq!(env.iter().count(), 3);
    ///
    ///     // This is like `prefixed("foo_")` but with two prefixes.
    ///     let env = env.filter_map(|k| {
    ///         if k.starts_with("foo_") {
    ///             Some(k["foo_".len()..].into())
    ///         } else if k.starts_with("baz_") {
    ///             Some(k["baz_".len()..].into())
    ///         } else {
    ///             None
    ///         }
    ///     });
    ///
    ///     // Now we have `FOO=100`, `BAZ="200"`.
    ///     let values = env.iter().collect::<HashMap<_, _>>();
    ///     assert_eq!(values.len(), 2);
    ///     assert_eq!(values["foo".as_uncased()], "100");
    ///     assert_eq!(values["baz".as_uncased()], "200");
    ///     Ok(())
    /// });
    /// ```
    pub fn filter_map<F: Clone + 'static>(self, f: F) -> Self
        where F: Fn(&UncasedStr) -> Option<Uncased<'_>>
    {
        self.chain(move |prev| prev.and_then(|v| f(&v).map(|v| v.into_owned())))
    }

    /// Whether to lowercase keys before emitting them. Defaults to `true`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::collections::HashMap;
    ///
    /// use figment::{Jail, Profile, Provider};
    /// use figment::providers::Env;
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.clear_env();
    ///     jail.set_env("FOO_BAR_BAZ", 1);
    ///     jail.set_env("FOO_barBaz", 2);
    ///
    ///     // The default is to lower-case variable name keys.
    ///     let env = Env::prefixed("FOO_");
    ///     let data = env.data().unwrap();
    ///     assert!(data[&Profile::Default].contains_key("bar_baz"));
    ///     assert!(data[&Profile::Default].contains_key("barbaz"));
    ///
    ///     // This can be changed with `lowercase(false)`. You'll need to
    ///     // arrange for deserialization to account for casing.
    ///     let env = Env::prefixed("FOO_").lowercase(false);
    ///     let data = env.data().unwrap();
    ///     assert!(data[&Profile::Default].contains_key("BAR_BAZ"));
    ///     assert!(data[&Profile::Default].contains_key("barBaz"));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn lowercase(mut self, lowercase: bool) -> Self {
        self.lowercase = lowercase;
        self
    }

    /// Splits each environment variable key at `pattern`, creating nested
    /// dictionaries for each split. Specifically, nested dictionaries are
    /// created for components delimited by `pattern` in the environment
    /// variable string (3 in `A_B_C` if `pattern` is `_`), each dictionary
    /// mapping to its parent.
    ///
    /// This is equivalent to: `self.map(|key| key.replace(pattern, "."))`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde::Deserialize;
    /// use figment::{Figment, Jail, util::map, value::Dict, providers::Env};
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Foo {
    ///     key: usize,
    /// }
    ///
    /// #[derive(Debug, PartialEq, Deserialize)]
    /// struct Config {
    ///     foo: Foo,
    ///     map: Dict,
    /// }
    ///
    /// Jail::expect_with(|jail| {
    ///     // Without splitting: using structured data.
    ///     jail.set_env("APP_FOO", "{key=10}");
    ///     jail.set_env("APP_MAP", "{one=1,two=2.0}");
    ///
    ///     let config: Config = Figment::from(Env::prefixed("APP_")).extract()?;
    ///     assert_eq!(config, Config {
    ///         foo: Foo { key: 10 },
    ///         map: map!["one".into() => 1u8.into(), "two".into() => 2.0.into()],
    ///     });
    ///
    ///     // With splitting.
    ///     jail.set_env("APP_FOO_KEY", 20);
    ///     jail.set_env("APP_MAP_ONE", "1.0");
    ///     jail.set_env("APP_MAP_TWO", "dos");
    ///
    ///     let config: Config = Figment::new()
    ///         .merge(Env::prefixed("APP_").split("_"))
    ///         .extract()?;
    ///
    ///     assert_eq!(config, Config {
    ///         foo: Foo { key: 20 },
    ///         map: map!["one".into() => 1.0.into(), "two".into() => "dos".into()],
    ///     });
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn split<P: Into<String>>(self, pattern: P) -> Self {
        let pattern = pattern.into();
        self.map(move |key| key.as_str().replace(&pattern, ".").into())
    }

    /// Filters out all environment variable keys contained in `keys`.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("FOO_FOO", 1);
    ///     jail.set_env("FOO_BAR", 2);
    ///     jail.set_env("FOO_BAZ", 3);
    ///     jail.set_env("FOO_BAM", 4);
    ///
    ///     let env = Env::prefixed("FOO_").ignore(&["bar", "baz"]);
    ///     assert_eq!(env.clone().iter().count(), 2);
    ///
    ///     // Ignores chain.
    ///     let env = env.ignore(&["bam"]);
    ///     assert_eq!(env.iter().count(), 1);
    ///     Ok(())
    /// });
    /// ```
    pub fn ignore(self, keys: &[&str]) -> Self {
        let keys: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        self.filter(move |key| !keys.iter().any(|k| k.as_str() == key))
    }

    /// Filters out all environment variables keys _not_ contained in `keys`.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("FOO_FOO", 1);
    ///     jail.set_env("FOO_BAR", 2);
    ///     jail.set_env("FOO_BAZ_BOB", 3);
    ///     jail.set_env("FOO_BAM_BOP", 4);
    ///
    ///     let env = Env::prefixed("FOO_").only(&["bar", "baz_bob", "zoo"]);
    ///     assert_eq!(env.iter().count(), 2);
    ///
    ///     jail.set_env("FOO_ZOO", 5);
    ///     assert_eq!(env.iter().count(), 3);
    ///
    ///     let env = Env::prefixed("FOO_").split("_");
    ///     assert_eq!(env.clone().only(&["bar", "baz.bob"]).iter().count(), 2);
    ///     assert_eq!(env.clone().only(&["bar", "bam_bop"]).iter().count(), 1);
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn only(self, keys: &[&str]) -> Self {
        let keys: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        self.filter(move |key| keys.iter().any(|k| k.as_str() == key))
    }

    /// Returns an iterator over all of the environment variable `(key, value)`
    /// pairs that will be considered by `self`. The order is not specified.
    ///
    /// Keys are lower-cased with leading and trailing whitespace removed. Empty
    /// keys, or partially empty keys, are not emitted.
    ///
    /// Any non-Unicode sequences in values are replaced with `U+FFFD
    /// REPLACEMENT CHARACTER`. Values are otherwise unmodified.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("FOO_B", 2);
    ///     jail.set_env("FOO_A", 1);
    ///     jail.set_env("FOO_C", 3);
    ///
    ///     let env = Env::prefixed("FOO_");
    ///     let mut pairs: Vec<_> = env.iter().collect();
    ///     pairs.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
    ///
    ///     assert_eq!(pairs.len(), 3);
    ///     assert_eq!(pairs[0], ("a".into(), "1".into()));
    ///     assert_eq!(pairs[1], ("b".into(), "2".into()));
    ///     assert_eq!(pairs[2], ("c".into(), "3".into()));
    ///
    ///     jail.set_env("FOO_D", 4);
    ///     let mut pairs: Vec<_> = env.iter().collect();
    ///     pairs.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
    ///
    ///     assert_eq!(pairs.len(), 4);
    ///     assert_eq!(pairs[3], ("d".into(), "4".into()));
    ///
    ///     Ok(())
    /// });
    /// ```
    pub fn iter<'a>(&'a self) -> impl Iterator<Item=(Uncased<'static>, String)> + 'a {
        std::env::vars_os()
            .filter(|(k, _)| !k.is_empty())
            .filter_map(move |(k, v)| {
                let key = k.to_string_lossy();
                let key = (self.filter_map)(UncasedStr::new(key.trim()))?;
                let key = key.as_str().trim();
                if key.split('.').any(|s| s.is_empty()) { return None }

                let key = match self.lowercase {
                    true => key.to_ascii_lowercase(),
                    false => key.to_owned(),
                };

                Some((key.into(), v.to_string_lossy().to_string()))
            })
    }

    /// Sets the profile config data will be emitted to.
    ///
    /// ```rust
    /// use figment::{Profile, providers::Env};
    ///
    /// let env = Env::raw();
    /// assert_eq!(env.profile, Profile::Default);
    ///
    /// let env = env.profile("debug");
    /// assert_eq!(env.profile, Profile::from("debug"));
    /// ```
    pub fn profile<P: Into<Profile>>(mut self, profile: P) -> Self {
        self.profile = profile.into();
        self
    }

    /// Sets the profile config data will be emitted to to `global`.
    ///
    /// ```rust
    /// use figment::{Profile, providers::Env};
    ///
    /// let env = Env::raw();
    /// assert_eq!(env.profile, Profile::Default);
    ///
    /// let env = env.global();
    /// assert_eq!(env.profile, Profile::Global);
    /// ```
    pub fn global(mut self) -> Self {
        self.profile = Profile::Global;
        self
    }

    /// A convenience method to retrieve the value for an environment variable
    /// with name `name`. Retrieval is case-insensitive.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("TESTING", 123);
    ///     assert_eq!(Env::var("testing"), Some("123".to_string()));
    ///     Ok(())
    /// });
    /// ```
    pub fn var(name: &str) -> Option<String> {
        for (env_key, val) in std::env::vars_os() {
            let env_key = env_key.to_string_lossy();
            if uncased::eq(env_key.trim(), name) {
                return Some(val.to_string_lossy().trim().into());
            }
        }

        None
    }

    /// A convenience method to retrieve the value for an environment variable
    /// with name `name` or a default `default` if one is not set. Retrieval
    /// is case-insensitive.
    ///
    /// ```rust
    /// use figment::{Jail, providers::Env};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("TESTING", 123);
    ///     assert_eq!(Env::var_or("testing", "whoops"), "123");
    ///     assert_eq!(Env::var_or("hi", "whoops"), "whoops");
    ///     Ok(())
    /// });
    /// ```
    pub fn var_or<S: Into<String>>(name: &str, default: S) -> String {
        Self::var(name).unwrap_or_else(|| default.into())
    }
}

impl Provider for Env {
    fn metadata(&self) -> Metadata {
        let mut md = Metadata::named("environment variable(s)")
            .interpolater(move |_: &Profile, k: &[&str]| {
                let keys: Vec<_> = k.iter()
                    .map(|k| k.to_ascii_uppercase())
                    .collect();

                keys.join(".")
            });

        if let Some(prefix) = &self.prefix {
            md.name = format!("`{}` {}", prefix.to_ascii_uppercase(), md.name).into();
        }

        md
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = Dict::new();
        for (k, v) in self.iter() {
            let nested_dict = nest(k.as_str(), v.parse().expect("infallible"))
                .into_dict()
                .expect("key is non-empty: must have dict");

            dict = dict.merge(nested_dict);
        }

        Ok(self.profile.collect(dict))
    }
}
