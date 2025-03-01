use serde::{de, ser};
use uncased::{Uncased, UncasedStr};

use crate::value::{Dict, Map};

/// A configuration profile: effectively a case-insensitive string.
///
/// See [the top-level docs](crate#extracting-and-profiles) for details.
#[derive(Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord)]
pub struct Profile(Uncased<'static>);

impl Default for Profile {
    fn default() -> Self {
        Profile::Default
    }
}

/// 2-bit tags used by the top bits of `Tag`.
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum ProfileTag {
    Default = 0b00,
    Global = 0b01,
    Custom = 0b11,
}

impl From<u8> for ProfileTag {
    fn from(bits: u8) -> ProfileTag {
        if bits == ProfileTag::Default as u8 {
            ProfileTag::Default
        } else if bits == ProfileTag::Global as u8 {
            ProfileTag::Global
        } else {
            ProfileTag::Custom
        }
    }
}

impl From<ProfileTag> for Option<Profile> {
    fn from(tag: ProfileTag) -> Self {
        match tag {
            ProfileTag::Default => Some(Profile::Default),
            ProfileTag::Global => Some(Profile::Global),
            ProfileTag::Custom => None,
        }
    }
}

impl From<&Profile> for ProfileTag {
    fn from(profile: &Profile) -> Self {
        match profile {
            p if p == Profile::Default => ProfileTag::Default,
            p if p == Profile::Global => ProfileTag::Global,
            _ => ProfileTag::Custom
        }
    }
}

impl Profile {
    /// The default profile: `"default"`.
    #[allow(non_upper_case_globals)]
    pub const Default: Profile = Profile::const_new("default");

    /// The global profile: `"global"`.
    #[allow(non_upper_case_globals)]
    pub const Global: Profile = Profile::const_new("global");

    /// Constructs a profile with the name `name`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Profile;
    ///
    /// let profile = Profile::new("staging");
    /// assert_eq!(profile, "staging");
    /// assert_eq!(profile, "STAGING");
    /// ```
    pub fn new(name: &str) -> Profile {
        Profile(name.to_string().into())
    }

    /// A `const` to construct a profile with the name `name`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Profile;
    ///
    /// const STAGING: Profile = Profile::const_new("staging");
    ///
    /// assert_eq!(STAGING, "staging");
    /// assert_eq!(STAGING, "STAGING");
    /// ```
    pub const fn const_new(name: &'static str) -> Profile {
        Profile(Uncased::from_borrowed(name))
    }

    /// Constructs a profile from the value of the environment variable with
    /// name `key`, if one is present. The search for `key` is case-insensitive.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Profile, Jail};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("MY_PROFILE", "secret");
    ///
    ///     assert_eq!(Profile::from_env("MY_PROFILE"), Some("secret".into()));
    ///     assert_eq!(Profile::from_env("MY_PROFILE"), Some("secret".into()));
    ///     assert_eq!(Profile::from_env("MY_profile"), Some("secret".into()));
    ///     assert_eq!(Profile::from_env("other_profile"), None);
    ///     Ok(())
    /// });
    /// ```
    pub fn from_env(key: &str) -> Option<Self> {
        for (env_key, val) in std::env::vars_os() {
            let env_key = env_key.to_string_lossy();
            if uncased::eq(env_key.trim(), key) {
                return Some(Profile::new(&val.to_string_lossy()));
            }
        }

        None
    }

    /// Constructs a profile from the value of the environment variable with
    /// name `var`, if one is present, or `default` if one is not. The search
    /// for `var` is case-insensitive.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Profile, Jail};
    ///
    /// Jail::expect_with(|jail| {
    ///     jail.set_env("MY_PROFILE", "secret");
    ///
    ///     assert_eq!(Profile::from_env_or("MY_PROFILE", "default"), "secret");
    ///     assert_eq!(Profile::from_env_or("MY_profile", "default"), "secret");
    ///     assert_eq!(Profile::from_env_or("other_prof", "default"), "default");
    ///     Ok(())
    /// });
    /// ```
    pub fn from_env_or<P: Into<Profile>>(var: &str, default: P) -> Self {
        Profile::from_env(var).unwrap_or_else(|| default.into())
    }

    /// Converts `self` into an `&UncasedStr`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Profile;
    ///
    /// let profile = Profile::new("static");
    /// let string = profile.as_str();
    /// ```
    pub fn as_str(&self) -> &UncasedStr {
        &self.0
    }

    /// Returns `true` iff `self` case-insensitively starts with `prefix`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Profile;
    ///
    /// let profile = Profile::new("static");
    /// assert!(profile.starts_with("STAT"));
    /// assert!(profile.starts_with("stat"));
    /// assert!(profile.starts_with("static"));
    /// ```
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.as_str().starts_with(prefix)
    }

    /// Returns `true` iff `self` is neither "default" nor "global".
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Profile;
    ///
    /// let profile = Profile::new("static");
    /// assert!(profile.is_custom());
    ///
    /// assert!(!Profile::Default.is_custom());
    /// assert!(!Profile::Global.is_custom());
    /// ```
    pub fn is_custom(&self) -> bool {
        self != Profile::Default && self != Profile::Global
    }

    /// Creates a new map with a single key of `*self` and a value of `dict`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{Profile, util::map};
    ///
    /// let profile = Profile::new("static");
    /// let map = profile.collect(map!["hi".into() => 123.into()]);
    /// ```
    pub fn collect(&self, dict: Dict) -> Map<Profile, Dict> {
        let mut map = Map::new();
        map.insert(self.clone(), dict);
        map
    }
}

impl<T: AsRef<str>> From<T> for Profile {
    fn from(string: T) -> Profile {
        Profile::new(string.as_ref())
    }
}

impl From<Profile> for String {
    fn from(profile: Profile) -> String {
        profile.0.to_string()
    }
}

impl std::ops::Deref for Profile {
    type Target = UncasedStr;

    fn deref(&self) -> &UncasedStr {
        self.as_str()
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

impl PartialEq<str> for Profile {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Profile {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<Profile> for str {
    fn eq(&self, other: &Profile) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Profile> for &str {
    fn eq(&self, other: &Profile) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Profile> for &Profile {
    fn eq(&self, other: &Profile) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<&Profile> for Profile {
    fn eq(&self, other: &&Profile) -> bool {
        self.as_str() == other.as_str()
    }
}

impl<'de> de::Deserialize<'de> for Profile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: serde::Deserializer<'de>
    {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Profile;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Profile::from(v))
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

impl ser::Serialize for Profile {
    fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str().as_str())
    }
}
