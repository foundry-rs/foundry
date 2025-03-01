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
#![allow(clippy::literal_string_with_formatting_args)] // TODO: https://github.com/rust-lang/rust-clippy/issues/13885

#[macro_use]
#[allow(unused_imports)]
extern crate alloc;

pub extern crate alloy_sol_type_parser as parser;

mod abi;
pub use abi::{ContractObject, IntoItems, Items, JsonAbi};

mod item;
pub use item::{AbiItem, Constructor, Error, Event, Fallback, Function, Receive};

mod param;
pub use param::{EventParam, Param};

pub use parser::{serde_state_mutability_compat, StateMutability};

mod internal_type;
pub use internal_type::InternalType;

mod to_sol;
pub use to_sol::ToSolConfig;

pub(crate) mod utils;
