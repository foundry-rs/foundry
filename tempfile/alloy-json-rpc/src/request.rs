use crate::{common::Id, RpcBorrow, RpcSend};
use alloy_primitives::{keccak256, B256};
use serde::{
    de::{DeserializeOwned, MapAccess},
    ser::SerializeMap,
    Deserialize, Serialize,
};
use serde_json::value::RawValue;
use std::{borrow::Cow, marker::PhantomData, mem::MaybeUninit};

/// `RequestMeta` contains the [`Id`] and method name of a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestMeta {
    /// The method name.
    pub method: Cow<'static, str>,
    /// The request ID.
    pub id: Id,
    /// Whether the request is a subscription, other than `eth_subscribe`.
    is_subscription: bool,
}

impl RequestMeta {
    /// Create a new `RequestMeta`.
    pub const fn new(method: Cow<'static, str>, id: Id) -> Self {
        Self { method, id, is_subscription: false }
    }

    /// Returns `true` if the request is a subscription.
    pub fn is_subscription(&self) -> bool {
        self.is_subscription || self.method == "eth_subscribe"
    }

    /// Indicates that the request is a non-standard subscription (i.e. not
    /// "eth_subscribe").
    pub fn set_is_subscription(&mut self) {
        self.set_subscription_status(true);
    }

    /// Setter for `is_subscription`. Indicates to RPC clients that the request
    /// triggers a stream of notifications.
    pub fn set_subscription_status(&mut self, sub: bool) {
        self.is_subscription = sub;
    }
}

/// A JSON-RPC 2.0 request object.
///
/// This is a generic type that can be used to represent any JSON-RPC request.
/// The `Params` type parameter is used to represent the parameters of the
/// request, and the `method` field is used to represent the method name.
///
/// ### Note
///
/// The value of `method` should be known at compile time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request<Params> {
    /// The request metadata (ID and method).
    pub meta: RequestMeta,
    /// The request parameters.
    pub params: Params,
}

impl<Params> Request<Params> {
    /// Create a new `Request`.
    pub fn new(method: impl Into<Cow<'static, str>>, id: Id, params: Params) -> Self {
        Self { meta: RequestMeta::new(method.into(), id), params }
    }

    /// Returns `true` if the request is a subscription.
    pub fn is_subscription(&self) -> bool {
        self.meta.is_subscription()
    }

    /// Indicates that the request is a non-standard subscription (i.e. not
    /// "eth_subscribe").
    pub fn set_is_subscription(&mut self) {
        self.meta.set_is_subscription()
    }

    /// Setter for `is_subscription`. Indicates to RPC clients that the request
    /// triggers a stream of notifications.
    pub fn set_subscription_status(&mut self, sub: bool) {
        self.meta.set_subscription_status(sub);
    }

    /// Change type of the request parameters.
    pub fn map_params<NewParams>(
        self,
        map: impl FnOnce(Params) -> NewParams,
    ) -> Request<NewParams> {
        Request { meta: self.meta, params: map(self.params) }
    }
}

/// A [`Request`] that has been partially serialized.
///
/// The request parameters have been serialized, and are represented as a boxed [`RawValue`]. This
/// is useful for collections containing many requests, as it erases the `Param` type. It can be
/// created with [`Request::box_params()`].
///
/// See the [top-level docs] for more info.
///
/// [top-level docs]: crate
pub type PartiallySerializedRequest = Request<Box<RawValue>>;

impl<Params> Request<Params>
where
    Params: RpcSend,
{
    /// Serialize the request parameters as a boxed [`RawValue`].
    ///
    /// # Panics
    ///
    /// If serialization of the params fails.
    pub fn box_params(self) -> PartiallySerializedRequest {
        Request { meta: self.meta, params: serde_json::value::to_raw_value(&self.params).unwrap() }
    }

    /// Serialize the request, including the request parameters.
    pub fn serialize(self) -> serde_json::Result<SerializedRequest> {
        let request = serde_json::value::to_raw_value(&self)?;
        Ok(SerializedRequest { meta: self.meta, request })
    }
}

impl<Params> Request<&Params>
where
    Params: ToOwned,
    Params::Owned: RpcSend,
{
    /// Clone the request, including the request parameters.
    pub fn into_owned_params(self) -> Request<Params::Owned> {
        Request { meta: self.meta, params: self.params.to_owned() }
    }
}

impl<'a, Params> Request<Params>
where
    Params: AsRef<RawValue> + 'a,
{
    /// Attempt to deserialize the params.
    ///
    /// To borrow from the params via the deserializer, use
    /// [`Request::try_borrow_params_as`].
    ///
    /// # Returns
    /// - `Ok(T)` if the params can be deserialized as `T`
    /// - `Err(e)` if the params cannot be deserialized as `T`
    pub fn try_params_as<T: DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_str(self.params.as_ref().get())
    }

    /// Attempt to deserialize the params, borrowing from the params
    ///
    /// # Returns
    /// - `Ok(T)` if the params can be deserialized as `T`
    /// - `Err(e)` if the params cannot be deserialized as `T`
    pub fn try_borrow_params_as<T: Deserialize<'a>>(&'a self) -> serde_json::Result<T> {
        serde_json::from_str(self.params.as_ref().get())
    }
}

// manually implemented to avoid adding a type for the protocol-required
// `jsonrpc` field
impl<Params> Serialize for Request<Params>
where
    Params: RpcSend,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let sized_params = std::mem::size_of::<Params>() != 0;

        let mut map = serializer.serialize_map(Some(3 + sized_params as usize))?;
        map.serialize_entry("method", &self.meta.method[..])?;

        // Params may be omitted if it is 0-sized
        if sized_params {
            map.serialize_entry("params", &self.params)?;
        }

        map.serialize_entry("id", &self.meta.id)?;
        map.serialize_entry("jsonrpc", "2.0")?;
        map.end()
    }
}

impl<'de, Params> Deserialize<'de> for Request<Params>
where
    Params: RpcBorrow<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor<Params>(PhantomData<Params>);
        impl<'de, Params> serde::de::Visitor<'de> for Visitor<Params>
        where
            Params: RpcBorrow<'de>,
        {
            type Value = Request<Params>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    formatter,
                    "a JSON-RPC 2.0 request object with params of type {}",
                    std::any::type_name::<Params>()
                )
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut id = None;
                let mut params = None;
                let mut method = None;
                let mut jsonrpc = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        "id" => {
                            if id.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id = Some(map.next_value()?);
                        }
                        "params" => {
                            if params.is_some() {
                                return Err(serde::de::Error::duplicate_field("params"));
                            }
                            params = Some(map.next_value()?);
                        }
                        "method" => {
                            if method.is_some() {
                                return Err(serde::de::Error::duplicate_field("method"));
                            }
                            method = Some(map.next_value()?);
                        }
                        "jsonrpc" => {
                            let version: String = map.next_value()?;
                            if version != "2.0" {
                                return Err(serde::de::Error::custom(format!(
                                    "unsupported JSON-RPC version: {}",
                                    version
                                )));
                            }
                            jsonrpc = Some(());
                        }
                        other => {
                            return Err(serde::de::Error::unknown_field(
                                other,
                                &["id", "params", "method", "jsonrpc"],
                            ));
                        }
                    }
                }
                if jsonrpc.is_none() {
                    return Err(serde::de::Error::missing_field("jsonrpc"));
                }
                if method.is_none() {
                    return Err(serde::de::Error::missing_field("method"));
                }

                if params.is_none() {
                    if std::mem::size_of::<Params>() == 0 {
                        // SAFETY: params is a ZST, so it's safe to fail to initialize it
                        unsafe { params = Some(MaybeUninit::<Params>::zeroed().assume_init()) }
                    } else {
                        return Err(serde::de::Error::missing_field("params"));
                    }
                }

                Ok(Request {
                    meta: RequestMeta::new(method.unwrap(), id.unwrap_or(Id::None)),
                    params: params.unwrap(),
                })
            }
        }

        deserializer.deserialize_map(Visitor(PhantomData))
    }
}

/// A JSON-RPC 2.0 request object that has been serialized, with its [`Id`] and
/// method preserved.
///
/// This struct is used to represent a request that has been serialized, but
/// not yet sent. It is used by RPC clients to build batch requests and manage
/// in-flight requests.
#[derive(Clone, Debug)]
pub struct SerializedRequest {
    meta: RequestMeta,
    request: Box<RawValue>,
}

impl<Params> std::convert::TryFrom<Request<Params>> for SerializedRequest
where
    Params: RpcSend,
{
    type Error = serde_json::Error;

    fn try_from(value: Request<Params>) -> Result<Self, Self::Error> {
        value.serialize()
    }
}

impl SerializedRequest {
    /// Returns the request metadata (ID and Method).
    pub const fn meta(&self) -> &RequestMeta {
        &self.meta
    }

    /// Returns the request ID.
    pub const fn id(&self) -> &Id {
        &self.meta.id
    }

    /// Returns the request method.
    pub fn method(&self) -> &str {
        &self.meta.method
    }

    /// Mark the request as a non-standard subscription (i.e. not
    /// `eth_subscribe`)
    pub fn set_is_subscription(&mut self) {
        self.meta.set_is_subscription();
    }

    /// Returns `true` if the request is a subscription.
    pub fn is_subscription(&self) -> bool {
        self.meta.is_subscription()
    }

    /// Returns the serialized request.
    pub const fn serialized(&self) -> &RawValue {
        &self.request
    }

    /// Consume the serialized request, returning the underlying [`RawValue`].
    pub fn into_serialized(self) -> Box<RawValue> {
        self.request
    }

    /// Consumes the serialized request, returning the underlying
    /// [`RequestMeta`] and the [`RawValue`].
    pub fn decompose(self) -> (RequestMeta, Box<RawValue>) {
        (self.meta, self.request)
    }

    /// Take the serialized request, consuming the [`SerializedRequest`].
    pub fn take_request(self) -> Box<RawValue> {
        self.request
    }

    /// Get a reference to the serialized request's params.
    ///
    /// This partially deserializes the request, and should be avoided if
    /// possible.
    pub fn params(&self) -> Option<&RawValue> {
        #[derive(Deserialize)]
        struct Req<'a> {
            #[serde(borrow)]
            params: Option<&'a RawValue>,
        }

        let req: Req<'_> = serde_json::from_str(self.request.get()).unwrap();
        req.params
    }

    /// Get the hash of the serialized request's params.
    ///
    /// This partially deserializes the request, and should be avoided if
    /// possible.
    pub fn params_hash(&self) -> B256 {
        self.params().map_or_else(|| keccak256(""), |params| keccak256(params.get()))
    }
}

impl Serialize for SerializedRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.request.serialize(serializer)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::RpcObject;

    fn test_inner<T: RpcObject + PartialEq>(t: T) {
        let ser = serde_json::to_string(&t).unwrap();
        let de: T = serde_json::from_str(&ser).unwrap();
        let reser = serde_json::to_string(&de).unwrap();
        assert_eq!(de, t, "deser error for {}", std::any::type_name::<T>());
        assert_eq!(ser, reser, "reser error for {}", std::any::type_name::<T>());
    }

    #[test]
    fn test_ser_deser() {
        test_inner(Request::<()>::new("test", 1.into(), ()));
        test_inner(Request::<u64>::new("test", "hello".to_string().into(), 1));
        test_inner(Request::<String>::new("test", Id::None, "test".to_string()));
        test_inner(Request::<Vec<u64>>::new("test", u64::MAX.into(), vec![1, 2, 3]));
    }
}
