use crate::utils::{
    attributes_include, field_ident, is_optional, make_generics, parse_struct, EMPTY_STRING_CODE,
};
use proc_macro2::TokenStream;
use quote::quote;
use std::iter::Peekable;
use syn::{Error, Result};

pub(crate) fn impl_encodable(ast: &syn::DeriveInput) -> Result<TokenStream> {
    let body = parse_struct(ast, "RlpEncodable")?;

    let mut fields = body
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !attributes_include(&field.attrs, "skip"))
        .peekable();

    let supports_trailing_opt = attributes_include(&ast.attrs, "trailing");

    let mut encountered_opt_item = false;
    let mut length_exprs = Vec::with_capacity(body.fields.len());
    let mut encode_exprs = Vec::with_capacity(body.fields.len());

    while let Some((i, field)) = fields.next() {
        let is_opt = is_optional(field);
        if is_opt {
            if !supports_trailing_opt {
                let msg = "optional fields are disabled.\nAdd the `#[rlp(trailing)]` attribute to the struct in order to enable optional fields";
                return Err(Error::new_spanned(field, msg));
            }
            encountered_opt_item = true;
        } else if encountered_opt_item {
            let msg = "all the fields after the first optional field must be optional";
            return Err(Error::new_spanned(field, msg));
        }

        length_exprs.push(encodable_length(i, field, is_opt, fields.clone()));
        encode_exprs.push(encodable_field(i, field, is_opt, fields.clone()));
    }

    let name = &ast.ident;
    let generics = make_generics(&ast.generics, quote!(alloy_rlp::Encodable));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            extern crate alloy_rlp;

            impl #impl_generics alloy_rlp::Encodable for #name #ty_generics #where_clause {
                #[inline]
                fn length(&self) -> usize {
                    let payload_length = self._alloy_rlp_payload_length();
                    payload_length + alloy_rlp::length_of_length(payload_length)
                }

                #[inline]
                fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
                    alloy_rlp::Header {
                        list: true,
                        payload_length: self._alloy_rlp_payload_length(),
                    }
                    .encode(out);
                    #(#encode_exprs)*
                }
            }

            impl #impl_generics #name #ty_generics #where_clause {
                #[allow(unused_parens)]
                #[inline]
                fn _alloy_rlp_payload_length(&self) -> usize {
                    0usize #( + #length_exprs)*
                }
            }
        };
    })
}

pub(crate) fn impl_encodable_wrapper(ast: &syn::DeriveInput) -> Result<TokenStream> {
    let body = parse_struct(ast, "RlpEncodableWrapper")?;

    let name = &ast.ident;
    let generics = make_generics(&ast.generics, quote!(alloy_rlp::Encodable));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let ident = {
        let fields: Vec<_> = body.fields.iter().collect();
        if let [field] = fields[..] {
            field_ident(0, field)
        } else {
            let msg = "`RlpEncodableWrapper` is only derivable for structs with one field";
            return Err(Error::new(name.span(), msg));
        }
    };

    Ok(quote! {
        const _: () = {
            extern crate alloy_rlp;

            impl #impl_generics alloy_rlp::Encodable for #name #ty_generics #where_clause {
                #[inline]
                fn length(&self) -> usize {
                    alloy_rlp::Encodable::length(&self.#ident)
                }

                #[inline]
                fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
                    alloy_rlp::Encodable::encode(&self.#ident, out)
                }
            }
        };
    })
}

pub(crate) fn impl_max_encoded_len(ast: &syn::DeriveInput) -> Result<TokenStream> {
    let body = parse_struct(ast, "RlpMaxEncodedLen")?;

    let tys = body
        .fields
        .iter()
        .filter(|field| !attributes_include(&field.attrs, "skip"))
        .map(|field| &field.ty);

    let name = &ast.ident;

    let generics = make_generics(&ast.generics, quote!(alloy_rlp::MaxEncodedLenAssoc));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let imp = quote! {{
        let _sz = 0usize #( + <#tys as alloy_rlp::MaxEncodedLenAssoc>::LEN )*;
        _sz + alloy_rlp::length_of_length(_sz)
    }};

    // can't do operations with const generic params / associated consts in the
    // non-associated impl
    let can_derive_non_assoc = ast
        .generics
        .params
        .iter()
        .all(|g| !matches!(g, syn::GenericParam::Type(_) | syn::GenericParam::Const(_)));
    let non_assoc_impl =  can_derive_non_assoc.then(|| {
        quote! {
            unsafe impl #impl_generics alloy_rlp::MaxEncodedLen<#imp> for #name #ty_generics #where_clause {}
        }
    });

    Ok(quote! {
        #[allow(unsafe_code)]
        const _: () = {
            extern crate alloy_rlp;

            #non_assoc_impl

            unsafe impl #impl_generics alloy_rlp::MaxEncodedLenAssoc for #name #ty_generics #where_clause {
                const LEN: usize = #imp;
            }
        };
    })
}

fn encodable_length<'a>(
    index: usize,
    field: &syn::Field,
    is_opt: bool,
    mut remaining: Peekable<impl Iterator<Item = (usize, &'a syn::Field)>>,
) -> TokenStream {
    let ident = field_ident(index, field);

    if is_opt {
        let default = if remaining.peek().is_some() {
            let condition = remaining_opt_fields_some_condition(remaining);
            quote! { (#condition) as usize }
        } else {
            quote! { 0 }
        };

        quote! { self.#ident.as_ref().map(|val| alloy_rlp::Encodable::length(val)).unwrap_or(#default) }
    } else {
        quote! { alloy_rlp::Encodable::length(&self.#ident) }
    }
}

fn encodable_field<'a>(
    index: usize,
    field: &syn::Field,
    is_opt: bool,
    mut remaining: Peekable<impl Iterator<Item = (usize, &'a syn::Field)>>,
) -> TokenStream {
    let ident = field_ident(index, field);

    if is_opt {
        let if_some_encode = quote! {
            if let Some(val) = self.#ident.as_ref() {
                alloy_rlp::Encodable::encode(val, out)
            }
        };

        if remaining.peek().is_some() {
            let condition = remaining_opt_fields_some_condition(remaining);
            quote! {
                #if_some_encode
                else if #condition {
                    out.put_u8(#EMPTY_STRING_CODE);
                }
            }
        } else {
            quote! { #if_some_encode }
        }
    } else {
        quote! { alloy_rlp::Encodable::encode(&self.#ident, out); }
    }
}

fn remaining_opt_fields_some_condition<'a>(
    remaining: impl Iterator<Item = (usize, &'a syn::Field)>,
) -> TokenStream {
    let conditions = remaining.map(|(index, field)| {
        let ident = field_ident(index, field);
        quote! { self.#ident.is_some() }
    });
    quote! { #(#conditions)||* }
}
