//! Provides common utils function shared between full and semi-automatic.

use proc_macro2::TokenStream;

use syn::{parse_quote, Generics, Ident};

use quote::quote;

/// Add the given tuple elements as generics with the given `bounds` to `generics`.
pub fn add_tuple_element_generics(
    tuple_elements: &[Ident],
    bounds: Option<TokenStream>,
    generics: &mut Generics,
) {
    let bound = bounds.map(|b| quote!(: #b)).unwrap_or_else(|| quote!());

    tuple_elements.iter().for_each(|tuple_element| {
        generics.params.push(parse_quote!(#tuple_element #bound));
    });
}
