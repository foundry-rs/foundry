// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::_internal::log;
use crate::buf::BufferFormat;
use crate::prelude::*;
use core::fmt;
use displaydoc::Display;

/// A list specifying general categories of data provider error.
///
/// Errors may be caused either by a malformed request or by the data provider
/// not being able to fulfill a well-formed request.
#[derive(Clone, Copy, Eq, PartialEq, Display, Debug)]
#[non_exhaustive]
pub enum DataErrorKind {
    /// No data for the provided resource key.
    #[displaydoc("Missing data for key")]
    MissingDataKey,

    /// There is data for the key, but not for this particular locale.
    #[displaydoc("Missing data for locale")]
    MissingLocale,

    /// The request should include a locale.
    #[displaydoc("Request needs a locale")]
    NeedsLocale,

    /// The request should not contain a locale.
    #[displaydoc("Request has an extraneous locale")]
    ExtraneousLocale,

    /// The resource was blocked by a filter. The resource may or may not be available.
    #[displaydoc("Resource blocked by filter")]
    FilteredResource,

    /// The generic type parameter does not match the TypeId. The expected type name is stored
    /// as context when this error is returned.
    #[displaydoc("Mismatched types: tried to downcast with {0}, but actual type is different")]
    MismatchedType(&'static str),

    /// The payload is missing. This is usually caused by a previous error.
    #[displaydoc("Missing payload")]
    MissingPayload,

    /// A data provider object was given to an operation in an invalid state.
    #[displaydoc("Invalid state")]
    InvalidState,

    /// The syntax of the [`DataKey`] or [`DataLocale`] was invalid.
    #[displaydoc("Parse error for data key or data locale")]
    KeyLocaleSyntax,

    /// An unspecified error occurred, such as a Serde error.
    ///
    /// Check debug logs for potentially more information.
    #[displaydoc("Custom")]
    Custom,

    /// An error occurred while accessing a system resource.
    #[displaydoc("I/O error: {0:?}")]
    #[cfg(feature = "std")]
    Io(std::io::ErrorKind),

    /// An unspecified data source containing the required data is unavailable.
    #[displaydoc("Missing source data")]
    #[cfg(feature = "datagen")]
    MissingSourceData,

    /// An error indicating that the desired buffer format is not available. This usually
    /// means that a required Cargo feature was not enabled
    #[displaydoc("Unavailable buffer format: {0:?} (does icu_provider need to be compiled with an additional Cargo feature?)")]
    UnavailableBufferFormat(BufferFormat),
}

/// The error type for ICU4X data provider operations.
///
/// To create one of these, either start with a [`DataErrorKind`] or use [`DataError::custom()`].
///
/// # Example
///
/// Create a NeedsLocale error and attach a data request for context:
///
/// ```no_run
/// # use icu_provider::prelude::*;
/// let key: DataKey = unimplemented!();
/// let req: DataRequest = unimplemented!();
/// DataErrorKind::NeedsLocale.with_req(key, req);
/// ```
///
/// Create a named custom error:
///
/// ```
/// # use icu_provider::prelude::*;
/// DataError::custom("This is an example error");
/// ```
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
#[non_exhaustive]
pub struct DataError {
    /// Broad category of the error.
    pub kind: DataErrorKind,

    /// The data key of the request, if available.
    pub key: Option<DataKey>,

    /// Additional context, if available.
    pub str_context: Option<&'static str>,

    /// Whether this error was created in silent mode to not log.
    pub silent: bool,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ICU4X data error")?;
        if self.kind != DataErrorKind::Custom {
            write!(f, ": {}", self.kind)?;
        }
        if let Some(key) = self.key {
            write!(f, " (key: {key})")?;
        }
        if let Some(str_context) = self.str_context {
            write!(f, ": {str_context}")?;
        }
        Ok(())
    }
}

impl DataErrorKind {
    /// Converts this DataErrorKind into a DataError.
    ///
    /// If possible, you should attach context using a `with_` function.
    #[inline]
    pub const fn into_error(self) -> DataError {
        DataError {
            kind: self,
            key: None,
            str_context: None,
            silent: false,
        }
    }

    /// Creates a DataError with a resource key context.
    #[inline]
    pub const fn with_key(self, key: DataKey) -> DataError {
        self.into_error().with_key(key)
    }

    /// Creates a DataError with a string context.
    #[inline]
    pub const fn with_str_context(self, context: &'static str) -> DataError {
        self.into_error().with_str_context(context)
    }

    /// Creates a DataError with a type name context.
    #[inline]
    pub fn with_type_context<T>(self) -> DataError {
        self.into_error().with_type_context::<T>()
    }

    /// Creates a DataError with a request context.
    #[inline]
    pub fn with_req(self, key: DataKey, req: DataRequest) -> DataError {
        self.into_error().with_req(key, req)
    }
}

impl DataError {
    /// Returns a new, empty DataError with kind Custom and a string error message.
    #[inline]
    pub const fn custom(str_context: &'static str) -> Self {
        Self {
            kind: DataErrorKind::Custom,
            key: None,
            str_context: Some(str_context),
            silent: false,
        }
    }

    /// Sets the resource key of a DataError, returning a modified error.
    #[inline]
    pub const fn with_key(self, key: DataKey) -> Self {
        Self {
            kind: self.kind,
            key: Some(key),
            str_context: self.str_context,
            silent: self.silent,
        }
    }

    /// Sets the string context of a DataError, returning a modified error.
    #[inline]
    pub const fn with_str_context(self, context: &'static str) -> Self {
        Self {
            kind: self.kind,
            key: self.key,
            str_context: Some(context),
            silent: self.silent,
        }
    }

    /// Sets the string context of a DataError to the given type name, returning a modified error.
    #[inline]
    pub fn with_type_context<T>(self) -> Self {
        if !self.silent {
            log::warn!("{self}: Type context: {}", core::any::type_name::<T>());
        }
        self.with_str_context(core::any::type_name::<T>())
    }

    /// Logs the data error with the given request, returning an error containing the resource key.
    ///
    /// If the "logging" Cargo feature is enabled, this logs the whole request. Either way,
    /// it returns an error with the resource key portion of the request as context.
    pub fn with_req(mut self, key: DataKey, req: DataRequest) -> Self {
        if req.metadata.silent {
            self.silent = true;
        }
        // Don't write out a log for MissingDataKey since there is no context to add
        if !self.silent && self.kind != DataErrorKind::MissingDataKey {
            log::warn!("{} (key: {}, request: {})", self, key, req);
        }
        self.with_key(key)
    }

    /// Logs the data error with the given context, then return self.
    ///
    /// This does not modify the error, but if the "logging" Cargo feature is enabled,
    /// it will print out the context.
    #[cfg(feature = "std")]
    pub fn with_path_context<P: AsRef<std::path::Path> + ?Sized>(self, _path: &P) -> Self {
        if !self.silent {
            log::warn!("{} (path: {:?})", self, _path.as_ref());
        }
        self
    }

    /// Logs the data error with the given context, then return self.
    ///
    /// This does not modify the error, but if the "logging" Cargo feature is enabled,
    /// it will print out the context.
    #[cfg_attr(not(feature = "logging"), allow(unused_variables))]
    #[inline]
    pub fn with_display_context<D: fmt::Display + ?Sized>(self, context: &D) -> Self {
        if !self.silent {
            log::warn!("{}: {}", self, context);
        }
        self
    }

    /// Logs the data error with the given context, then return self.
    ///
    /// This does not modify the error, but if the "logging" Cargo feature is enabled,
    /// it will print out the context.
    #[cfg_attr(not(feature = "logging"), allow(unused_variables))]
    #[inline]
    pub fn with_debug_context<D: fmt::Debug + ?Sized>(self, context: &D) -> Self {
        if !self.silent {
            log::warn!("{}: {:?}", self, context);
        }
        self
    }

    #[inline]
    pub(crate) fn for_type<T>() -> DataError {
        DataError {
            kind: DataErrorKind::MismatchedType(core::any::type_name::<T>()),
            key: None,
            str_context: None,
            silent: false,
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DataError {}

#[cfg(feature = "std")]
impl From<std::io::Error> for DataError {
    fn from(e: std::io::Error) -> Self {
        log::warn!("I/O error: {}", e);
        DataErrorKind::Io(e.kind()).into_error()
    }
}
