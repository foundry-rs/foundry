// Copyright 2015-2020 Parity Technologies
// Copyright 2023-2023 Alloy Contributors

// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate alloc;

pub extern crate alloy_sol_type_parser as parser;

#[cfg(feature = "arbitrary")]
mod arbitrary;

mod coerce;

mod dynamic;
pub use dynamic::{
    DecodedError, DecodedEvent, DynSolCall, DynSolError, DynSolEvent, DynSolReturns, DynSolType,
    DynSolValue, DynToken,
};

mod error;
pub use error::{Error, Result};

mod ext;
pub use ext::{ErrorExt, EventExt, FunctionExt, JsonAbiExt};

mod specifier;
pub use specifier::Specifier;

#[cfg(feature = "eip712")]
pub mod eip712;
#[cfg(feature = "eip712")]
pub use eip712::{parser as eip712_parser, Eip712Types, PropertyDef, Resolver, TypeDef, TypedData};

#[doc(no_inline)]
pub use alloy_sol_types::{
    abi::{self, Decoder, Encoder},
    Eip712Domain, SolType, Word,
};
