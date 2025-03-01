// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Traits for data providers that produce opaque buffers.

use crate::prelude::*;

/// [`DataMarker`] for raw buffers. Returned by [`BufferProvider`].
///
/// The data is expected to be deserialized before it can be used; see
/// [`DataPayload::into_deserialized`].
#[allow(clippy::exhaustive_structs)] // marker type
#[derive(Debug)]
pub struct BufferMarker;

impl DataMarker for BufferMarker {
    type Yokeable = &'static [u8];
}

/// A data provider that returns opaque bytes.
///
/// Generally, these bytes are expected to be deserializable with Serde. To get an object
/// implementing [`DataProvider`] via Serde, use [`as_deserializing()`].
///
/// Passing a  `BufferProvider` to a `*_with_buffer_provider` constructor requires enabling
/// the deserialization Cargo feature for the expected format(s):
/// - `deserialize_json`
/// - `deserialize_postcard_1`
/// - `deserialize_bincode_1`
///
/// Along with [`DataProvider`], this is one of the two foundational traits in this crate.
///
/// [`BufferProvider`] can be made into a trait object. It is used over FFI.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "deserialize_json")] {
/// use icu_locid::langid;
/// use icu_provider::hello_world::*;
/// use icu_provider::prelude::*;
/// use std::borrow::Cow;
///
/// let buffer_provider = HelloWorldProvider.into_json_provider();
///
/// let req = DataRequest {
///     locale: &langid!("de").into(),
///     metadata: Default::default(),
/// };
///
/// // Deserializing manually
/// assert_eq!(
///     serde_json::from_slice::<HelloWorldV1>(
///         buffer_provider
///             .load_buffer(HelloWorldV1Marker::KEY, req)
///             .expect("load should succeed")
///             .take_payload()
///             .unwrap()
///             .get()
///     )
///     .expect("should deserialize"),
///     HelloWorldV1 {
///         message: Cow::Borrowed("Hallo Welt"),
///     },
/// );
///
/// // Deserialize automatically
/// let deserializing_provider: &dyn DataProvider<HelloWorldV1Marker> =
///     &buffer_provider.as_deserializing();
///
/// assert_eq!(
///     deserializing_provider
///         .load(req)
///         .expect("load should succeed")
///         .take_payload()
///         .unwrap()
///         .get(),
///     &HelloWorldV1 {
///         message: Cow::Borrowed("Hallo Welt"),
///     },
/// );
/// # }
/// ```
///
/// [`as_deserializing()`]: AsDeserializingBufferProvider::as_deserializing
pub trait BufferProvider {
    /// Loads a [`DataPayload`]`<`[`BufferMarker`]`>` according to the key and request.
    fn load_buffer(
        &self,
        key: DataKey,
        req: DataRequest,
    ) -> Result<DataResponse<BufferMarker>, DataError>;
}

impl<'a, T: BufferProvider + ?Sized> BufferProvider for &'a T {
    #[inline]
    fn load_buffer(
        &self,
        key: DataKey,
        req: DataRequest,
    ) -> Result<DataResponse<BufferMarker>, DataError> {
        (**self).load_buffer(key, req)
    }
}

impl<T: BufferProvider + ?Sized> BufferProvider for alloc::boxed::Box<T> {
    #[inline]
    fn load_buffer(
        &self,
        key: DataKey,
        req: DataRequest,
    ) -> Result<DataResponse<BufferMarker>, DataError> {
        (**self).load_buffer(key, req)
    }
}

impl<T: BufferProvider + ?Sized> BufferProvider for alloc::rc::Rc<T> {
    #[inline]
    fn load_buffer(
        &self,
        key: DataKey,
        req: DataRequest,
    ) -> Result<DataResponse<BufferMarker>, DataError> {
        (**self).load_buffer(key, req)
    }
}

#[cfg(target_has_atomic = "ptr")]
impl<T: BufferProvider + ?Sized> BufferProvider for alloc::sync::Arc<T> {
    #[inline]
    fn load_buffer(
        &self,
        key: DataKey,
        req: DataRequest,
    ) -> Result<DataResponse<BufferMarker>, DataError> {
        (**self).load_buffer(key, req)
    }
}

/// An enum expressing all Serde formats known to ICU4X.
#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BufferFormat {
    /// Serialize using JavaScript Object Notation (JSON).
    Json,
    /// Serialize using Bincode version 1.
    Bincode1,
    /// Serialize using Postcard version 1.
    Postcard1,
}

impl BufferFormat {
    /// Returns an error if the buffer format is not enabled.
    pub fn check_available(&self) -> Result<(), DataError> {
        match self {
            #[cfg(feature = "deserialize_json")]
            BufferFormat::Json => Ok(()),

            #[cfg(feature = "deserialize_bincode_1")]
            BufferFormat::Bincode1 => Ok(()),

            #[cfg(feature = "deserialize_postcard_1")]
            BufferFormat::Postcard1 => Ok(()),

            // Allowed for cases in which all features are enabled
            #[allow(unreachable_patterns)]
            _ => Err(DataErrorKind::UnavailableBufferFormat(*self).into_error()),
        }
    }
}
