use crate::SolInput;
use proc_macro2::TokenStream;

/// Expands a `SolInput` into a `TokenStream`.
pub trait SolInputExpander {
    /// Expand a `SolInput` into a `TokenStream`.
    fn expand(&mut self, input: &SolInput) -> syn::Result<TokenStream>;
}
