#![warn(unused_crate_dependencies)]

mod console_fmt;
mod utils;

pub(crate) use utils::*;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(ConsoleFmt)]
pub fn console_fmt(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    console_fmt::console_fmt(input).into()
}
