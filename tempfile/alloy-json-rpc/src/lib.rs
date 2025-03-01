//! Alloy JSON-RPC data types.
//!
//! This crate provides data types for use with the JSON-RPC 2.0 protocol. It
//! does not provide any functionality for actually sending or receiving
//! JSON-RPC data.
//!
//! If you find yourself importing this crate, and you are not implementing a
//! JSON-RPC client or transport, you are likely at the wrong layer of
//! abstraction. If you want to _use_ a JSON-RPC client, consider using the
//! [`alloy-transports`] crate.
//!
//! [`alloy-transports`]: https://docs.rs/alloy-transports/latest/alloy-transports
//!
//! ## Usage
//!
//! This crate models the JSON-RPC 2.0 protocol data-types. It is intended to
//! be used to build JSON-RPC clients or servers. Most users will not need to
//! import this crate.
//!
//! This crate provides the following low-level data types:
//!
//! - [`Request`] - A JSON-RPC request.
//! - [`Response`] - A JSON-RPC response.
//! - [`ErrorPayload`] - A JSON-RPC error response payload, including code and message.
//! - [`ResponsePayload`] - The payload of a JSON-RPC response, either a success payload, or an
//!   [`ErrorPayload`].
//!
//! For client-side Rust ergonomics, we want to map responses to [`Result`]s.
//! To that end, we provide the following types:
//!
//! - [`RpcError`] - An error that can occur during JSON-RPC communication. This type aggregates
//!   errors that are common to all transports, such as (de)serialization, error responses, and
//!   includes a generic transport error.
//! - [`RpcResult`] - A result modeling an Rpc outcome as `Result<T,
//! RpcError<E>>`.
//!
//! We recommend that transport implementors use [`RpcResult`] as the return
//! type for their transport methods, parameterized by their transport error
//! type. This will allow them to return either a successful response or an
//! error.
//!
//! ## Note On (De)Serialization
//!
//! [`Request`], [`Response`], and similar types are generic over the
//! actual data being passed to and from the RPC. We can achieve partial
//! (de)serialization by making them generic over a `serde_json::RawValue`.
//!
//! - For [`Request`] - [`PartiallySerializedRequest`] is a `Request<Box<RawValue>`. It represents a
//!   `Request` whose parameters have been serialized. [`SerializedRequest`], on the other hand is a
//!   request that has been totally serialized. For client-development purposes, its [`Id`] and
//!   method have been preserved.
//! - For [`Response`] - [`BorrowedResponse`] is a `Response<&RawValue>`. It represents a Response
//!   whose [`Id`] and return status (success or failure) have been deserialized, but whose payload
//!   has not.
//!
//! Allowing partial serialization lets us include many unlike [`Request`]
//! objects in collections (e.g. in a batch request). This is useful for
//! implementing a client.
//!
//! Allowing partial deserialization lets learn request status, and associate
//! the raw response data with the corresponding client request before doing
//! full deserialization work. This is useful for implementing a client.
//!
//! In general, partially deserialized responses can be further deserialized.
//! E.g. an [`BorrowedRpcResult`] may have success responses deserialized
//! with [`crate::try_deserialize_ok::<U>`], which will transform it to an
//! [`RpcResult<U>`].

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt::Debug;

mod common;
pub use common::Id;

mod error;
pub use error::RpcError;

mod notification;
pub use notification::{EthNotification, PubSubItem, SubId};

mod packet;
pub use packet::{BorrowedResponsePacket, RequestPacket, ResponsePacket};

mod request;
pub use request::{PartiallySerializedRequest, Request, RequestMeta, SerializedRequest};

mod response;
pub use response::{
    BorrowedErrorPayload, BorrowedResponse, BorrowedResponsePayload, ErrorPayload, Response,
    ResponsePayload,
};

mod result;
pub use result::{
    transform_response, transform_result, try_deserialize_ok, BorrowedRpcResult, RpcResult,
};

/// An object that can be sent over RPC.
///
/// This marker trait is blanket-implemented for every qualifying type. It is
/// used to indicate that a type can be sent in the body of a JSON-RPC message.
pub trait RpcSend: Serialize + Clone + Debug + Send + Sync + Unpin {}

impl<T> RpcSend for T where T: Serialize + Clone + Debug + Send + Sync + Unpin {}

/// An object that can be received over RPC.
///
/// This marker trait is blanket-implemented for every qualifying type. It is
/// used to indicate that a type can be received in the body of a JSON-RPC
/// message.
///
/// # Note
///
/// We add the `'static` lifetime to the supertraits to indicate that the type
/// can't borrow. This is a simplification that makes it easier to use the
/// types in client code. Servers may prefer borrowing, using the [`RpcBorrow`]
/// trait.
pub trait RpcRecv: DeserializeOwned + Debug + Send + Sync + Unpin + 'static {}

impl<T> RpcRecv for T where T: DeserializeOwned + Debug + Send + Sync + Unpin + 'static {}

/// An object that can be received over RPC, borrowing from the the
/// deserialization context.
///
/// This marker trait is blanket-implemented for every qualifying type. It is
/// used to indicate that a type can be borrowed from the body of a wholly or
/// partially serialized JSON-RPC message.
pub trait RpcBorrow<'de>: Deserialize<'de> + Debug + Send + Sync + Unpin {}

impl<'de, T> RpcBorrow<'de> for T where T: Deserialize<'de> + Debug + Send + Sync + Unpin {}

/// An object that can be both sent and received over RPC.
///
/// This marker trait is blanket-implemented for every qualifying type. It is
/// used to indicate that a type can be both sent and received in the body of a
/// JSON-RPC message.
///
/// # Note
///
/// We add the `'static` lifetime to the supertraits to indicate that the type
/// can't borrow. This is a simplification that makes it easier to use the
/// types in client code. Servers may prefer borrowing, using the
/// [`BorrowedRpcObject`] trait.
pub trait RpcObject: RpcSend + RpcRecv {}

impl<T> RpcObject for T where T: RpcSend + RpcRecv {}

/// An object that can be both sent and received over RPC, borrowing from the
/// the deserialization context.
///
/// This marker trait is blanket-implemented for every qualifying type. It is
/// used to indicate that a type can be both sent and received in the body of a
/// JSON-RPC message, and can borrow from the deserialization context.
pub trait BorrowedRpcObject<'de>: RpcBorrow<'de> + RpcSend {}

impl<'de, T> BorrowedRpcObject<'de> for T where T: RpcBorrow<'de> + RpcSend {}
