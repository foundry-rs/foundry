use crate::{ErrorPayload, RpcSend};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::value::{to_raw_value, RawValue};
use std::borrow::{Borrow, Cow};

/// A JSON-RPC 2.0 response payload.
///
/// This enum covers both the success and error cases of a JSON-RPC 2.0
/// response. It is used to represent the `result` and `error` fields of a
/// response object.
///
/// ### Note
///
/// This type does not implement `Serialize` or `Deserialize` directly. It is
/// deserialized as part of the [`Response`] type.
///
/// [`Response`]: crate::Response
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponsePayload<Payload = Box<RawValue>, ErrData = Box<RawValue>> {
    /// A successful response payload.
    Success(Payload),
    /// An error response payload.
    Failure(ErrorPayload<ErrData>),
}

/// A [`ResponsePayload`] that has been partially deserialized, borrowing its
/// contents from the deserializer. This is used primarily for intermediate
/// deserialization. Most users will not require it.
///
/// See the [top-level docs] for more info.
///
/// [top-level docs]: crate
pub type BorrowedResponsePayload<'a> = ResponsePayload<&'a RawValue, &'a RawValue>;

impl BorrowedResponsePayload<'_> {
    /// Convert this borrowed response payload into an owned payload by copying
    /// the data from the deserializer (if necessary).
    pub fn into_owned(self) -> ResponsePayload {
        match self {
            Self::Success(payload) => ResponsePayload::Success(payload.to_owned()),
            Self::Failure(error) => ResponsePayload::Failure(error.into_owned()),
        }
    }
}

impl<Payload, ErrData> ResponsePayload<Payload, ErrData> {
    /// Create a new error payload for a parse error.
    pub const fn parse_error() -> Self {
        Self::Failure(ErrorPayload::parse_error())
    }

    /// Create a new error payload for an invalid request.
    pub const fn invalid_request() -> Self {
        Self::Failure(ErrorPayload::invalid_request())
    }

    /// Create a new error payload for a method not found error.
    pub const fn method_not_found() -> Self {
        Self::Failure(ErrorPayload::method_not_found())
    }

    /// Create a new error payload for an invalid params error.
    pub const fn invalid_params() -> Self {
        Self::Failure(ErrorPayload::invalid_params())
    }

    /// Create a new error payload for an internal error.
    pub const fn internal_error() -> Self {
        Self::Failure(ErrorPayload::internal_error())
    }

    /// Create a new error payload for an internal error with a custom message.
    pub const fn internal_error_message(message: Cow<'static, str>) -> Self {
        Self::Failure(ErrorPayload::internal_error_message(message))
    }

    /// Create a new error payload for an internal error with a custom message
    /// and additional data.
    pub const fn internal_error_with_obj(data: ErrData) -> Self
    where
        ErrData: RpcSend,
    {
        Self::Failure(ErrorPayload::internal_error_with_obj(data))
    }

    /// Create a new error payload for an internal error with a custom message
    /// and additional data.
    pub const fn internal_error_with_message_and_obj(
        message: Cow<'static, str>,
        data: ErrData,
    ) -> Self
    where
        ErrData: RpcSend,
    {
        Self::Failure(ErrorPayload::internal_error_with_message_and_obj(message, data))
    }

    /// Fallible conversion to the successful payload.
    pub const fn as_success(&self) -> Option<&Payload> {
        match self {
            Self::Success(payload) => Some(payload),
            _ => None,
        }
    }

    /// Fallible conversion to the error object.
    pub const fn as_error(&self) -> Option<&ErrorPayload<ErrData>> {
        match self {
            Self::Failure(payload) => Some(payload),
            _ => None,
        }
    }

    /// Returns `true` if the response payload is a success.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    /// Returns `true` if the response payload is an error.
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Failure(_))
    }
}

impl<Payload, ErrData> ResponsePayload<Payload, ErrData>
where
    Payload: RpcSend,
    ErrData: RpcSend,
{
    /// Convert the inner types into a [`RawValue`] by serializing them.
    pub fn serialize_payload(&self) -> serde_json::Result<ResponsePayload> {
        match self {
            Self::Success(payload) => Ok(ResponsePayload::Success(to_raw_value(payload)?)),
            Self::Failure(error) => Ok(ResponsePayload::Failure(error.serialize_payload()?)),
        }
    }
}

impl<'a, Payload, ErrData> ResponsePayload<Payload, ErrData>
where
    Payload: AsRef<RawValue> + 'a,
{
    /// Attempt to deserialize the success payload, borrowing from the payload
    /// if necessary.
    ///
    /// # Returns
    /// - `None` if the payload is an error
    /// - `Some(Ok(T))` if the payload is a success and can be deserialized
    /// - `Some(Err(serde_json::Error))` if the payload is a success and can't be deserialized as
    ///   `T`
    pub fn try_success_as<T: Deserialize<'a>>(&'a self) -> Option<serde_json::Result<T>> {
        self.as_success().map(|payload| serde_json::from_str(payload.as_ref().get()))
    }

    /// Deserialize a Success payload, if possible, transforming this type.
    ///
    /// # Returns
    ///
    /// - `Ok(ResponsePayload<T>)` if the payload is an error, or if the payload is a success and
    ///   can be deserialized as `T`
    /// - `Err(self)` if the payload is a success and can't be deserialized
    pub fn deserialize_success<T: DeserializeOwned>(
        self,
    ) -> Result<ResponsePayload<T, ErrData>, Self> {
        match self {
            Self::Success(ref payload) => serde_json::from_str(payload.as_ref().get())
                .map_or_else(|_| Err(self), |payload| Ok(ResponsePayload::Success(payload))),
            Self::Failure(e) => Ok(ResponsePayload::Failure(e)),
        }
    }
}

impl<'a, Payload, Data> ResponsePayload<Payload, Data>
where
    Data: Borrow<RawValue> + 'a,
{
    /// Attempt to deserialize the error payload, borrowing from the payload if
    /// necessary.
    ///
    /// # Returns
    /// - `None` if the payload is a success
    /// - `Some(Ok(T))` if the payload is an error and can be deserialized
    /// - `Some(Err(serde_json::Error))` if the payload is an error and can't be deserialized as `T`
    pub fn try_error_as<T: Deserialize<'a>>(&'a self) -> Option<serde_json::Result<T>> {
        self.as_error().and_then(|error| error.try_data_as::<T>())
    }

    /// Deserialize an Error payload, if possible, transforming this type.
    ///
    /// # Returns
    ///
    /// - `Ok(ResponsePayload<Payload, T>)` if the payload is an error, or if the payload is an
    ///   error and can be deserialized as `T`.
    /// - `Err(self)` if the payload is an error and can't be deserialized.
    pub fn deserialize_error<T: DeserializeOwned>(
        self,
    ) -> Result<ResponsePayload<Payload, T>, Self> {
        match self {
            Self::Failure(err) => match err.deser_data() {
                Ok(deser) => Ok(ResponsePayload::Failure(deser)),
                Err(err) => Err(Self::Failure(err)),
            },
            Self::Success(payload) => Ok(ResponsePayload::Success(payload)),
        }
    }
}
