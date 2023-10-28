#![warn(unused_crate_dependencies)]

#[macro_use]
extern crate proc_macro_error;

use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use syn::{parse_macro_input, DeriveInput, Error};

mod cheatcodes;
mod console_fmt;

#[proc_macro_attribute]
pub fn vm(_attrs: TokenStream, input: TokenStream) -> TokenStream {
    let clone = input.clone();
    let item = parse_macro_input!(input);
    match cheatcodes::vm_attr(item) {
        Ok(Some(tts)) => tts.into(),
        Ok(None) => clone,
        Err(e) => e.to_compile_error().into(),
    }
}

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
