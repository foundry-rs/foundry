// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::prelude::*;
use crate::DataKey;
use alloc::boxed::Box;

/// A trait that allows for converting between data payloads of different types.
///
/// These payloads will typically be some kind of erased payload, either with
/// [`AnyMarker`], [`BufferMarker`], or [`ExportMarker`](crate::datagen::ExportMarker), where converting
/// requires reifying the type.
///
/// A type implementing [`DataConverter`] will essentially have a "registry" mapping keys to
/// concrete marker types M, and reifying the input to a `DataPayload<M>`, performing some conversion
/// or computation, and erasing the result to `DataPayload<MTo>`.
pub trait DataConverter<MFrom: DataMarker, MTo: DataMarker> {
    /// Attempt to convert a payload corresponding to the given data key
    /// from one marker type to another marker type.
    ///
    /// If this is not possible (for example, if the provider does not know about the key),
    /// the original payload is returned back to the caller.
    fn convert(
        &self,
        key: DataKey,
        from: DataPayload<MFrom>,
    ) -> Result<DataPayload<MTo>, (DataPayload<MFrom>, DataError)>;
}

impl<MFrom, MTo, P> DataConverter<MFrom, MTo> for Box<P>
where
    MFrom: DataMarker,
    MTo: DataMarker,
    P: DataConverter<MFrom, MTo> + ?Sized,
{
    fn convert(
        &self,
        key: DataKey,
        from: DataPayload<MFrom>,
    ) -> Result<DataPayload<MTo>, (DataPayload<MFrom>, DataError)> {
        (**self).convert(key, from)
    }
}
