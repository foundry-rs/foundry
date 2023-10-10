#![warn(unused_crate_dependencies)]

mod console_fmt;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(ConsoleFmt)]
pub fn console_fmt(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    console_fmt::console_fmt(&input).into()
}
