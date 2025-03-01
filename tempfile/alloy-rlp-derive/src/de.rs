use crate::utils::{
    attributes_include, field_ident, is_optional, make_generics, parse_struct, EMPTY_STRING_CODE,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Result};

pub(crate) fn impl_decodable(ast: &syn::DeriveInput) -> Result<TokenStream> {
    let body = parse_struct(ast, "RlpDecodable")?;

    let fields = body.fields.iter().enumerate();

    let supports_trailing_opt = attributes_include(&ast.attrs, "trailing");

    let mut encountered_opt_item = false;
    let mut decode_stmts = Vec::with_capacity(body.fields.len());
    for (i, field) in fields {
        let is_opt = is_optional(field);
        if is_opt {
            if !supports_trailing_opt {
                let msg = "optional fields are disabled.\nAdd the `#[rlp(trailing)]` attribute to the struct in order to enable optional fields";
                return Err(Error::new_spanned(field, msg));
            }
            encountered_opt_item = true;
        } else if encountered_opt_item && !attributes_include(&field.attrs, "default") {
            let msg =
                "all the fields after the first optional field must be either optional or default";
            return Err(Error::new_spanned(field, msg));
        }

        decode_stmts.push(decodable_field(i, field, is_opt));
    }

    let name = &ast.ident;
    let generics = make_generics(&ast.generics, quote!(alloy_rlp::Decodable));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            extern crate alloy_rlp;

            impl #impl_generics alloy_rlp::Decodable for #name #ty_generics #where_clause {
                #[inline]
                fn decode(b: &mut &[u8]) -> alloy_rlp::Result<Self> {
                    let alloy_rlp::Header { list, payload_length } = alloy_rlp::Header::decode(b)?;
                    if !list {
                        return Err(alloy_rlp::Error::UnexpectedString);
                    }

                    let started_len = b.len();
                    if started_len < payload_length {
                        return Err(alloy_rlp::DecodeError::InputTooShort);
                    }

                    let this = Self {
                        #(#decode_stmts)*
                    };

                    let consumed = started_len - b.len();
                    if consumed != payload_length {
                        return Err(alloy_rlp::Error::ListLengthMismatch {
                            expected: payload_length,
                            got: consumed,
                        });
                    }

                    Ok(this)
                }
            }
        };
    })
}

pub(crate) fn impl_decodable_wrapper(ast: &syn::DeriveInput) -> Result<TokenStream> {
    let body = parse_struct(ast, "RlpEncodableWrapper")?;

    if body.fields.iter().count() != 1 {
        let msg = "`RlpEncodableWrapper` is only defined for structs with one field.";
        return Err(Error::new(ast.ident.span(), msg));
    }

    let name = &ast.ident;
    let generics = make_generics(&ast.generics, quote!(alloy_rlp::Decodable));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            extern crate alloy_rlp;

            impl #impl_generics alloy_rlp::Decodable for #name #ty_generics #where_clause {
                #[inline]
                fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
                    alloy_rlp::private::Result::map(alloy_rlp::Decodable::decode(buf), Self)
                }
            }
        };
    })
}

fn decodable_field(index: usize, field: &syn::Field, is_opt: bool) -> TokenStream {
    let ident = field_ident(index, field);

    if attributes_include(&field.attrs, "default") {
        quote! { #ident: alloy_rlp::private::Default::default(), }
    } else if is_opt {
        quote! {
            #ident: if started_len - b.len() < payload_length {
                if alloy_rlp::private::Option::map_or(b.first(), false, |b| *b == #EMPTY_STRING_CODE) {
                    alloy_rlp::Buf::advance(b, 1);
                    None
                } else {
                    Some(alloy_rlp::Decodable::decode(b)?)
                }
            } else {
                None
            },
        }
    } else {
        quote! { #ident: alloy_rlp::Decodable::decode(b)?, }
    }
}
