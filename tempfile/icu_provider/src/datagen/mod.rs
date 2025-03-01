// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! This module contains various utilities required to generate ICU4X data files, typically
//! via the `icu_datagen` reference crate. End users should not need to consume anything in
//! this module as a library unless defining new types that integrate with `icu_datagen`.
//!
//! This module can be enabled with the `datagen` Cargo feature on `icu_provider`.

mod data_conversion;
mod iter;
mod payload;
pub use data_conversion::DataConverter;
pub use iter::IterableDataProvider;

#[doc(hidden)] // exposed for make_exportable_provider
pub use iter::IterableDynamicDataProvider;
#[doc(hidden)] // exposed for make_exportable_provider
pub use payload::{ExportBox, ExportMarker};

use crate::prelude::*;

/// The type of built-in fallback that the data was generated for, if applicable.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuiltInFallbackMode {
    /// Data uses full UTS 35 fallbacking.
    Standard,
}

/// An object capable of exporting data payloads in some form.
pub trait DataExporter: Sync {
    /// Save a `payload` corresponding to the given key and locale.
    /// Takes non-mut self as it can be called concurrently.
    fn put_payload(
        &self,
        key: DataKey,
        locale: &DataLocale,
        payload: &DataPayload<ExportMarker>,
    ) -> Result<(), DataError>;

    /// Function called for singleton keys.
    /// Takes non-mut self as it can be called concurrently.
    fn flush_singleton(
        &self,
        key: DataKey,
        payload: &DataPayload<ExportMarker>,
    ) -> Result<(), DataError> {
        self.put_payload(key, &Default::default(), payload)?;
        self.flush(key)
    }

    /// Function called after a non-singleton key has been fully enumerated,
    /// flushing that key with built-in fallback.
    ///
    /// Takes non-mut self as it can be called concurrently.
    fn flush_with_built_in_fallback(
        &self,
        _key: DataKey,
        _fallback_mode: BuiltInFallbackMode,
    ) -> Result<(), DataError> {
        Err(DataError::custom(
            "Exporter does not implement built-in fallback",
        ))
    }

    /// Function called after a non-singleton key has been fully enumerated.
    /// Does not include built-in fallback.
    ///
    /// Takes non-mut self as it can be called concurrently.
    fn flush(&self, _key: DataKey) -> Result<(), DataError> {
        Ok(())
    }

    /// This function has to be called before the object is dropped (after all
    /// keys have been fully dumped). This conceptually takes ownership, so
    /// clients *may not* interact with this object after close has been called.
    fn close(&mut self) -> Result<(), DataError> {
        Ok(())
    }

    /// Returns whether the provider supports built-in fallback. If `true`, the provider must
    /// implement [`Self::flush_with_built_in_fallback()`].
    fn supports_built_in_fallback(&self) -> bool {
        false
    }
}

impl DataExporter for Box<dyn DataExporter> {
    fn put_payload(
        &self,
        key: DataKey,
        locale: &DataLocale,
        payload: &DataPayload<ExportMarker>,
    ) -> Result<(), DataError> {
        (**self).put_payload(key, locale, payload)
    }

    fn flush_singleton(
        &self,
        key: DataKey,
        payload: &DataPayload<ExportMarker>,
    ) -> Result<(), DataError> {
        (**self).flush_singleton(key, payload)
    }

    fn flush_with_built_in_fallback(
        &self,
        key: DataKey,
        fallback_mode: BuiltInFallbackMode,
    ) -> Result<(), DataError> {
        (**self).flush_with_built_in_fallback(key, fallback_mode)
    }

    fn flush(&self, key: DataKey) -> Result<(), DataError> {
        (**self).flush(key)
    }

    fn close(&mut self) -> Result<(), DataError> {
        (**self).close()
    }

    fn supports_built_in_fallback(&self) -> bool {
        (**self).supports_built_in_fallback()
    }
}

/// A [`DynamicDataProvider`] that can be used for exporting data.
///
/// Use [`make_exportable_provider`](crate::make_exportable_provider) to implement this.
pub trait ExportableProvider:
    IterableDynamicDataProvider<ExportMarker> + DynamicDataProvider<AnyMarker> + Sync
{
}

impl<T> ExportableProvider for T where
    T: IterableDynamicDataProvider<ExportMarker> + DynamicDataProvider<AnyMarker> + Sync
{
}

/// This macro can be used on a data provider to allow it to be used for data generation.
///
/// Data generation 'compiles' data by using this data provider (which usually translates data from
/// different sources and doesn't have to be efficient) to generate data structs, and then writing
/// them to an efficient format like [`BlobDataProvider`] or [`BakedDataProvider`]. The requirements
/// for `make_exportable_provider` are:
/// * The data struct has to implement [`serde::Serialize`](::serde::Serialize) and [`databake::Bake`]
/// * The provider needs to implement [`IterableDataProvider`] for all specified [`KeyedDataMarker`]s.
///   This allows the generating code to know which [`DataLocale`] to collect.
///
/// [`BlobDataProvider`]: ../../icu_provider_blob/struct.BlobDataProvider.html
/// [`BakedDataProvider`]: ../../icu_datagen/index.html
#[macro_export]
macro_rules! make_exportable_provider {
    ($provider:ty, [ $($(#[$cfg:meta])? $struct_m:ty),+, ]) => {
        $crate::impl_dynamic_data_provider!(
            $provider,
            [ $($(#[$cfg])? $struct_m),+, ],
            $crate::datagen::ExportMarker
        );
        $crate::impl_dynamic_data_provider!(
            $provider,
            [ $($(#[$cfg])? $struct_m),+, ],
            $crate::any::AnyMarker
        );

        impl $crate::datagen::IterableDynamicDataProvider<$crate::datagen::ExportMarker> for $provider {
            fn supported_locales_for_key(&self, key: $crate::DataKey) -> Result<Vec<$crate::DataLocale>, $crate::DataError> {
                match key.hashed() {
                    $(
                        $(#[$cfg])?
                        h if h == <$struct_m as $crate::KeyedDataMarker>::KEY.hashed() => {
                            $crate::datagen::IterableDataProvider::<$struct_m>::supported_locales(self)
                        }
                    )+,
                    _ => Err($crate::DataErrorKind::MissingDataKey.with_key(key))
                }
            }
        }
    };
}

/// A `DataExporter` that forks to multiple `DataExporter`s.
#[derive(Default)]
pub struct MultiExporter(Vec<Box<dyn DataExporter>>);

impl MultiExporter {
    /// Creates a `MultiExporter` for the given exporters.
    pub const fn new(exporters: Vec<Box<dyn DataExporter>>) -> Self {
        Self(exporters)
    }
}

impl core::fmt::Debug for MultiExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiExporter")
            .field("0", &format!("vec[len = {}]", self.0.len()))
            .finish()
    }
}

impl DataExporter for MultiExporter {
    fn put_payload(
        &self,
        key: DataKey,
        locale: &DataLocale,
        payload: &DataPayload<ExportMarker>,
    ) -> Result<(), DataError> {
        self.0
            .iter()
            .try_for_each(|e| e.put_payload(key, locale, payload))
    }

    fn flush_singleton(
        &self,
        key: DataKey,
        payload: &DataPayload<ExportMarker>,
    ) -> Result<(), DataError> {
        self.0
            .iter()
            .try_for_each(|e| e.flush_singleton(key, payload))
    }

    fn flush(&self, key: DataKey) -> Result<(), DataError> {
        self.0.iter().try_for_each(|e| e.flush(key))
    }

    fn flush_with_built_in_fallback(
        &self,
        key: DataKey,
        fallback_mode: BuiltInFallbackMode,
    ) -> Result<(), DataError> {
        self.0
            .iter()
            .try_for_each(|e| e.flush_with_built_in_fallback(key, fallback_mode))
    }

    fn close(&mut self) -> Result<(), DataError> {
        self.0.iter_mut().try_for_each(|e| e.close())
    }
}
