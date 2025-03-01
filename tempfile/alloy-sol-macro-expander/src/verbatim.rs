use crate::expand::ExternCrates;
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

/// Converts the given value into tokens that represent itself.
pub(crate) fn verbatim<T: Verbatim>(t: &T, crates: &ExternCrates) -> TokenStream {
    let mut s = TokenStream::new();
    t.to_verbatim_tokens(&mut s, crates);
    s
}

/// Conversion to tokens that represent the value itself.
pub(crate) trait Verbatim {
    /// Converts `self` into tokens that represent itself.
    fn to_verbatim_tokens(&self, s: &mut TokenStream, crates: &ExternCrates);
}

/// Provides a [`quote::ToTokens`] implementations for references of values that implement
/// [`Verbatim`].
pub(crate) struct ToTokensCompat<'a, T: ?Sized + Verbatim>(pub(crate) &'a T, &'a ExternCrates);

impl<T: Verbatim> quote::ToTokens for ToTokensCompat<'_, T> {
    #[inline]
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.to_verbatim_tokens(tokens, self.1)
    }
}

impl Verbatim for String {
    fn to_verbatim_tokens(&self, tokens: &mut TokenStream, crates: &ExternCrates) {
        let alloy_sol_types = &crates.sol_types;
        tokens.extend(if self.is_empty() {
            quote!(#alloy_sol_types::private::String::new())
        } else {
            quote!(#alloy_sol_types::private::ToOwned::to_owned(#self))
        })
    }
}

impl Verbatim for bool {
    #[inline]
    fn to_verbatim_tokens(&self, s: &mut TokenStream, _crates: &ExternCrates) {
        quote::ToTokens::to_tokens(self, s)
    }
}

impl Verbatim for usize {
    #[inline]
    fn to_verbatim_tokens(&self, s: &mut TokenStream, _crates: &ExternCrates) {
        quote::ToTokens::to_tokens(self, s)
    }
}

impl<T: Verbatim> Verbatim for Vec<T> {
    fn to_verbatim_tokens(&self, s: &mut TokenStream, crates: &ExternCrates) {
        let alloy_sol_types = &crates.sol_types;
        s.extend(if self.is_empty() {
            quote!(#alloy_sol_types::private::Vec::new())
        } else {
            let iter = self.iter().map(|t| ToTokensCompat(t, crates));
            quote!(#alloy_sol_types::private::vec![#(#iter),*])
        });
    }
}

impl<K: Verbatim, V: Verbatim> Verbatim for BTreeMap<K, V> {
    fn to_verbatim_tokens(&self, s: &mut TokenStream, crates: &ExternCrates) {
        let alloy_sol_types = &crates.sol_types;
        s.extend(if self.is_empty() {
            quote!(#alloy_sol_types::private::BTreeMap::new())
        } else {
            let k = self.keys().map(|t| ToTokensCompat(t, crates));
            let v = self.values().map(|t| ToTokensCompat(t, crates));
            quote!(#alloy_sol_types::private::BTreeMap::from([#( (#k, #v) ),*]))
        });
    }
}

impl<T: Verbatim> Verbatim for Option<T> {
    fn to_verbatim_tokens(&self, s: &mut TokenStream, crates: &ExternCrates) {
        let tts = match self {
            Some(t) => {
                let mut s = TokenStream::new();
                t.to_verbatim_tokens(&mut s, crates);
                quote!(::core::option::Option::Some(#s))
            }
            None => quote!(::core::option::Option::None),
        };
        s.extend(tts);
    }
}

macro_rules! derive_verbatim {
    () => {};

    (struct $name:ident { $($field:ident),* $(,)? } $($rest:tt)*) => {
        impl Verbatim for alloy_json_abi::$name {
            fn to_verbatim_tokens(&self, s: &mut TokenStream, crates: &ExternCrates) {
                let Self { $($field),* } = self;
                $(
                    let $field = ToTokensCompat($field, crates);
                )*
                let alloy_sol_types = &crates.sol_types;
                s.extend(quote! {
                    #alloy_sol_types::private::alloy_json_abi::$name {
                        $($field: #$field,)*
                    }
                });
            }
        }
        derive_verbatim!($($rest)*);
    };

    (enum $name:ident { $($variant:ident $( { $($field_idx:tt : $field:ident),* $(,)? } )?),* $(,)? } $($rest:tt)*) => {
        impl Verbatim for alloy_json_abi::$name {
            fn to_verbatim_tokens(&self, s: &mut TokenStream, crates: &ExternCrates) {
                match self {$(
                    Self::$variant $( { $($field_idx: $field),* } )? => {
                        $($(
                            let $field = ToTokensCompat($field, crates);
                        )*)?
                        let alloy_sol_types = &crates.sol_types;
                        s.extend(quote! {
                            #alloy_sol_types::private::alloy_json_abi::$name::$variant $( { $($field_idx: #$field),* } )?
                        });
                    }
                )*}
            }
        }
        derive_verbatim!($($rest)*);
    };
}

derive_verbatim! {
    struct Constructor { inputs, state_mutability }
    struct Fallback { state_mutability }
    struct Receive { state_mutability }
    struct Function { name, inputs, outputs, state_mutability }
    struct Error { name, inputs }
    struct Event { name, inputs, anonymous }
    struct Param { ty, name, components, internal_type }
    struct EventParam { ty, name, indexed, components, internal_type }

    enum InternalType {
        AddressPayable { 0: s },
        Contract { 0: s },
        Enum { contract: contract, ty: ty },
        Struct { contract: contract, ty: ty },
        Other { contract: contract, ty: ty },
    }

    enum StateMutability {
        Pure,
        View,
        NonPayable,
        Payable,
    }
}
