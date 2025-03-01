use crate::{common::Id, RpcSend};
use serde::{
    de::{DeserializeOwned, MapAccess, Visitor},
    ser::SerializeMap,
    Deserialize, Deserializer, Serialize,
};
use serde_json::value::RawValue;
use std::{
    borrow::{Borrow, Cow},
    fmt,
    marker::PhantomData,
};

mod error;
pub use error::{BorrowedErrorPayload, ErrorPayload};

mod payload;
pub use payload::{BorrowedResponsePayload, ResponsePayload};

/// A JSON-RPC 2.0 response object containing a [`ResponsePayload`].
///
/// This object is used to represent a JSON-RPC 2.0 response. It may contain
/// either a successful result or an error. The `id` field is used to match
/// the response to the request that it is responding to, and should be
/// mirrored from the response.
#[derive(Clone, Debug)]
pub struct Response<Payload = Box<RawValue>, ErrData = Box<RawValue>> {
    /// The ID of the request that this response is responding to.
    pub id: Id,
    /// The response payload.
    pub payload: ResponsePayload<Payload, ErrData>,
}

/// A [`Response`] that has been partially deserialized, borrowing its contents
/// from the deserializer. This is used primarily for intermediate
/// deserialization. Most users will not require it.
///
/// See the [top-level docs] for more info.
///
/// [top-level docs]: crate
pub type BorrowedResponse<'a> = Response<&'a RawValue, &'a RawValue>;

impl BorrowedResponse<'_> {
    /// Convert this borrowed response to an owned response by copying the data
    /// from the deserializer (if necessary).
    pub fn into_owned(self) -> Response {
        Response { id: self.id.clone(), payload: self.payload.into_owned() }
    }
}

impl<Payload, ErrData> Response<Payload, ErrData> {
    /// Create a new response with a parsed error payload.
    pub const fn parse_error(id: Id) -> Self {
        Self { id, payload: ResponsePayload::parse_error() }
    }

    /// Create a new response with an invalid request error payload.
    pub const fn invalid_request(id: Id) -> Self {
        Self { id, payload: ResponsePayload::invalid_request() }
    }

    /// Create a new response with a method not found error payload.
    pub const fn method_not_found(id: Id) -> Self {
        Self { id, payload: ResponsePayload::method_not_found() }
    }

    /// Create a new response with an invalid params error payload.
    pub const fn invalid_params(id: Id) -> Self {
        Self { id, payload: ResponsePayload::invalid_params() }
    }

    /// Create a new response with an internal error payload.
    pub const fn internal_error(id: Id) -> Self {
        Self { id, payload: ResponsePayload::internal_error() }
    }

    /// Create a new error response for an internal error with a custom message.
    pub const fn internal_error_message(id: Id, message: Cow<'static, str>) -> Self {
        Self {
            id,
            payload: ResponsePayload::Failure(ErrorPayload::internal_error_message(message)),
        }
    }

    /// Create a new error response for an internal error with additional data.
    pub const fn internal_error_with_obj(id: Id, data: ErrData) -> Self
    where
        ErrData: RpcSend,
    {
        Self { id, payload: ResponsePayload::Failure(ErrorPayload::internal_error_with_obj(data)) }
    }

    /// Create a new error response for an internal error with a custom message
    /// and additional data.
    pub const fn internal_error_with_message_and_obj(
        id: Id,
        message: Cow<'static, str>,
        data: ErrData,
    ) -> Self
    where
        ErrData: RpcSend,
    {
        Self {
            id,
            payload: ResponsePayload::Failure(ErrorPayload::internal_error_with_message_and_obj(
                message, data,
            )),
        }
    }

    /// Returns `true` if the response is a success.
    pub const fn is_success(&self) -> bool {
        self.payload.is_success()
    }

    /// Returns `true` if the response is an error.
    pub const fn is_error(&self) -> bool {
        self.payload.is_error()
    }
}

impl<Payload, ErrData> Response<Payload, ErrData>
where
    Payload: RpcSend,
    ErrData: RpcSend,
{
    /// Serialize the payload of this response.
    pub fn serialize_payload(&self) -> serde_json::Result<Response> {
        self.payload.serialize_payload().map(|payload| Response { id: self.id.clone(), payload })
    }
}

impl<'a, Payload, ErrData> Response<Payload, ErrData>
where
    Payload: AsRef<RawValue> + 'a,
{
    /// Attempt to deserialize the success payload, borrowing from the payload
    /// if necessary.
    ///
    /// See [`ResponsePayload::try_success_as`].
    pub fn try_success_as<T: Deserialize<'a>>(&'a self) -> Option<serde_json::Result<T>> {
        self.payload.try_success_as()
    }

    /// Attempt to deserialize the Success payload, transforming this type.
    ///
    /// # Returns
    ///
    /// - `Ok(Response<T, ErrData>)` if the payload is a success and can be deserialized as T, or if
    ///   the payload is an error.
    /// - `Err(self)` if the payload is a success and can't be deserialized.
    pub fn deser_success<T: DeserializeOwned>(self) -> Result<Response<T, ErrData>, Self> {
        match self.payload.deserialize_success() {
            Ok(payload) => Ok(Response { id: self.id, payload }),
            Err(payload) => Err(Self { id: self.id, payload }),
        }
    }
}

impl<'a, Payload, ErrData> Response<Payload, ErrData>
where
    ErrData: Borrow<RawValue> + 'a,
{
    /// Attempt to deserialize the error payload, borrowing from the payload if
    /// necessary.
    ///
    /// See [`ResponsePayload::try_error_as`].
    pub fn try_error_as<T: Deserialize<'a>>(&'a self) -> Option<serde_json::Result<T>> {
        self.payload.try_error_as()
    }

    /// Attempt to deserialize the Error payload, transforming this type.
    ///
    /// # Returns
    ///
    /// - `Ok(Response<Payload, T>)` if the payload is an error and can be deserialized as `T`, or
    ///   if the payload is a success.
    /// - `Err(self)` if the payload is an error and can't be deserialized.
    pub fn deser_err<T: DeserializeOwned>(self) -> Result<Response<Payload, T>, Self> {
        match self.payload.deserialize_error() {
            Ok(payload) => Ok(Response { id: self.id, payload }),
            Err(payload) => Err(Self { id: self.id, payload }),
        }
    }
}

impl<'de, Payload, ErrData> Deserialize<'de> for Response<Payload, ErrData>
where
    Payload: Deserialize<'de>,
    ErrData: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        enum Field {
            Result,
            Error,
            Id,
            Unknown,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl serde::de::Visitor<'_> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("`result`, `error` and `id`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "result" => Ok(Field::Result),
                            "error" => Ok(Field::Error),
                            "id" => Ok(Field::Id),
                            _ => Ok(Field::Unknown),
                        }
                    }
                }
                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct JsonRpcResponseVisitor<T>(PhantomData<T>);

        impl<'de, Payload, ErrData> Visitor<'de> for JsonRpcResponseVisitor<fn() -> (Payload, ErrData)>
        where
            Payload: Deserialize<'de>,
            ErrData: Deserialize<'de>,
        {
            type Value = Response<Payload, ErrData>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "a JSON-RPC response object, consisting of either a result or an error",
                )
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut result = None;
                let mut error = None;
                let mut id: Option<Id> = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Result => {
                            if result.is_some() {
                                return Err(serde::de::Error::duplicate_field("result"));
                            }
                            result = Some(map.next_value()?);
                        }
                        Field::Error => {
                            if error.is_some() {
                                return Err(serde::de::Error::duplicate_field("error"));
                            }
                            error = Some(map.next_value()?);
                        }
                        Field::Id => {
                            if id.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id = Some(map.next_value()?);
                        }
                        Field::Unknown => {
                            let _: serde::de::IgnoredAny = map.next_value()?; // ignore
                        }
                    }
                }
                let id = id.unwrap_or(Id::None);

                match (result, error) {
                    (Some(result), None) => {
                        Ok(Response { id, payload: ResponsePayload::Success(result) })
                    }
                    (None, Some(error)) => {
                        Ok(Response { id, payload: ResponsePayload::Failure(error) })
                    }
                    (None, None) => Err(serde::de::Error::missing_field("result or error")),
                    (Some(_), Some(_)) => {
                        Err(serde::de::Error::custom("result and error are mutually exclusive"))
                    }
                }
            }
        }

        deserializer.deserialize_map(JsonRpcResponseVisitor(PhantomData))
    }
}

impl<Payload, ErrData> Serialize for Response<Payload, ErrData>
where
    Payload: Serialize,
    ErrData: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("jsonrpc", "2.0")?;
        map.serialize_entry("id", &self.id)?;
        match &self.payload {
            ResponsePayload::Success(result) => {
                map.serialize_entry("result", result)?;
            }
            ResponsePayload::Failure(error) => {
                map.serialize_entry("error", error)?;
            }
        }
        map.end()
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn deser_success() {
        let response = r#"{
            "jsonrpc": "2.0",
            "result": "california",
            "id": 1
        }"#;
        let response: super::Response = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, super::Id::Number(1));
        assert!(matches!(response.payload, super::ResponsePayload::Success(_)));
    }

    #[test]
    fn deser_err() {
        let response = r#"{
            "jsonrpc": "2.0",
            "error": {
                "code": -32600,
                "message": "Invalid Request"
            },
            "id": null
        }"#;
        let response: super::Response = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, super::Id::None);
        assert!(matches!(response.payload, super::ResponsePayload::Failure(_)));
    }

    #[test]
    fn deser_complex_success() {
        let response = r#"{
            "result": {
                "name": "california",
                "population": 39250000,
                "cities": [
                    "los angeles",
                    "san francisco"
                ]
            }
        }"#;
        let response: super::Response = serde_json::from_str(response).unwrap();
        assert_eq!(response.id, super::Id::None);
        assert!(matches!(response.payload, super::ResponsePayload::Success(_)));
    }
}

// Copyright 2019-2021 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.
