use alloy_primitives::Bytes;
use alloy_sol_types::SolInterface;
use serde::{
    de::{DeserializeOwned, MapAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{
    value::{to_raw_value, RawValue},
    Value,
};
use std::{
    borrow::{Borrow, Cow},
    fmt,
    marker::PhantomData,
};

use crate::RpcSend;

const INTERNAL_ERROR: Cow<'static, str> = Cow::Borrowed("Internal error");

/// A JSON-RPC 2.0 error object.
///
/// This response indicates that the server received and handled the request,
/// but that there was an error in the processing of it. The error should be
/// included in the `message` field of the response payload.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ErrorPayload<ErrData = Box<RawValue>> {
    /// The error code.
    pub code: i64,
    /// The error message (if any).
    pub message: Cow<'static, str>,
    /// The error data (if any).
    pub data: Option<ErrData>,
}

impl<E> ErrorPayload<E> {
    /// Create a new error payload for a parse error.
    pub const fn parse_error() -> Self {
        Self { code: -32700, message: Cow::Borrowed("Parse error"), data: None }
    }

    /// Create a new error payload for an invalid request.
    pub const fn invalid_request() -> Self {
        Self { code: -32600, message: Cow::Borrowed("Invalid Request"), data: None }
    }

    /// Create a new error payload for a method not found error.
    pub const fn method_not_found() -> Self {
        Self { code: -32601, message: Cow::Borrowed("Method not found"), data: None }
    }

    /// Create a new error payload for an invalid params error.
    pub const fn invalid_params() -> Self {
        Self { code: -32602, message: Cow::Borrowed("Invalid params"), data: None }
    }

    /// Create a new error payload for an internal error.
    pub const fn internal_error() -> Self {
        Self { code: -32603, message: INTERNAL_ERROR, data: None }
    }

    /// Create a new error payload for an internal error with a custom message.
    pub const fn internal_error_message(message: Cow<'static, str>) -> Self {
        Self { code: -32603, message, data: None }
    }

    /// Create a new error payload for an internal error with a custom message
    /// and additional data.
    pub const fn internal_error_with_obj(data: E) -> Self
    where
        E: RpcSend,
    {
        Self { code: -32603, message: INTERNAL_ERROR, data: Some(data) }
    }

    /// Create a new error payload for an internal error with a custom message
    pub const fn internal_error_with_message_and_obj(message: Cow<'static, str>, data: E) -> Self
    where
        E: RpcSend,
    {
        Self { code: -32603, message, data: Some(data) }
    }

    /// Analyzes the [ErrorPayload] and decides if the request should be
    /// retried based on the error code or the message.
    pub fn is_retry_err(&self) -> bool {
        // alchemy throws it this way
        if self.code == 429 {
            return true;
        }

        // This is an infura error code for `exceeded project rate limit`
        if self.code == -32005 {
            return true;
        }

        // alternative alchemy error for specific IPs
        if self.code == -32016 && self.message.contains("rate limit") {
            return true;
        }

        // quick node error `"credits limited to 6000/sec"`
        // <https://github.com/foundry-rs/foundry/pull/6712#issuecomment-1951441240>
        if self.code == -32012 && self.message.contains("credits") {
            return true;
        }

        // quick node rate limit error: `100/second request limit reached - reduce calls per second
        // or upgrade your account at quicknode.com` <https://github.com/foundry-rs/foundry/issues/4894>
        if self.code == -32007 && self.message.contains("request limit reached") {
            return true;
        }

        match self.message.as_ref() {
            // this is commonly thrown by infura and is apparently a load balancer issue, see also <https://github.com/MetaMask/metamask-extension/issues/7234>
            "header not found" => true,
            // also thrown by infura if out of budget for the day and ratelimited
            "daily request count exceeded, request rate limited" => true,
            msg => {
                msg.contains("rate limit")
                    || msg.contains("rate exceeded")
                    || msg.contains("too many requests")
                    || msg.contains("credits limited")
                    || msg.contains("request limit")
            }
        }
    }
}

impl<T> From<T> for ErrorPayload<T>
where
    T: std::error::Error + RpcSend,
{
    fn from(value: T) -> Self {
        Self { code: -32603, message: INTERNAL_ERROR, data: Some(value) }
    }
}

impl<E> ErrorPayload<E>
where
    E: RpcSend,
{
    /// Serialize the inner data into a [`RawValue`].
    pub fn serialize_payload(&self) -> serde_json::Result<ErrorPayload> {
        Ok(ErrorPayload {
            code: self.code,
            message: self.message.clone(),
            data: match self.data.as_ref() {
                Some(data) => Some(to_raw_value(data)?),
                None => None,
            },
        })
    }
}

/// Recursively traverses the value, looking for hex data that it can extract.
///
/// Inspired by ethers-js logic:
/// <https://github.com/ethers-io/ethers.js/blob/9f990c57f0486728902d4b8e049536f2bb3487ee/packages/providers/src.ts/json-rpc-provider.ts#L25-L53>
fn spelunk_revert(value: &Value) -> Option<Bytes> {
    match value {
        Value::String(s) => s.parse().ok(),
        Value::Object(o) => o.values().find_map(spelunk_revert),
        _ => None,
    }
}

impl<ErrData: fmt::Display> fmt::Display for ErrorPayload<ErrData> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error code {}: {}{}",
            self.code,
            self.message,
            self.data.as_ref().map(|data| format!(", data: {}", data)).unwrap_or_default()
        )
    }
}

/// A [`ErrorPayload`] that has been partially deserialized, borrowing its
/// contents from the deserializer. This is used primarily for intermediate
/// deserialization. Most users will not require it.
///
/// See the [top-level docs] for more info.
///
/// [top-level docs]: crate
pub type BorrowedErrorPayload<'a> = ErrorPayload<&'a RawValue>;

impl BorrowedErrorPayload<'_> {
    /// Convert this borrowed error payload into an owned payload by copying
    /// the data from the deserializer (if necessary).
    pub fn into_owned(self) -> ErrorPayload {
        ErrorPayload {
            code: self.code,
            message: self.message,
            data: self.data.map(|data| data.to_owned()),
        }
    }
}

impl<'de, ErrData: Deserialize<'de>> Deserialize<'de> for ErrorPayload<ErrData> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Code,
            Message,
            Data,
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
                        formatter.write_str("`code`, `message` and `data`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "code" => Ok(Field::Code),
                            "message" => Ok(Field::Message),
                            "data" => Ok(Field::Data),
                            _ => Ok(Field::Unknown),
                        }
                    }
                }
                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct ErrorPayloadVisitor<T>(PhantomData<T>);

        impl<'de, Data> Visitor<'de> for ErrorPayloadVisitor<Data>
        where
            Data: Deserialize<'de>,
        {
            type Value = ErrorPayload<Data>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a JSON-RPC 2.0 error object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut code = None;
                let mut message = None;
                let mut data = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Code => {
                            if code.is_some() {
                                return Err(serde::de::Error::duplicate_field("code"));
                            }
                            code = Some(map.next_value()?);
                        }
                        Field::Message => {
                            if message.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message = Some(map.next_value()?);
                        }
                        Field::Data => {
                            if data.is_some() {
                                return Err(serde::de::Error::duplicate_field("data"));
                            }
                            data = Some(map.next_value()?);
                        }
                        Field::Unknown => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                            // ignore
                        }
                    }
                }
                Ok(ErrorPayload {
                    code: code.ok_or_else(|| serde::de::Error::missing_field("code"))?,
                    message: message.unwrap_or_default(),
                    data,
                })
            }
        }

        deserializer.deserialize_any(ErrorPayloadVisitor(PhantomData))
    }
}

impl<'a, Data> ErrorPayload<Data>
where
    Data: Borrow<RawValue> + 'a,
{
    /// Deserialize the error's `data` field, borrowing from the data field if
    /// necessary.
    ///
    /// # Returns
    ///
    /// - `None` if the error has no `data` field.
    /// - `Some(Ok(data))` if the error has a `data` field that can be deserialized.
    /// - `Some(Err(err))` if the error has a `data` field that can't be deserialized.
    pub fn try_data_as<T: Deserialize<'a>>(&'a self) -> Option<serde_json::Result<T>> {
        self.data.as_ref().map(|data| serde_json::from_str(data.borrow().get()))
    }

    /// Attempt to deserialize the data field.
    ///
    /// # Returns
    ///
    /// - `Ok(ErrorPayload<T>)` if the data field can be deserialized
    /// - `Err(self)` if the data field can't be deserialized, or if there is no data field.
    pub fn deser_data<T: DeserializeOwned>(self) -> Result<ErrorPayload<T>, Self> {
        match self.try_data_as::<T>() {
            Some(Ok(data)) => {
                Ok(ErrorPayload { code: self.code, message: self.message, data: Some(data) })
            }
            _ => Err(self),
        }
    }

    /// Attempt to extract revert data from the JsonRpcError be recursively
    /// traversing the error's data field
    ///
    /// This returns the first hex it finds in the data object, and its
    /// behavior may change with `serde_json` internal changes.
    ///
    /// If no hex object is found, it will return an empty bytes IFF the error
    /// is a revert
    ///
    /// Inspired by ethers-js logic:
    /// <https://github.com/ethers-io/ethers.js/blob/9f990c57f0486728902d4b8e049536f2bb3487ee/packages/providers/src.ts/json-rpc-provider.ts#L25-L53>
    pub fn as_revert_data(&self) -> Option<Bytes> {
        if self.message.contains("revert") {
            let value = Value::deserialize(self.data.as_ref()?.borrow()).ok()?;
            spelunk_revert(&value)
        } else {
            None
        }
    }

    /// Extracts revert data and tries decoding it into given custom errors set.
    pub fn as_decoded_error<E: SolInterface>(&self, validate: bool) -> Option<E> {
        self.as_revert_data().and_then(|data| E::abi_decode(&data, validate).ok())
    }
}

#[cfg(test)]
mod test {
    use alloy_primitives::U256;
    use alloy_sol_types::sol;

    use super::BorrowedErrorPayload;
    use crate::ErrorPayload;

    #[test]
    fn smooth_borrowing() {
        let json = r#"{ "code": -32000, "message": "b", "data": null }"#;
        let payload: BorrowedErrorPayload<'_> = serde_json::from_str(json).unwrap();

        assert_eq!(payload.code, -32000);
        assert_eq!(payload.message, "b");
        assert_eq!(payload.data.unwrap().get(), "null");
    }

    #[test]
    fn smooth_deser() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct TestData {
            a: u32,
            b: Option<String>,
        }

        let json = r#"{ "code": -32000, "message": "b", "data": { "a": 5, "b": null } }"#;

        let payload: BorrowedErrorPayload<'_> = serde_json::from_str(json).unwrap();
        let data: TestData = payload.try_data_as().unwrap().unwrap();
        assert_eq!(data, TestData { a: 5, b: None });
    }

    #[test]
    fn missing_data() {
        let json = r#"{"code":-32007,"message":"20/second request limit reached - reduce calls per second or upgrade your account at quicknode.com"}"#;
        let payload: ErrorPayload = serde_json::from_str(json).unwrap();

        assert_eq!(payload.code, -32007);
        assert_eq!(payload.message, "20/second request limit reached - reduce calls per second or upgrade your account at quicknode.com");
        assert!(payload.data.is_none());
    }

    #[test]
    fn custom_error_decoding() {
        sol!(
            library Errors {
                error SomeCustomError(uint256 a);
            }
        );

        let json = r#"{"code":3,"message":"execution reverted: ","data":"0x810f00230000000000000000000000000000000000000000000000000000000000000001"}"#;
        let payload: ErrorPayload = serde_json::from_str(json).unwrap();

        let Errors::ErrorsErrors::SomeCustomError(value) =
            payload.as_decoded_error::<Errors::ErrorsErrors>(false).unwrap();

        assert_eq!(value.a, U256::from(1));
    }
}
