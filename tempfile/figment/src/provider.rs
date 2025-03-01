use crate::{Profile, Error, Metadata};
use crate::value::{Tag, Map, Dict};

/// Trait implemented by configuration source providers.
///
/// For an overview of built-in providers, see the [top-level
/// docs](crate#built-in-providers).
///
/// # Overview
///
/// A [`Provider`] reads from a source to provide configuration data for
/// [`Figment`]s ([`Provider::data()`]). A `Provider` also provides [`Metadata`]
/// to identify the source of its configuration data ([`Provider::metadata()`]).
/// A provider may also optionally set a `Profile` for the `Figment` it is
/// merged (but not joined) into by implementing [`Provider::profile()`].
///
/// # Nesting
///
/// A [`Provider`] meant to be consumed externally should allow for optional
/// [nesting](crate#extracting-and-profiles) when sensible. The general pattern
/// is to allow a `Profile` to be specified. If one is not, read the
/// configuration data as a `Map<Profile, Dict>`, thus using the top-level keys
/// as profiles. If one _is_ specified, read the data as `Dict` and
/// [`Profile::collect()`] into the specified profile.
///
/// # Example
///
/// Implementing a `Provider` requires implementing methods that provide both of
/// these pieces of data. The first, [`Provider::metadata()`] identifies the
/// provider's configuration sources, if any, and allows the provider to
/// customize how paths to keys are interpolated. The second,
/// [`Provider::data()`], actually reads the configuration and returns the data.
///
/// As an example, consider a provider that reads configuration from a
/// networked store at some `Url`. A `Provider` implementation for such a
/// provider may resemble the following:
///
/// ```rust,no_run
/// # use serde::Deserialize;
/// use figment::{Provider, Metadata, Profile, Error, value::{Map, Dict}};
///
/// # type Url = String;
/// /// A provider that fetches its data from a given URL.
/// struct NetProvider {
///     /// The profile to emit data to if nesting is disabled.
///     profile: Option<Profile>,
///     /// The url to fetch data from.
///     url: Url
/// };
///
/// impl Provider for NetProvider {
///     /// Returns metadata with kind `Network`, custom source `self.url`,
///     /// and interpolator that returns a URL of `url/a/b/c` for key `a.b.c`.
///     fn metadata(&self) -> Metadata {
///         let url = self.url.clone();
///         Metadata::named("Network")
///             .source(self.url.as_str())
///             .interpolater(move |profile, keys| match profile.is_custom() {
///                 true => format!("{}/{}/{}", url, profile, keys.join("/")),
///                 false => format!("{}/{}", url, keys.join("/")),
///             })
///     }
///
///     /// Fetches the data from `self.url`. Note that `Dict`, `Map`, and
///     /// `Profile` are `Deserialize`, so we can deserialized to them.
///     fn data(&self) -> Result<Map<Profile, Dict>, Error> {
///         fn fetch<'a, T: Deserialize<'a>>(url: &Url) -> Result<T, Error> {
///             /* fetch from the network, deserialize into `T` */
///             # todo!()
///         }
///
///         match &self.profile {
///             // Don't nest: `fetch` into a `Dict`.
///             Some(profile) => Ok(profile.collect(fetch(&self.url)?)),
///             // Nest: `fetch` into a `Map<Profile, Dict>`.
///             None => fetch(&self.url),
///         }
///     }
/// }
/// ```
///
/// [`Figment`]: crate::Figment
pub trait Provider {
    /// Returns the [`Metadata`] for this provider, identifying itself and its
    /// configuration sources.
    fn metadata(&self) -> Metadata;

    /// Returns the configuration data.
    fn data(&self) -> Result<Map<Profile, Dict>, Error>;

    /// Optionally returns a profile to set on the [`Figment`](crate::Figment)
    /// this provider is merged into. The profile is only set if `self` is
    /// _merged_.
    fn profile(&self) -> Option<Profile> {
        None
    }

    /// This is used internally! Please, please don't use this externally. If
    /// you have a good usecase for this, let me know!
    #[doc(hidden)]
    fn __metadata_map(&self) -> Option<Map<Tag, Metadata>> { None }
}

/// This is exactly `<T as Provider>`.
impl<T: Provider> Provider for &T {
    fn metadata(&self) -> Metadata { T::metadata(self) }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> { T::data(self) }

    fn profile(&self) -> Option<Profile> {
        T::profile(self)
    }

    #[doc(hidden)]
    fn __metadata_map(&self) -> Option<Map<Tag, Metadata>> {
        T::__metadata_map(self)
    }
}

/// This is exactly equivalent to [`Serialized::global(K, V)`].
///
/// [`Serialized::global(K, V)`]: crate::providers::Serialized::global()
impl<K: AsRef<str>, V: serde::Serialize> Provider for (K, V) {
    fn metadata(&self) -> Metadata {
        use std::any::type_name;
        Metadata::named(format!("({}, {})", type_name::<K>(), type_name::<V>()))
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        use crate::providers::Serialized;
        Serialized::global(self.0.as_ref(), &self.1).data()
    }
}
