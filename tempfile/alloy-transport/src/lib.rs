#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod boxed;
pub use boxed::{BoxTransport, IntoBoxTransport};

mod connect;
pub use connect::TransportConnect;

mod common;
pub use common::Authorization;

mod error;
#[doc(hidden)]
pub use error::TransportErrorKind;
pub use error::{HttpError, TransportError, TransportResult};

mod r#trait;
pub use r#trait::Transport;

pub use alloy_json_rpc::{RpcError, RpcResult};
pub use futures_utils_wasm::{impl_future, BoxFuture};

pub mod layers;

/// Misc. utilities for building transports.
pub mod utils;

/// Pin-boxed future.
pub type Pbf<'a, T, E> = futures_utils_wasm::BoxFuture<'a, Result<T, E>>;

/// Future for transport-level requests.
pub type TransportFut<'a, T = alloy_json_rpc::ResponsePacket, E = TransportError> = Pbf<'a, T, E>;

/// Future for RPC-level requests.
pub type RpcFut<'a, T> = futures_utils_wasm::BoxFuture<'a, TransportResult<T>>;
