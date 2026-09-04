//! # foundry-macros
//!
//! Internal Foundry proc-macros.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

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
pub fn cheatcode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let mut errors = Vec::new();
    let mut output =
        cheatcodes::derive_cheatcode(&input, &mut errors).unwrap_or_else(Error::into_compile_error);
    output.extend(errors.into_iter().map(Error::into_compile_error));
    output.into()
}
