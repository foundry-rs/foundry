//! Token generation utilities.
//!
//! These use items from the standard library as `::std`. This works unless
//! users do `extern crate x as std`, which is extremely unlikely.

use proc_macro2::TokenStream;
use quote::quote;

pub fn option_some() -> TokenStream {
    quote!(::std::option::Option::Some)
}

pub fn option_none() -> TokenStream {
    quote!(::std::option::Option::None)
}
