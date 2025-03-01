//! [`ItemStruct`] expansion.

use super::{expand_fields, expand_from_into_tuples, expand_tokenize, ExpCtxt};
use alloy_sol_macro_input::{mk_doc, ContainsSolAttrs};
use ast::{Item, ItemStruct, Spanned, Type};
use proc_macro2::TokenStream;
use quote::quote;
use std::num::NonZeroU16;
use syn::Result;

/// Expands an [`ItemStruct`]:
///
/// ```ignore (pseudo-code)
/// pub struct #name {
///     #(pub #field_name: #field_type,)*
/// }
///
/// impl SolStruct for #name {
///     ...
/// }
///
/// // Needed to use in event parameters
/// impl EventTopic for #name {
///     ...
/// }
/// ```
pub(super) fn expand(cx: &ExpCtxt<'_>, s: &ItemStruct) -> Result<TokenStream> {
    let ItemStruct { name, fields, .. } = s;

    let (sol_attrs, mut attrs) = s.split_attrs()?;

    cx.derives(&mut attrs, fields, true);
    let docs = sol_attrs.docs.or(cx.attrs.docs).unwrap_or(true);

    let (field_types, field_names): (Vec<_>, Vec<_>) =
        fields.iter().map(|f| (cx.expand_type(&f.ty), f.name.as_ref().unwrap())).unzip();

    let eip712_encode_type_fns = expand_encode_type_fns(cx, fields, name);

    let tokenize_impl = expand_tokenize(fields, cx);

    let encode_data_impl = match fields.len() {
        0 => unreachable!("struct with zero fields"),
        1 => {
            let name = *field_names.first().unwrap();
            let ty = field_types.first().unwrap();
            quote!(<#ty as alloy_sol_types::SolType>::eip712_data_word(&self.#name).0.to_vec())
        }
        _ => quote! {
            [#(
                <#field_types as alloy_sol_types::SolType>::eip712_data_word(&self.#field_names).0,
            )*].concat()
        },
    };

    let alloy_sol_types = &cx.crates.sol_types;

    let attrs = attrs.iter();
    let convert = expand_from_into_tuples(&name.0, fields, cx);
    let name_s = name.as_string();
    let fields = expand_fields(fields, cx);

    let doc = docs.then(|| mk_doc(format!("```solidity\n{s}\n```")));
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

            #convert

            #[automatically_derived]
            impl alloy_sol_types::SolValue for #name {
                type SolType = Self;
            }

            #[automatically_derived]
            impl alloy_sol_types::private::SolTypeValue<Self> for #name {
                #[inline]
                fn stv_to_tokens(&self) -> <Self as alloy_sol_types::SolType>::Token<'_> {
                    #tokenize_impl
                }

                #[inline]
                fn stv_abi_encoded_size(&self) -> usize {
                    if let Some(size) = <Self as alloy_sol_types::SolType>::ENCODED_SIZE {
                        return size;
                    }

                    // TODO: Avoid cloning
                    let tuple = <UnderlyingRustTuple<'_> as ::core::convert::From<Self>>::from(self.clone());
                    <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::abi_encoded_size(&tuple)
                }

                #[inline]
                fn stv_eip712_data_word(&self) -> alloy_sol_types::Word {
                    <Self as alloy_sol_types::SolStruct>::eip712_hash_struct(self)
                }

                #[inline]
                fn stv_abi_encode_packed_to(&self, out: &mut alloy_sol_types::private::Vec<u8>) {
                    // TODO: Avoid cloning
                    let tuple = <UnderlyingRustTuple<'_> as ::core::convert::From<Self>>::from(self.clone());
                    <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::abi_encode_packed_to(&tuple, out)
                }

                #[inline]
                fn stv_abi_packed_encoded_size(&self) -> usize {
                    if let Some(size) = <Self as alloy_sol_types::SolType>::PACKED_ENCODED_SIZE {
                        return size;
                    }

                    // TODO: Avoid cloning
                    let tuple = <UnderlyingRustTuple<'_> as ::core::convert::From<Self>>::from(self.clone());
                    <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::abi_packed_encoded_size(&tuple)
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::SolType for #name {
                type RustType = Self;
                type Token<'a> = <UnderlyingSolTuple<'a> as alloy_sol_types::SolType>::Token<'a>;

                const SOL_NAME: &'static str = <Self as alloy_sol_types::SolStruct>::NAME;
                const ENCODED_SIZE: Option<usize> =
                    <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::ENCODED_SIZE;
                const PACKED_ENCODED_SIZE: Option<usize> =
                    <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::PACKED_ENCODED_SIZE;

                #[inline]
                fn valid_token(token: &Self::Token<'_>) -> bool {
                    <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::valid_token(token)
                }

                #[inline]
                fn detokenize(token: Self::Token<'_>) -> Self::RustType {
                    let tuple = <UnderlyingSolTuple<'_> as alloy_sol_types::SolType>::detokenize(token);
                    <Self as ::core::convert::From<UnderlyingRustTuple<'_>>>::from(tuple)
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::SolStruct for #name {
                const NAME: &'static str = #name_s;

                #eip712_encode_type_fns

                #[inline]
                fn eip712_encode_data(&self) -> alloy_sol_types::private::Vec<u8> {
                    #encode_data_impl
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::EventTopic for #name {
                #[inline]
                fn topic_preimage_length(rust: &Self::RustType) -> usize {
                    0usize
                    #(
                        + <#field_types as alloy_sol_types::EventTopic>::topic_preimage_length(&rust.#field_names)
                    )*
                }

                #[inline]
                fn encode_topic_preimage(rust: &Self::RustType, out: &mut alloy_sol_types::private::Vec<u8>) {
                    out.reserve(<Self as alloy_sol_types::EventTopic>::topic_preimage_length(rust));
                    #(
                        <#field_types as alloy_sol_types::EventTopic>::encode_topic_preimage(&rust.#field_names, out);
                    )*
                }

                #[inline]
                fn encode_topic(rust: &Self::RustType) -> alloy_sol_types::abi::token::WordToken {
                    let mut out = alloy_sol_types::private::Vec::new();
                    <Self as alloy_sol_types::EventTopic>::encode_topic_preimage(rust, &mut out);
                    alloy_sol_types::abi::token::WordToken(
                        alloy_sol_types::private::keccak256(out)
                    )
                }
            }
        };
    };
    Ok(tokens)
}

fn expand_encode_type_fns(
    cx: &ExpCtxt<'_>,
    fields: &ast::Parameters<syn::token::Semi>,
    name: &ast::SolIdent,
) -> TokenStream {
    // account for UDVTs and enums which do not implement SolStruct
    let mut fields = fields.clone();
    fields.visit_types_mut(|ty| {
        let Type::Custom(name) = ty else { return };
        match cx.try_item(name) {
            // keep as custom
            Some(Item::Struct(_)) | None => {}
            // convert to underlying
            Some(Item::Contract(_)) => *ty = Type::Address(ty.span(), None),
            Some(Item::Enum(_)) => *ty = Type::Uint(ty.span(), NonZeroU16::new(8)),
            Some(Item::Udt(udt)) => *ty = udt.ty.clone(),
            Some(item) => {
                proc_macro_error2::abort!(item.span(), "Invalid type in struct field: {:?}", item)
            }
        }
    });

    let root = fields.eip712_signature(name.as_string());

    let custom = fields.iter().filter(|f| f.ty.has_custom());
    let n_custom = custom.clone().count();

    let components_impl = if n_custom > 0 {
        let bits = custom.map(|field| {
            // need to recurse to find the inner custom type
            let mut ty = None;
            field.ty.visit(|field_ty| {
                if ty.is_none() && field_ty.is_custom() {
                    ty = Some(field_ty.clone());
                }
            });
            // cannot panic as this field is guaranteed to contain a custom type
            let ty = cx.expand_type(&ty.unwrap());

            quote! {
                components.push(<#ty as alloy_sol_types::SolStruct>::eip712_root_type());
                components.extend(<#ty as alloy_sol_types::SolStruct>::eip712_components());
            }
        });
        let capacity = proc_macro2::Literal::usize_unsuffixed(n_custom);
        quote! {
            let mut components = alloy_sol_types::private::Vec::with_capacity(#capacity);
            #(#bits)*
            components
        }
    } else {
        quote! { alloy_sol_types::private::Vec::new() }
    };

    let encode_type_impl_opt = (n_custom == 0).then(|| {
        quote! {
            #[inline]
            fn eip712_encode_type() -> alloy_sol_types::private::Cow<'static, str> {
                <Self as alloy_sol_types::SolStruct>::eip712_root_type()
            }
        }
    });

    quote! {
        #[inline]
        fn eip712_root_type() -> alloy_sol_types::private::Cow<'static, str> {
            alloy_sol_types::private::Cow::Borrowed(#root)
        }

        #[inline]
        fn eip712_components() -> alloy_sol_types::private::Vec<alloy_sol_types::private::Cow<'static, str>> {
            #components_impl
        }

        #encode_type_impl_opt
    }
}
