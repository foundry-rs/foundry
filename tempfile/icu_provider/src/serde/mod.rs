// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Provides the [`DeserializingBufferProvider`] wrapper, which deserializes data using Serde.
//!
//! Providers that produce opaque buffers that need to be deserialized into concrete data structs,
//! such as `FsDataProvider`, should implement [`BufferProvider`]. These can be converted into
//! [`DeserializingBufferProvider`] using the [`as_deserializing`](AsDeserializingBufferProvider::as_deserializing)
//! convenience method.
//!
//! [`BufferProvider`]: crate::buf::BufferProvider

// Hidden for now, but could be made public-stable in the future.
#[doc(hidden)]
pub mod borrow_de_utils;

use crate::buf::BufferFormat;
use crate::buf::BufferProvider;
use crate::prelude::*;
use serde::de::Deserialize;
use yoke::trait_hack::YokeTraitHack;
use yoke::Yokeable;

/// A [`BufferProvider`] that deserializes its data using Serde.
#[derive(Debug)]
pub struct DeserializingBufferProvider<'a, P: ?Sized>(&'a P);

/// Blanket-implemented trait adding the [`Self::as_deserializing()`] function.
pub trait AsDeserializingBufferProvider {
    /// Wrap this [`BufferProvider`] in a [`DeserializingBufferProvider`].
    ///
    /// This requires enabling the deserialization Cargo feature
    /// for the expected format(s):
    ///
    /// - `deserialize_json`
    /// - `deserialize_postcard_1`
    /// - `deserialize_bincode_1`
    fn as_deserializing(&self) -> DeserializingBufferProvider<Self>;
}

impl<P> AsDeserializingBufferProvider for P
where
    P: BufferProvider + ?Sized,
{
    /// Wrap this [`BufferProvider`] in a [`DeserializingBufferProvider`].
    ///
    /// This requires enabling the deserialization Cargo feature
    /// for the expected format(s):
    ///
    /// - `deserialize_json`
    /// - `deserialize_postcard_1`
    /// - `deserialize_bincode_1`
    fn as_deserializing(&self) -> DeserializingBufferProvider<Self> {
        DeserializingBufferProvider(self)
    }
}

fn deserialize_impl<'data, M>(
    // Allow `bytes` to be unused in case all buffer formats are disabled
    #[allow(unused_variables)] bytes: &'data [u8],
    buffer_format: BufferFormat,
) -> Result<<M::Yokeable as Yokeable<'data>>::Output, DataError>
where
    M: DataMarker,
    // Actual bound:
    //     for<'de> <M::Yokeable as Yokeable<'de>>::Output: Deserialize<'de>,
    // Necessary workaround bound (see `yoke::trait_hack` docs):
    for<'de> YokeTraitHack<<M::Yokeable as Yokeable<'de>>::Output>: Deserialize<'de>,
{
    match buffer_format {
        #[cfg(feature = "deserialize_json")]
        BufferFormat::Json => {
            let mut d = serde_json::Deserializer::from_slice(bytes);
            let data = YokeTraitHack::<<M::Yokeable as Yokeable>::Output>::deserialize(&mut d)?;
            Ok(data.0)
        }

        #[cfg(feature = "deserialize_bincode_1")]
        BufferFormat::Bincode1 => {
            use bincode::Options;
            let options = bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .allow_trailing_bytes();
            let mut d = bincode::de::Deserializer::from_slice(bytes, options);
            let data = YokeTraitHack::<<M::Yokeable as Yokeable>::Output>::deserialize(&mut d)?;
            Ok(data.0)
        }

        #[cfg(feature = "deserialize_postcard_1")]
        BufferFormat::Postcard1 => {
            let mut d = postcard::Deserializer::from_bytes(bytes);
            let data = YokeTraitHack::<<M::Yokeable as Yokeable>::Output>::deserialize(&mut d)?;
            Ok(data.0)
        }

        // Allowed for cases in which all features are enabled
        #[allow(unreachable_patterns)]
        _ => Err(DataErrorKind::UnavailableBufferFormat(buffer_format).into_error()),
    }
}

impl DataPayload<BufferMarker> {
    /// Deserialize a [`DataPayload`]`<`[`BufferMarker`]`>` into a [`DataPayload`] of a
    /// specific concrete type.
    ///
    /// This requires enabling the deserialization Cargo feature
    /// for the expected format(s):
    ///
    /// - `deserialize_json`
    /// - `deserialize_postcard_1`
    /// - `deserialize_bincode_1`
    ///
    /// This function takes the buffer format as an argument. When a buffer payload is returned
    /// from a data provider, the buffer format is stored in the [`DataResponseMetadata`].
    ///
    /// # Examples
    ///
    /// Requires the `deserialize_json` Cargo feature:
    ///
    /// ```
    /// use icu_provider::buf::BufferFormat;
    /// use icu_provider::hello_world::*;
    /// use icu_provider::prelude::*;
    ///
    /// let buffer: &[u8] = br#"{"message":"Hallo Welt"}"#;
    ///
    /// let buffer_payload = DataPayload::from_owned(buffer);
    /// let payload: DataPayload<HelloWorldV1Marker> = buffer_payload
    ///     .into_deserialized(BufferFormat::Json)
    ///     .expect("Deserialization successful");
    ///
    /// assert_eq!(payload.get().message, "Hallo Welt");
    /// ```
    pub fn into_deserialized<M>(
        self,
        buffer_format: BufferFormat,
    ) -> Result<DataPayload<M>, DataError>
    where
        M: DataMarker,
        // Actual bound:
        //     for<'de> <M::Yokeable as Yokeable<'de>>::Output: Deserialize<'de>,
        // Necessary workaround bound (see `yoke::trait_hack` docs):
        for<'de> YokeTraitHack<<M::Yokeable as Yokeable<'de>>::Output>: Deserialize<'de>,
    {
        self.try_map_project(|bytes, _| deserialize_impl::<M>(bytes, buffer_format))
    }
}

impl<P, M> DynamicDataProvider<M> for DeserializingBufferProvider<'_, P>
where
    M: DataMarker,
    P: BufferProvider + ?Sized,
    // Actual bound:
    //     for<'de> <M::Yokeable as Yokeable<'de>>::Output: serde::de::Deserialize<'de>,
    // Necessary workaround bound (see `yoke::trait_hack` docs):
    for<'de> YokeTraitHack<<M::Yokeable as Yokeable<'de>>::Output>: Deserialize<'de>,
{
    /// Converts a buffer into a concrete type by deserializing from a supported buffer format.
    ///
    /// This requires enabling the deserialization Cargo feature
    /// for the expected format(s):
    ///
    /// - `deserialize_json`
    /// - `deserialize_postcard_1`
    /// - `deserialize_bincode_1`
    fn load_data(&self, key: DataKey, req: DataRequest) -> Result<DataResponse<M>, DataError> {
        let buffer_response = BufferProvider::load_buffer(self.0, key, req)?;
        let buffer_format = buffer_response.metadata.buffer_format.ok_or_else(|| {
            DataError::custom("BufferProvider didn't set BufferFormat").with_req(key, req)
        })?;
        Ok(DataResponse {
            metadata: buffer_response.metadata,
            payload: buffer_response
                .payload
                .map(|p| p.into_deserialized(buffer_format))
                .transpose()
                .map_err(|e| e.with_req(key, req))?,
        })
    }
}

impl<P, M> DataProvider<M> for DeserializingBufferProvider<'_, P>
where
    M: KeyedDataMarker,
    P: BufferProvider + ?Sized,
    // Actual bound:
    //     for<'de> <M::Yokeable as Yokeable<'de>>::Output: Deserialize<'de>,
    // Necessary workaround bound (see `yoke::trait_hack` docs):
    for<'de> YokeTraitHack<<M::Yokeable as Yokeable<'de>>::Output>: Deserialize<'de>,
{
    /// Converts a buffer into a concrete type by deserializing from a supported buffer format.
    ///
    /// This requires enabling the deserialization Cargo feature
    /// for the expected format(s):
    ///
    /// - `deserialize_json`
    /// - `deserialize_postcard_1`
    /// - `deserialize_bincode_1`
    fn load(&self, req: DataRequest) -> Result<DataResponse<M>, DataError> {
        self.load_data(M::KEY, req)
    }
}

#[cfg(feature = "deserialize_json")]
impl From<serde_json::error::Error> for crate::DataError {
    fn from(e: serde_json::error::Error) -> Self {
        crate::DataError::custom("JSON deserialize").with_display_context(&e)
    }
}

#[cfg(feature = "deserialize_bincode_1")]
impl From<bincode::Error> for crate::DataError {
    fn from(e: bincode::Error) -> Self {
        crate::DataError::custom("Bincode deserialize").with_display_context(&e)
    }
}

#[cfg(feature = "deserialize_postcard_1")]
impl From<postcard::Error> for crate::DataError {
    fn from(e: postcard::Error) -> Self {
        crate::DataError::custom("Postcard deserialize").with_display_context(&e)
    }
}
