//! Useful functions and macros for writing figments.
//!
//! # `map!` macro
//!
//! The `map!` macro constructs a [`Map`](crate::value::Map) from key-value
//! pairs and is particularly useful during testing:
//!
//! ```rust
//! use figment::util::map;
//!
//! let map = map! {
//!     "name" => "Bob",
//!     "age" => "100"
//! };
//!
//! assert_eq!(map.get("name"), Some(&"Bob"));
//! assert_eq!(map.get("age"), Some(&"100"));
//!
//! let map = map! {
//!     100 => "one hundred",
//!     23 => "twenty-three"
//! };
//!
//! assert_eq!(map.get(&100), Some(&"one hundred"));
//! assert_eq!(map.get(&23), Some(&"twenty-three"));
//!
//! ```
use std::fmt;
use std::path::{Path, PathBuf, Component};

use serde::de::{self, Unexpected, Deserializer};

/// A helper function to determine the relative path to `path` from `base`.
///
/// Returns `None` if there is no relative path from `base` to `path`, that is,
/// `base` and `path` do not share a common ancestor. `path` and `base` must be
/// either both absolute or both relative; returns `None` if one is relative and
/// the other absolute.
///
/// ```
/// use std::path::Path;
/// use figment::util::diff_paths;
///
/// // Paths must be both relative or both absolute.
/// assert_eq!(diff_paths("/a/b/c", "b/c"), None);
/// assert_eq!(diff_paths("a/b/c", "/b/c"), None);
///
/// // The root/relative root is always a common ancestor.
/// assert_eq!(diff_paths("/a/b/c", "/b/c"), Some("../../a/b/c".into()));
/// assert_eq!(diff_paths("c/a", "b/c/a"), Some("../../../c/a".into()));
///
/// let bar = "/foo/bar";
/// let baz = "/foo/bar/baz";
/// let quux = "/foo/bar/quux";
///
/// assert_eq!(diff_paths(bar, baz), Some("../".into()));
/// assert_eq!(diff_paths(baz, bar), Some("baz".into()));
/// assert_eq!(diff_paths(quux, baz), Some("../quux".into()));
/// assert_eq!(diff_paths(baz, quux), Some("../baz".into()));
/// assert_eq!(diff_paths(bar, quux), Some("../".into()));
/// assert_eq!(diff_paths(baz, bar), Some("baz".into()));
/// ```
// Copyright 2012-2015 The Rust Project Developers.
// Copyright 2017 The Rust Project Developers.
// Adapted from `pathdiff`, which itself adapted from rustc's path_relative_from.
pub fn diff_paths<P, B>(path: P, base: B) -> Option<PathBuf>
     where P: AsRef<Path>, B: AsRef<Path>
{
    let (path, base) = (path.as_ref(), base.as_ref());
    if path.has_root() != base.has_root() {
        return None;
    }

    let mut ita = path.components();
    let mut itb = base.components();
    let mut comps: Vec<Component> = vec![];
    loop {
        match (ita.next(), itb.next()) {
            (None, None) => break,
            (Some(a), None) => {
                comps.push(a);
                comps.extend(ita.by_ref());
                break;
            }
            (None, _) => comps.push(Component::ParentDir),
            (Some(a), Some(b)) if comps.is_empty() && a == b => (),
            (Some(a), Some(b)) if b == Component::CurDir => comps.push(a),
            (Some(_), Some(b)) if b == Component::ParentDir => return None,
            (Some(a), Some(_)) => {
                comps.push(Component::ParentDir);
                for _ in itb {
                    comps.push(Component::ParentDir);
                }
                comps.push(a);
                comps.extend(ita.by_ref());
                break;
            }
        }
    }

    Some(comps.iter().map(|c| c.as_os_str()).collect())
}

/// A helper to deserialize `0/false` as `false` and `1/true` as `true`.
///
/// Serde's default deserializer for `bool` only parses the strings `"true"` and
/// `"false"` as the booleans `true` and `false`, respectively. By contract,
/// this function _case-insensitively_ parses both the strings `"true"/"false"`
/// and the integers `1/0` as the booleans `true/false`, respectively.
///
/// # Example
///
/// ```rust
/// use figment::Figment;
///
/// #[derive(serde::Deserialize)]
/// struct Config {
///     #[serde(deserialize_with = "figment::util::bool_from_str_or_int")]
///     cli_colors: bool,
/// }
///
/// let c0: Config = Figment::from(("cli_colors", "true")).extract().unwrap();
/// let c1: Config = Figment::from(("cli_colors", "TRUE")).extract().unwrap();
/// let c2: Config = Figment::from(("cli_colors", 1)).extract().unwrap();
/// assert_eq!(c0.cli_colors, true);
/// assert_eq!(c1.cli_colors, true);
/// assert_eq!(c2.cli_colors, true);
///
/// let c0: Config = Figment::from(("cli_colors", "false")).extract().unwrap();
/// let c1: Config = Figment::from(("cli_colors", "fAlSe")).extract().unwrap();
/// let c2: Config = Figment::from(("cli_colors", 0)).extract().unwrap();
/// assert_eq!(c0.cli_colors, false);
/// assert_eq!(c1.cli_colors, false);
/// assert_eq!(c2.cli_colors, false);
/// ```
pub fn bool_from_str_or_int<'de, D: Deserializer<'de>>(de: D) -> Result<bool, D::Error> {
    struct Visitor;

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = bool;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a boolean")
        }

        fn visit_str<E: de::Error>(self, val: &str) -> Result<bool, E> {
            match val {
                v if uncased::eq(v, "true") => Ok(true),
                v if uncased::eq(v, "false") => Ok(false),
                s => Err(E::invalid_value(Unexpected::Str(s), &"true or false"))
            }
        }

        fn visit_u64<E: de::Error>(self, n: u64) -> Result<bool, E> {
            match n {
                0 | 1 => Ok(n != 0),
                n => Err(E::invalid_value(Unexpected::Unsigned(n), &"0 or 1"))
            }
        }

        fn visit_i64<E: de::Error>(self, n: i64) -> Result<bool, E> {
            match n {
                0 | 1 => Ok(n != 0),
                n => Err(E::invalid_value(Unexpected::Signed(n), &"0 or 1"))
            }
        }

        fn visit_bool<E: de::Error>(self, b: bool) -> Result<bool, E> {
            Ok(b)
        }
    }

    de.deserialize_any(Visitor)
}

/// A helper to serialize and deserialize a map as a vector of `(key, value)`
/// pairs.
///
/// ```
/// use figment::{Figment, util::map};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// pub struct Config {
///     #[serde(with = "figment::util::vec_tuple_map")]
///     pairs: Vec<(String, usize)>
/// }
///
/// let map = map!["key" => 1, "value" => 100, "name" => 20];
/// let c: Config = Figment::from(("pairs", map)).extract().unwrap();
/// assert_eq!(c.pairs.len(), 3);
///
/// let mut pairs = c.pairs;
/// pairs.sort_by_key(|(_, v)| *v);
///
/// assert_eq!(pairs[0], ("key".into(), 1));
/// assert_eq!(pairs[1], ("name".into(), 20));
/// assert_eq!(pairs[2], ("value".into(), 100));
/// ```
pub mod vec_tuple_map {
    use std::fmt;
    use serde::{de, Deserialize, Serialize, Deserializer, Serializer};

    /// The serializer half.
    pub fn serialize<S, K, V>(vec: &[(K, V)], se: S) -> Result<S::Ok, S::Error>
        where S: Serializer, K: Serialize, V: Serialize
    {
        se.collect_map(vec.iter().map(|(ref k, ref v)| (k, v)))
    }

    /// The deserializer half.
    pub fn deserialize<'de, K, V, D>(de: D) -> Result<Vec<(K, V)>, D::Error>
        where D: Deserializer<'de>, K: Deserialize<'de>, V: Deserialize<'de>
    {
        struct Visitor<K, V>(std::marker::PhantomData<Vec<(K, V)>>);

        impl<'de, K, V> de::Visitor<'de> for Visitor<K, V>
            where K: Deserialize<'de>, V: Deserialize<'de>,
        {
            type Value = Vec<(K, V)>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Vec<(K, V)>, A::Error>
                where A: de::MapAccess<'de>
            {
                let mut vec = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((k, v)) = map.next_entry()? {
                    vec.push((k, v));
                }

                Ok(vec)
            }
        }

        de.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

use crate::value::{Value, Dict};

/// Given a key path `key` of the form `a.b.c`, creates nested dictionaries for
/// for every path component delimited by `.` in the path string (3 in `a.b.c`),
/// each a parent of the next, and the leaf mapping to `value` (`a` -> `b` ->
/// `c` -> `value`).
///
/// If `key` is empty, simply returns `value`. Otherwise, `Value` will be a
/// dictionary with the nested mappings.
///
/// # Example
///
/// ```rust
/// use figment::{util::nest, value::Value};
///
/// let leaf = Value::from("I'm a leaf!");
///
/// let dict = nest("tea", leaf.clone());
/// assert_eq!(dict.find_ref("tea").unwrap(), &leaf);
///
/// let dict = nest("tea.leaf", leaf.clone());
/// let tea = dict.find_ref("tea").unwrap();
/// let found_leaf = tea.find_ref("leaf").unwrap();
/// assert_eq!(found_leaf, &leaf);
/// assert_eq!(dict.find_ref("tea.leaf").unwrap(), &leaf);
///
/// let just_leaf = nest("", leaf.clone());
/// assert_eq!(just_leaf, leaf);
/// ```
pub fn nest(key: &str, value: Value) -> Value {
    fn value_from(mut keys: std::str::Split<'_, char>, value: Value) -> Value {
        match keys.next() {
            Some(k) if !k.is_empty() => {
                let mut dict = Dict::new();
                dict.insert(k.into(), value_from(keys, value));
                dict.into()
            }
            Some(_) | None => value
        }
    }

    value_from(key.split('.'), value)
}

#[doc(hidden)]
#[macro_export]
/// This is a macro.
macro_rules! map {
    ($($key:expr => $value:expr),* $(,)?) => ({
        let mut map = $crate::value::Map::new();
        $(map.insert($key, $value);)*
        map
    });
}

pub use map;

#[doc(hidden)]
#[macro_export]
macro_rules! make_cloneable {
    ($Trait:path: $Cloneable:ident) => {
        trait $Cloneable {
            fn box_clone(&self) -> Box<dyn $Trait>;
        }

        impl std::clone::Clone for Box<dyn $Trait> {
            fn clone(&self) -> Box<dyn $Trait> {
                (&**self).box_clone()
            }
        }

        impl std::fmt::Debug for Box<dyn $Trait> {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                Ok(())
            }
        }

        impl<T: $Trait + Clone> $Cloneable for T {
            fn box_clone(&self) -> Box<dyn $Trait> {
                Box::new(self.clone())
            }
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! cloneable_fn_trait {
    ($Name:ident: $($rest:tt)*) => {
        trait $Name: $($rest)* + Cloneable + 'static { }
        impl<F: Clone + 'static> $Name for F where F: $($rest)* { }
        $crate::make_cloneable!($Name: Cloneable);
    }
}

pub(crate) use cloneable_fn_trait;
