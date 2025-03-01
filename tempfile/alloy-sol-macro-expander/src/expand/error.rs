//! [`ItemError`] expansion.

use super::{expand_fields, expand_from_into_tuples, expand_tokenize, ExpCtxt};
use alloy_sol_macro_input::{mk_doc, ContainsSolAttrs};
use ast::ItemError;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Result;

/// Expands an [`ItemError`]:
///
/// ```ignore (pseudo-code)
/// pub struct #name {
///     #(pub #parameter_name: #parameter_type,)*
/// }
///
/// impl SolError for #name {
///     ...
/// }
/// ```
pub(super) fn expand(cx: &ExpCtxt<'_>, error: &ItemError) -> Result<TokenStream> {
    let ItemError { parameters: params, .. } = error;
    cx.assert_resolved(params)?;

    let (sol_attrs, mut attrs) = error.split_attrs()?;
    cx.derives(&mut attrs, params, true);
    let docs = sol_attrs.docs.or(cx.attrs.docs).unwrap_or(true);
    let abi = sol_attrs.abi.or(cx.attrs.abi).unwrap_or(false);

    let tokenize_impl = expand_tokenize(params, cx);

    let name = cx.overloaded_name(error.into());
    let signature = cx.error_signature(error);
    let selector = crate::utils::selector(&signature);

    let alloy_sol_types = &cx.crates.sol_types;

    let converts = expand_from_into_tuples(&name.0, params, cx);
    let fields = expand_fields(params, cx);
    let doc = docs.then(|| {
        let selector = hex::encode_prefixed(selector.array.as_slice());
        mk_doc(format!(
            "Custom error with signature `{signature}` and selector `{selector}`.\n\
             ```solidity\n{error}\n```"
        ))
    });
    let abi: Option<TokenStream> = abi.then(|| {
        if_json! {
            let error = super::to_abi::generate(error, cx);
            quote! {
                #[automatically_derived]
                impl alloy_sol_types::JsonAbiExt for #name {
                    type Abi = alloy_sol_types::private::alloy_json_abi::Error;

                    #[inline]
                    fn abi() -> Self::Abi {
                        #error
                    }
                }
            }
        }
    });
    let tokens = quote! {
        #(#attrs)*
        #doc
        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields)]
        #[derive(Clone)]
        pub struct #name {
            #(#fields),*
        }

        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields, clippy::style)]
        const _: () = {
            use #alloy_sol_types as alloy_sol_types;

            #converts

            #[automatically_derived]
            impl alloy_sol_types::SolError for #name {
                type Parameters<'a> = UnderlyingSolTuple<'a>;
                type Token<'a> = <Self::Parameters<'a> as alloy_sol_types::SolType>::Token<'a>;

                const SIGNATURE: &'static str = #signature;
                const SELECTOR: [u8; 4] = #selector;

                #[inline]
                fn new<'a>(tuple: <Self::Parameters<'a> as alloy_sol_types::SolType>::RustType) -> Self {
                    tuple.into()
                }

                #[inline]
                fn tokenize(&self) -> Self::Token<'_> {
                    #tokenize_impl
                }
            }

            #abi
        };
    };
    Ok(tokens)
}
