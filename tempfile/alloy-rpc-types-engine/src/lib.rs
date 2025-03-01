#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
#[allow(unused_imports)]
extern crate alloc;

mod cancun;
pub use cancun::*;
mod prague;
pub use prague::*;
mod sidecar;
pub use sidecar::*;

mod forkchoice;
pub use forkchoice::*;

mod version;
pub use version::ForkchoiceUpdateVersion;

mod identification;
pub use identification::*;

#[cfg(feature = "jwt")]
mod jwt;
#[cfg(feature = "jwt")]
pub use jwt::*;

pub mod payload;
pub use payload::*;

mod error;
pub use error::*;

mod transition;
pub use transition::*;

#[doc(inline)]
pub use alloy_eips::eip4844::BlobAndProofV1;

/// The list of all supported Engine capabilities available over the engine endpoint.
///
/// Latest spec: Prague
pub const CAPABILITIES: &[&str] = &[
    "engine_forkchoiceUpdatedV1",
    "engine_forkchoiceUpdatedV2",
    "engine_forkchoiceUpdatedV3",
    "engine_exchangeTransitionConfigurationV1",
    "engine_getClientVersionV1",
    "engine_getPayloadV1",
    "engine_getPayloadV2",
    "engine_getPayloadV3",
    "engine_getPayloadV4",
    "engine_newPayloadV1",
    "engine_newPayloadV2",
    "engine_newPayloadV3",
    "engine_newPayloadV4",
    "engine_getPayloadBodiesByHashV1",
    "engine_getPayloadBodiesByRangeV1",
];
