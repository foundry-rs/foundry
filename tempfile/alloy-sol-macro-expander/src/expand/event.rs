//! [`ItemEvent`] expansion.

use super::{anon_name, expand_event_tokenize, expand_tuple_types, ExpCtxt};
use alloy_sol_macro_input::{mk_doc, ContainsSolAttrs};
use ast::{EventParameter, ItemEvent, SolIdent, Spanned};
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::Result;

/// Expands an [`ItemEvent`]:
///
/// ```ignore (pseudo-code)
/// pub struct #name {
///     #(pub #parameter_name: #parameter_type,)*
/// }
///
/// impl SolEvent for #name {
///     ...
/// }
/// ```
pub(super) fn expand(cx: &ExpCtxt<'_>, event: &ItemEvent) -> Result<TokenStream> {
    let params = event.params();

    let (sol_attrs, mut attrs) = event.split_attrs()?;
    cx.derives(&mut attrs, &params, true);
    let docs = sol_attrs.docs.or(cx.attrs.docs).unwrap_or(true);
    let abi = sol_attrs.abi.or(cx.attrs.abi).unwrap_or(false);

    cx.assert_resolved(&params)?;
    event.assert_valid()?;

    let name = cx.overloaded_name(event.into());
    let signature = cx.event_signature(event);
    let selector = crate::utils::event_selector(&signature);
    let anonymous = event.is_anonymous();

    // prepend the first topic if not anonymous
    let first_topic = (!anonymous).then(|| quote!(alloy_sol_types::sol_data::FixedBytes<32>));
    let topic_list = event.indexed_params().map(|p| expand_event_topic_type(p, cx));
    let topic_list = first_topic.into_iter().chain(topic_list);

    let (data_tuple, _) = expand_tuple_types(event.non_indexed_params().map(|p| &p.ty), cx);

    // skip first topic if not anonymous, which is the hash of the signature
    let mut topic_i = !anonymous as usize;
    let mut data_i = 0usize;
    let new_impl = event.parameters.iter().enumerate().map(|(i, p)| {
        let name = anon_name((i, p.name.as_ref()));
        let param;
        if p.is_indexed() {
            let i = syn::Index::from(topic_i);
            param = quote!(topics.#i);
            topic_i += 1;
        } else {
            let i = syn::Index::from(data_i);
            param = quote!(data.#i);
            data_i += 1;
        }
        quote!(#name: #param)
    });

    // NOTE: We need to enumerate before filtering.
    let topic_tuple_names = event
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_indexed())
        .map(|(i, p)| (i, p.name.as_ref()))
        .map(anon_name);

    let topics_impl = if anonymous {
        quote! {(#(self.#topic_tuple_names.clone(),)*)}
    } else {
        quote! {(Self::SIGNATURE_HASH.into(), #(self.#topic_tuple_names.clone(),)*)}
    };

    let encode_first_topic =
        (!anonymous).then(|| quote!(alloy_sol_types::abi::token::WordToken(Self::SIGNATURE_HASH)));

    let encode_topics_impl = event.parameters.iter().enumerate().filter(|(_, p)| p.is_indexed()).map(|(i, p)| {
        let name = anon_name((i, p.name.as_ref()));
        let ty = cx.expand_type(&p.ty);

        if cx.indexed_as_hash(p) {
            quote! {
                <alloy_sol_types::sol_data::FixedBytes<32> as alloy_sol_types::EventTopic>::encode_topic(&self.#name)
            }
        } else {
            quote! {
                <#ty as alloy_sol_types::EventTopic>::encode_topic(&self.#name)
            }
        }
    });

    let fields = event
        .parameters
        .iter()
        .enumerate()
        .map(|(i, p)| expand_event_topic_field(i, p, p.name.as_ref(), cx));

    let check_signature = (!anonymous).then(|| {
        quote! {
            #[inline]
            fn check_signature(topics: &<Self::TopicList as alloy_sol_types::SolType>::RustType) -> alloy_sol_types::Result<()> {
                if topics.0 != Self::SIGNATURE_HASH {
                    return Err(alloy_sol_types::Error::invalid_event_signature_hash(Self::SIGNATURE, topics.0, Self::SIGNATURE_HASH));
                }
                Ok(())
            }
        }
    });

    let tokenize_body_impl = expand_event_tokenize(&event.parameters, cx);

    let encode_topics_impl = encode_first_topic
        .into_iter()
        .chain(encode_topics_impl)
        .enumerate()
        .map(|(i, assign)| quote!(out[#i] = #assign;));

    let doc = docs.then(|| {
        let selector = hex::encode_prefixed(selector.array.as_slice());
        mk_doc(format!(
            "Event with signature `{signature}` and selector `{selector}`.\n\
            ```solidity\n{event}\n```"
        ))
    });

    let abi: Option<TokenStream> = abi.then(|| {
        if_json! {
            let event = super::to_abi::generate(event, cx);
            quote! {
                #[automatically_derived]
                impl alloy_sol_types::JsonAbiExt for #name {
                    type Abi = alloy_sol_types::private::alloy_json_abi::Event;

                    #[inline]
                    fn abi() -> Self::Abi {
                        #event
                    }
                }
            }
        }
    });

    let alloy_sol_types = &cx.crates.sol_types;

    let tokens = quote! {
        #(#attrs)*
        #doc
        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields, clippy::style)]
        #[derive(Clone)]
        pub struct #name {
            #( #fields, )*
        }

        #[allow(non_camel_case_types, non_snake_case, clippy::pub_underscore_fields, clippy::style)]
        const _: () = {
            use #alloy_sol_types as alloy_sol_types;

            #[automatically_derived]
            impl alloy_sol_types::SolEvent for #name {
                type DataTuple<'a> = #data_tuple;
                type DataToken<'a> = <Self::DataTuple<'a> as alloy_sol_types::SolType>::Token<'a>;

                type TopicList = (#(#topic_list,)*);

                const SIGNATURE: &'static str = #signature;
                const SIGNATURE_HASH: alloy_sol_types::private::B256 =
                    alloy_sol_types::private::B256::new(#selector);

                const ANONYMOUS: bool = #anonymous;

                #[allow(unused_variables)]
                #[inline]
                fn new(
                    topics: <Self::TopicList as alloy_sol_types::SolType>::RustType,
                    data: <Self::DataTuple<'_> as alloy_sol_types::SolType>::RustType,
                ) -> Self {
                    Self {
                        #(#new_impl,)*
                    }
                }

                #check_signature

                #[inline]
                fn tokenize_body(&self) -> Self::DataToken<'_> {
                    #tokenize_body_impl
                }

                #[inline]
                fn topics(&self) -> <Self::TopicList as alloy_sol_types::SolType>::RustType {
                    #topics_impl
                }

                #[inline]
                fn encode_topics_raw(
                    &self,
                    out: &mut [alloy_sol_types::abi::token::WordToken],
                ) -> alloy_sol_types::Result<()> {
                    if out.len() < <Self::TopicList as alloy_sol_types::TopicList>::COUNT {
                        return Err(alloy_sol_types::Error::Overrun);
                    }
                    #(#encode_topics_impl)*
                    Ok(())
                }
            }

            #[automatically_derived]
            impl alloy_sol_types::private::IntoLogData for #name {
                fn to_log_data(&self) -> alloy_sol_types::private::LogData {
                    From::from(self)
                }

                fn into_log_data(self) -> alloy_sol_types::private::LogData {
                    From::from(&self)
                }
            }


            #[automatically_derived]
            impl From<&#name> for alloy_sol_types::private::LogData {
                #[inline]
                fn from(this: &#name) -> alloy_sol_types::private::LogData {
                    alloy_sol_types::SolEvent::encode_log_data(this)
                }
            }

            #abi
        };
    };
    Ok(tokens)
}

fn expand_event_topic_type(param: &EventParameter, cx: &ExpCtxt<'_>) -> TokenStream {
    let alloy_sol_types = &cx.crates.sol_types;
    assert!(param.is_indexed());
    if cx.indexed_as_hash(param) {
        quote_spanned! {param.ty.span()=> #alloy_sol_types::sol_data::FixedBytes<32> }
    } else {
        cx.expand_type(&param.ty)
    }
}

fn expand_event_topic_field(
    i: usize,
    param: &EventParameter,
    name: Option<&SolIdent>,
    cx: &ExpCtxt<'_>,
) -> TokenStream {
    let name = anon_name((i, name));
    let ty = if cx.indexed_as_hash(param) {
        let bytes32 = ast::Type::FixedBytes(name.span(), core::num::NonZeroU16::new(32).unwrap());
        cx.expand_rust_type(&bytes32)
    } else {
        cx.expand_rust_type(&param.ty)
    };
    let attrs = &param.attrs;
    quote! {
        #(#attrs)*
        #[allow(missing_docs)]
        pub #name: #ty
    }
}
