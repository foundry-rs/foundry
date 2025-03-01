//! [`ItemUdt`] expansion.

use super::ExpCtxt;
use alloy_sol_macro_input::ContainsSolAttrs;
use ast::ItemUdt;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Result;

pub(super) fn expand(cx: &ExpCtxt<'_>, udt: &ItemUdt) -> Result<TokenStream> {
    let ItemUdt { name, ty, .. } = udt;

    let (sol_attrs, mut attrs) = udt.split_attrs()?;
    cx.type_derives(&mut attrs, std::iter::once(ty), true);

    let underlying_sol = cx.expand_type(ty);
    let underlying_rust = cx.expand_rust_type(ty);

    let type_check_body = if let Some(lit_str) = sol_attrs.type_check {
        let func_path: syn::Path = lit_str.parse()?;
        quote! {
            <#underlying_sol as alloy_sol_types::SolType>::type_check(token)?;
            #func_path(token)
        }
    } else {
        quote! {
            <#underlying_sol as alloy_sol_types::SolType>::type_check(token)
        }
    };

    let alloy_sol_types = &cx.crates.sol_types;

    let tokens = quote! {
        #(#attrs)*
        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields)]
        #[derive(Clone)]
        pub struct #name(#underlying_rust);

        const _: () = {
            use #alloy_sol_types as alloy_sol_types;

            #[automatically_derived]
            impl alloy_sol_types::private::SolTypeValue<#name> for #underlying_rust {
                #[inline]
                fn stv_to_tokens(&self) -> <#underlying_sol as alloy_sol_types::SolType>::Token<'_> {
                    alloy_sol_types::private::SolTypeValue::<#underlying_sol>::stv_to_tokens(self)
                }

                #[inline]
                fn stv_eip712_data_word(&self) -> alloy_sol_types::Word {
                    <#underlying_sol as alloy_sol_types::SolType>::tokenize(self).0
                }

                #[inline]
                fn stv_abi_encode_packed_to(&self, out: &mut alloy_sol_types::private::Vec<u8>) {
                    <#underlying_sol as alloy_sol_types::SolType>::abi_encode_packed_to(self, out)
                }

                #[inline]
                fn stv_abi_packed_encoded_size(&self) -> usize {
                    <#underlying_sol as alloy_sol_types::SolType>::abi_encoded_size(self)
                }
            }

            #[automatically_derived]
            impl #name {
                /// The Solidity type name.
                pub const NAME: &'static str = stringify!(@name);

                /// Convert from the underlying value type.
                #[inline]
                pub const fn from(value: #underlying_rust) -> Self {
                    Self(value)
                }

                /// Return the underlying value.
                #[inline]
                pub const fn into(self) -> #underlying_rust {
                    self.0
                }

                /// Return the single encoding of this value, delegating to the
                /// underlying type.
                #[inline]
                pub fn abi_encode(&self) -> alloy_sol_types::private::Vec<u8> {
                    <Self as alloy_sol_types::SolType>::abi_encode(&self.0)
                }

                /// Return the packed encoding of this value, delegating to the
                /// underlying type.
                #[inline]
                pub fn abi_encode_packed(&self) -> alloy_sol_types::private::Vec<u8> {
                    <Self as alloy_sol_types::SolType>::abi_encode_packed(&self.0)
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::SolType for #name {
                type RustType = #underlying_rust;
                type Token<'a> = <#underlying_sol as alloy_sol_types::SolType>::Token<'a>;

                const SOL_NAME: &'static str = Self::NAME;
                const ENCODED_SIZE: Option<usize> = <#underlying_sol as alloy_sol_types::SolType>::ENCODED_SIZE;
                const PACKED_ENCODED_SIZE: Option<usize> = <#underlying_sol as alloy_sol_types::SolType>::PACKED_ENCODED_SIZE;

                #[inline]
                fn valid_token(token: &Self::Token<'_>) -> bool {
                    Self::type_check(token).is_ok()
                }

                #[inline]
                fn type_check(token: &Self::Token<'_>) -> alloy_sol_types::Result<()> {
                    #type_check_body
                }

                #[inline]
                fn detokenize(token: Self::Token<'_>) -> Self::RustType {
                    <#underlying_sol as alloy_sol_types::SolType>::detokenize(token)
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::EventTopic for #name {
                #[inline]
                fn topic_preimage_length(rust: &Self::RustType) -> usize {
                    <#underlying_sol as alloy_sol_types::EventTopic>::topic_preimage_length(rust)
                }

                #[inline]
                fn encode_topic_preimage(rust: &Self::RustType, out: &mut alloy_sol_types::private::Vec<u8>) {
                    <#underlying_sol as alloy_sol_types::EventTopic>::encode_topic_preimage(rust, out)
                }

                #[inline]
                fn encode_topic(rust: &Self::RustType) -> alloy_sol_types::abi::token::WordToken {
                    <#underlying_sol as alloy_sol_types::EventTopic>::encode_topic(rust)
                }
            }
        };
    };
    Ok(tokens)
}
