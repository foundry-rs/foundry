//! [`ItemEnum`] expansion.

use super::ExpCtxt;
use alloy_sol_macro_input::{derives_mapped, mk_doc, ContainsSolAttrs};
use ast::{ItemEnum, Spanned};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Result;

/// Expands an [`ItemEnum`]:
///
/// ```ignore (pseudo-code)
/// #[repr(u8)]
/// pub enum #name {
///     #(#variant,)*
/// }
///
/// impl SolEnum for #name {
///     ...
/// }
/// ```
pub(super) fn expand(cx: &ExpCtxt<'_>, enumm: &ItemEnum) -> Result<TokenStream> {
    let ItemEnum { name, variants, .. } = enumm;

    let (sol_attrs, mut attrs) = enumm.split_attrs()?;
    cx.derives(&mut attrs, [], false);
    let docs = sol_attrs.docs.or(cx.attrs.docs).unwrap_or(true);

    let name_s = name.to_string();

    let count = variants.len();
    if count == 0 {
        return Err(syn::Error::new(enumm.span(), "enum has no variants"));
    }
    if count > 256 {
        return Err(syn::Error::new(enumm.span(), "enum has too many variants"));
    }
    let max = (count - 1) as u8;

    let has_invalid_variant = max != u8::MAX;
    let invalid_variant = has_invalid_variant.then(|| {
        let comma = (!variants.trailing_punct()).then(syn::token::Comma::default);

        let has_serde = derives_mapped(&attrs).any(|path| {
            let Some(last) = path.segments.last() else {
                return false;
            };
            last.ident == "Serialize" || last.ident == "Deserialize"
        });
        let serde_other = has_serde.then(|| quote!(#[serde(other)]));

        quote! {
            #comma
            /// Invalid variant.
            ///
            /// This is only used when decoding an out-of-range `u8` value.
            #[doc(hidden)]
            #serde_other
            __Invalid = u8::MAX,
        }
    });
    let detokenize_unwrap = if has_invalid_variant {
        quote! { unwrap_or(Self::__Invalid) }
    } else {
        quote! { expect("unreachable") }
    };

    let alloy_sol_types = &cx.crates.sol_types;

    let uint8 = quote!(alloy_sol_types::sol_data::Uint<8>);
    let uint8_st = quote!(<#uint8 as alloy_sol_types::SolType>);

    let index_to_variant = variants.iter().enumerate().map(|(idx, variant)| {
        let ident = &variant.ident;
        let idx = idx as u8;
        quote! { #idx => ::core::result::Result::Ok(Self::#ident), }
    });

    let doc = docs.then(|| mk_doc(format!("```solidity\n{enumm}\n```")));
    let tokens = quote! {
        #(#attrs)*
        #doc
        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields, clippy::style)]
        #[derive(Clone, Copy)]
        #[repr(u8)]
        pub enum #name {
            #variants
            #invalid_variant
        }

        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields, clippy::style)]
        const _: () = {
            use #alloy_sol_types as alloy_sol_types;

            #[automatically_derived]
            impl ::core::convert::From<#name> for u8 {
                #[inline]
                fn from(v: #name) -> Self {
                    v as u8
                }
            }

            #[automatically_derived]
            impl ::core::convert::TryFrom<u8> for #name {
                type Error = alloy_sol_types::Error;

                #[inline]
                fn try_from(value: u8) -> alloy_sol_types::Result<Self> {
                    match value {
                        #(#index_to_variant)*
                        value => ::core::result::Result::Err(alloy_sol_types::Error::InvalidEnumValue {
                            name: #name_s,
                            value,
                            max: #max,
                        })
                    }
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::SolValue for #name {
                type SolType = Self;
            }

            #[automatically_derived]
            impl alloy_sol_types::private::SolTypeValue<#name> for #name {
                #[inline]
                fn stv_to_tokens(&self) -> #uint8_st::Token<'_> {
                    alloy_sol_types::Word::with_last_byte(*self as u8).into()
                }

                #[inline]
                fn stv_eip712_data_word(&self) -> alloy_sol_types::Word {
                    #uint8_st::eip712_data_word(&(*self as u8))
                }

                #[inline]
                fn stv_abi_encode_packed_to(&self, out: &mut alloy_sol_types::private::Vec<u8>) {
                    out.push(*self as u8);
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::SolType for #name {
                type RustType = #name;
                type Token<'a> = #uint8_st::Token<'a>;

                const SOL_NAME: &'static str = #uint8_st::SOL_NAME;
                const ENCODED_SIZE: ::core::option::Option<usize> = #uint8_st::ENCODED_SIZE;
                const PACKED_ENCODED_SIZE: ::core::option::Option<usize> = #uint8_st::PACKED_ENCODED_SIZE;

                #[inline]
                fn valid_token(token: &Self::Token<'_>) -> bool {
                    Self::type_check(token).is_ok()
                }

                #[inline]
                fn type_check(token: &Self::Token<'_>) -> alloy_sol_types::Result<()> {
                    #uint8_st::type_check(token)?;
                    <Self as ::core::convert::TryFrom<u8>>::try_from(
                        #uint8_st::detokenize(*token)
                    ).map(::core::mem::drop)
                }

                #[inline]
                fn detokenize(token: Self::Token<'_>) -> Self::RustType {
                    <Self as ::core::convert::TryFrom<u8>>::try_from(
                        #uint8_st::detokenize(token)
                    ).#detokenize_unwrap
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::EventTopic for #name {
                #[inline]
                fn topic_preimage_length(rust: &Self::RustType) -> usize {
                    <#uint8 as alloy_sol_types::EventTopic>::topic_preimage_length(&(*rust as u8))
                }

                #[inline]
                fn encode_topic_preimage(rust: &Self::RustType, out: &mut alloy_sol_types::private::Vec<u8>) {
                    <#uint8 as alloy_sol_types::EventTopic>::encode_topic_preimage(&(*rust as u8), out);
                }

                #[inline]
                fn encode_topic(rust: &Self::RustType) -> alloy_sol_types::abi::token::WordToken {
                    <#uint8 as alloy_sol_types::EventTopic>::encode_topic(&(*rust as u8))
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::SolEnum for #name {
                const COUNT: usize = #count;
            }
        };
    };
    Ok(tokens)
}
