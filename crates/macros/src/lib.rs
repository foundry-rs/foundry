//! # foundry-macros
//!
//! Internal Foundry proc-macros.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate proc_macro_error2;

use proc_macro::TokenStream;
use syn::{DeriveInput, Error, parse_macro_input};

mod cheatcodes;
mod console_fmt;

#[proc_macro_derive(ConsoleFmt)]
pub fn console_fmt(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    console_fmt::console_fmt(&input).into()
}

#[proc_macro_derive(Cheatcode, attributes(cheatcode))]
#[proc_macro_error]
pub fn cheatcode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    cheatcodes::derive_cheatcode(&input).unwrap_or_else(Error::into_compile_error).into()
}
