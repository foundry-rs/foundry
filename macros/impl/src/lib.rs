mod hhcl;
mod utils;

pub(crate) use utils::*;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(FormatValue)]
pub fn hhcl(input: TokenStream) -> TokenStream {
    eprintln!("INPUT ={input}");
    let input = parse_macro_input!(input as DeriveInput);
    let out = TokenStream::from(hhcl::hhcl(input));
    eprintln!("OUTPUT={out}");
    out
}
