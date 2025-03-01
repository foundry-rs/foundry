#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[doc(hidden)]
pub use alloy_serde as serde_helpers;

mod rpc;
pub use rpc::*;

#[cfg(feature = "admin")]
pub use alloy_rpc_types_admin as admin;

#[cfg(feature = "anvil")]
pub use alloy_rpc_types_anvil as anvil;

#[cfg(feature = "beacon")]
pub use alloy_rpc_types_beacon as beacon;

#[cfg(feature = "debug")]
pub use alloy_rpc_types_debug as debug;

#[cfg(feature = "engine")]
pub use alloy_rpc_types_engine as engine;

#[cfg(feature = "eth")]
pub use alloy_rpc_types_eth as eth;
#[cfg(feature = "eth")]
pub use eth::*;

#[cfg(feature = "mev")]
pub use alloy_rpc_types_mev as mev;

#[cfg(feature = "trace")]
pub use alloy_rpc_types_trace as trace;

#[cfg(feature = "txpool")]
pub use alloy_rpc_types_txpool as txpool;
