// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Tools for locale fallback, enabling arbitrary input locales to be mapped into the nearest
//! locale with data.

use crate::provider::*;
use icu_locid::extensions::unicode::Value;
use icu_locid::subtags::Variants;
use icu_provider::prelude::*;

#[doc(inline)]
pub use icu_provider::fallback::*;

mod algorithms;

/// Implements the algorithm defined in *[UTS #35: Locale Inheritance and Matching]*.
///
/// Note that this implementation performs some additional steps compared to the *UTS #35*
/// algorithm. See *[the design doc]* for a detailed description and [#2243](
/// https://github.com/unicode-org/icu4x/issues/2243) to track alignment with *UTS #35*.
///
/// If running fallback in a loop, use [`DataLocale::is_und()`] to break from the loop.
///
/// # Examples
///
/// ```
/// use icu::locid::locale;
/// use icu::locid_transform::fallback::LocaleFallbacker;
///
/// // Set up a LocaleFallbacker with data.
/// let fallbacker = LocaleFallbacker::new();
///
/// // Create a LocaleFallbackerIterator with a default configuration.
/// // By default, uses language priority with no additional extension keywords.
/// let mut fallback_iterator = fallbacker
///     .for_config(Default::default())
///     .fallback_for(locale!("hi-Latn-IN").into());
///
/// // Run the algorithm and check the results.
/// assert_eq!(fallback_iterator.get(), &locale!("hi-Latn-IN").into());
/// fallback_iterator.step();
/// assert_eq!(fallback_iterator.get(), &locale!("hi-Latn").into());
/// fallback_iterator.step();
/// assert_eq!(fallback_iterator.get(), &locale!("en-IN").into());
/// fallback_iterator.step();
/// assert_eq!(fallback_iterator.get(), &locale!("en-001").into());
/// fallback_iterator.step();
/// assert_eq!(fallback_iterator.get(), &locale!("en").into());
/// fallback_iterator.step();
/// assert_eq!(fallback_iterator.get(), &locale!("und").into());
/// ```
///
/// [UTS #35: Locale Inheritance and Matching]: https://www.unicode.org/reports/tr35/#Locale_Inheritance
/// [the design doc]: https://docs.google.com/document/d/1Mp7EUyl-sFh_HZYgyeVwj88vJGpCBIWxzlCwGgLCDwM/edit
/// [language identifier]: icu::locid::LanguageIdentifier
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct LocaleFallbacker {
    likely_subtags: DataPayload<LocaleFallbackLikelySubtagsV1Marker>,
    parents: DataPayload<LocaleFallbackParentsV1Marker>,
    collation_supplement: Option<DataPayload<CollationFallbackSupplementV1Marker>>,
}

/// Borrowed version of [`LocaleFallbacker`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocaleFallbackerBorrowed<'a> {
    likely_subtags: &'a LocaleFallbackLikelySubtagsV1<'a>,
    parents: &'a LocaleFallbackParentsV1<'a>,
    collation_supplement: Option<&'a LocaleFallbackSupplementV1<'a>>,
}

/// A [`LocaleFallbackerBorrowed`] with an associated [`LocaleFallbackConfig`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocaleFallbackerWithConfig<'a> {
    likely_subtags: &'a LocaleFallbackLikelySubtagsV1<'a>,
    parents: &'a LocaleFallbackParentsV1<'a>,
    supplement: Option<&'a LocaleFallbackSupplementV1<'a>>,
    config: LocaleFallbackConfig,
}

/// Inner iteration type. Does not own the item under fallback.
#[derive(Debug)]
struct LocaleFallbackIteratorInner<'a> {
    likely_subtags: &'a LocaleFallbackLikelySubtagsV1<'a>,
    parents: &'a LocaleFallbackParentsV1<'a>,
    supplement: Option<&'a LocaleFallbackSupplementV1<'a>>,
    config: LocaleFallbackConfig,
    backup_extension: Option<Value>,
    backup_subdivision: Option<Value>,
    backup_variants: Option<Variants>,
}

/// Iteration type for locale fallback operations.
///
/// Because the `Iterator` trait does not allow items to borrow from the iterator, this class does
/// not implement that trait. Instead, use `.step()` and `.get()`.
#[derive(Debug)]
pub struct LocaleFallbackIterator<'a, 'b> {
    current: DataLocale,
    inner: LocaleFallbackIteratorInner<'a>,
    phantom: core::marker::PhantomData<&'b ()>,
}

impl LocaleFallbacker {
    /// Creates a [`LocaleFallbacker`] with compiled fallback data (likely subtags and parent locales).
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    #[cfg(feature = "compiled_data")]
    #[allow(clippy::new_ret_no_self)] // keeping constructors together
    pub const fn new<'a>() -> LocaleFallbackerBorrowed<'a> {
        let tickstatic = LocaleFallbackerBorrowed {
            likely_subtags: crate::provider::Baked::SINGLETON_FALLBACK_LIKELYSUBTAGS_V1,
            parents: crate::provider::Baked::SINGLETON_FALLBACK_PARENTS_V1,
            collation_supplement: Some(crate::provider::Baked::SINGLETON_FALLBACK_SUPPLEMENT_CO_V1),
        };
        // Safety: we're transmuting down from LocaleFallbackerBorrowed<'static> to LocaleFallbackerBorrowed<'a>
        // ZeroMaps use associated types in a way that confuse the compiler which gives up and marks them
        // as invariant. However, they are covariant, and in non-const code this covariance can be safely triggered
        // using Yokeable::transform. In const code we must transmute. In the long run we should
        // be able to `transform()` in const code, and also we will have hopefully improved map polymorphism (#3128)
        unsafe { core::mem::transmute(tickstatic) }
    }

    icu_provider::gen_any_buffer_data_constructors!(locale: skip, options: skip, error: DataError,
        #[cfg(skip)]
        functions: [
            new,
            try_new_with_any_provider,
            try_new_with_buffer_provider,
            try_new_unstable,
            Self
    ]);

    #[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, Self::new)]
    pub fn try_new_unstable<P>(provider: &P) -> Result<Self, DataError>
    where
        P: DataProvider<LocaleFallbackLikelySubtagsV1Marker>
            + DataProvider<LocaleFallbackParentsV1Marker>
            + DataProvider<CollationFallbackSupplementV1Marker>
            + ?Sized,
    {
        let likely_subtags = provider.load(Default::default())?.take_payload()?;
        let parents = provider.load(Default::default())?.take_payload()?;
        let collation_supplement = match DataProvider::<CollationFallbackSupplementV1Marker>::load(
            provider,
            Default::default(),
        ) {
            Ok(response) => Some(response.take_payload()?),
            // It is expected that not all keys are present
            Err(DataError {
                kind: DataErrorKind::MissingDataKey,
                ..
            }) => None,
            Err(e) => return Err(e),
        };
        Ok(LocaleFallbacker {
            likely_subtags,
            parents,
            collation_supplement,
        })
    }

    /// Creates a [`LocaleFallbacker`] without fallback data. Using this constructor may result in
    /// surprising behavior, especially in multi-script languages.
    pub fn new_without_data() -> Self {
        LocaleFallbacker {
            likely_subtags: DataPayload::from_owned(Default::default()),
            parents: DataPayload::from_owned(Default::default()),
            collation_supplement: None,
        }
    }

    /// Associates a configuration with this fallbacker.
    #[inline]
    pub fn for_config(&self, config: LocaleFallbackConfig) -> LocaleFallbackerWithConfig {
        self.as_borrowed().for_config(config)
    }

    /// Derives a configuration from a [`DataKey`] and associates it
    /// with this fallbacker.
    #[inline]
    #[doc(hidden)] // will be removed in 2.0
    pub fn for_key(&self, data_key: DataKey) -> LocaleFallbackerWithConfig {
        self.for_config(data_key.fallback_config())
    }

    /// Creates a borrowed version of this fallbacker for performance.
    pub fn as_borrowed(&self) -> LocaleFallbackerBorrowed {
        LocaleFallbackerBorrowed {
            likely_subtags: self.likely_subtags.get(),
            parents: self.parents.get(),
            collation_supplement: self.collation_supplement.as_ref().map(|p| p.get()),
        }
    }
}

impl<'a> LocaleFallbackerBorrowed<'a> {
    /// Associates a configuration with this fallbacker.
    #[inline]
    pub const fn for_config(self, config: LocaleFallbackConfig) -> LocaleFallbackerWithConfig<'a> {
        LocaleFallbackerWithConfig {
            likely_subtags: self.likely_subtags,
            parents: self.parents,
            supplement: match config.fallback_supplement {
                Some(LocaleFallbackSupplement::Collation) => self.collation_supplement,
                _ => None,
            },
            config,
        }
    }
}

impl LocaleFallbackerBorrowed<'static> {
    /// Cheaply converts a [`LocaleFallbackerBorrowed<'static>`] into a [`LocaleFallbacker`].
    ///
    /// Note: Due to branching and indirection, using [`LocaleFallbacker`] might inhibit some
    /// compile-time optimizations that are possible with [`LocaleFallbackerBorrowed`].
    pub const fn static_to_owned(self) -> LocaleFallbacker {
        LocaleFallbacker {
            likely_subtags: DataPayload::from_static_ref(self.likely_subtags),
            parents: DataPayload::from_static_ref(self.parents),
            collation_supplement: match self.collation_supplement {
                None => None,
                Some(x) => Some(DataPayload::from_static_ref(x)),
            },
        }
    }
}

impl<'a> LocaleFallbackerWithConfig<'a> {
    /// Creates an iterator based on a [`DataLocale`].
    ///
    /// If you have a [`Locale`](icu_locid::Locale), call `.into()` to get a [`DataLocale`].
    ///
    /// When first initialized, the locale is normalized according to the fallback algorithm.
    pub fn fallback_for(&self, mut locale: DataLocale) -> LocaleFallbackIterator<'a, 'static> {
        self.normalize(&mut locale);
        LocaleFallbackIterator {
            current: locale,
            inner: LocaleFallbackIteratorInner {
                likely_subtags: self.likely_subtags,
                parents: self.parents,
                supplement: self.supplement,
                config: self.config,
                backup_extension: None,
                backup_subdivision: None,
                backup_variants: None,
            },
            phantom: core::marker::PhantomData,
        }
    }
}

impl LocaleFallbackIterator<'_, '_> {
    /// Borrows the current [`DataLocale`] under fallback.
    pub fn get(&self) -> &DataLocale {
        &self.current
    }

    /// Takes the current [`DataLocale`] under fallback.
    pub fn take(self) -> DataLocale {
        self.current
    }

    /// Performs one step of the locale fallback algorithm.
    ///
    /// The fallback is completed once the inner [`DataLocale`] becomes `und`.
    pub fn step(&mut self) -> &mut Self {
        self.inner.step(&mut self.current);
        self
    }
}
