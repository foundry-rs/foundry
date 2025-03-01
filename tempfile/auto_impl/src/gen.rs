use proc_macro2::{Span as Span2, TokenStream as TokenStream2, TokenTree as TokenTree2};
use quote::{ToTokens, TokenStreamExt};
use syn::{
    punctuated::Punctuated, spanned::Spanned, Attribute, Error, FnArg, GenericParam, Ident,
    ItemTrait, Lifetime, Pat, PatIdent, PatType, ReturnType, Signature, Token, TraitBound,
    TraitBoundModifier, TraitItem, TraitItemConst, TraitItemFn, TraitItemType, Type,
    TypeParamBound, WherePredicate,
};

use crate::{
    analyze::find_suitable_param_names,
    attr::{is_our_attr, parse_our_attr, OurAttr},
    proxy::ProxyType,
};

/// Generates one complete impl of the given trait for each of the given proxy
/// types. All impls are returned as token stream.
pub(crate) fn gen_impls(
    proxy_types: &[ProxyType],
    trait_def: &syn::ItemTrait,
) -> syn::Result<TokenStream2> {
    let mut tokens = TokenStream2::new();

    let (proxy_ty_param, proxy_lt_param) = find_suitable_param_names(trait_def);

    // One impl for each proxy type
    for proxy_type in proxy_types {
        let header = gen_header(proxy_type, trait_def, &proxy_ty_param, &proxy_lt_param)?;
        let items = gen_items(proxy_type, trait_def, &proxy_ty_param)?;

        if let ProxyType::Box | ProxyType::Rc | ProxyType::Arc = proxy_type {
            tokens.append_all(quote! {
                const _: () = {
                    extern crate alloc;
                    #header { #( #items )* }
                };
            });
        } else {
            tokens.append_all(quote! {
                const _: () = {
                    #header { #( #items )* }
                };
            });
        }
    }

    Ok(tokens)
}

/// Generates the header of the impl of the given trait for the given proxy
/// type.
fn gen_header(
    proxy_type: &ProxyType,
    trait_def: &ItemTrait,
    proxy_ty_param: &Ident,
    proxy_lt_param: &Lifetime,
) -> syn::Result<TokenStream2> {
    // Generate generics for impl positions from trait generics.
    let (impl_generics, trait_generics, where_clause) = trait_def.generics.split_for_impl();

    // The name of the trait with all generic parameters applied.
    let trait_ident = &trait_def.ident;
    let trait_path = quote! { #trait_ident #trait_generics };

    // Here we assemble the parameter list of the impl (the thing in
    // `impl< ... >`). This is simply the parameter list of the trait with
    // one or two parameters added. For a trait `trait Foo<'x, 'y, A, B>`,
    // it will look like this:
    //
    //    '{proxy_lt_param}, 'x, 'y, A, B, {proxy_ty_param}
    //
    // The `'{proxy_lt_param}` in the beginning is only added when the proxy
    // type is `&` or `&mut`.
    let impl_generics = {
        // Determine whether we can add a `?Sized` relaxation to allow trait
        // objects. We can do that as long as there is no method that has a
        // `self` by value receiver and no `where Self: Sized` bound.
        let methods = trait_def
            .items
            .iter()
            // Only interested in methods
            .filter_map(|item| {
                if let TraitItem::Fn(m) = item {
                    Some(m)
                } else {
                    None
                }
            });

        let mut sized_required = false;
        for m in methods {
            if should_keep_default_for(m, proxy_type)? {
                continue;
            }

            // Check if there is a `Self: Sized` bound on the method.
            let self_is_bounded_sized = m
                .sig
                .generics
                .where_clause
                .iter()
                .flat_map(|wc| &wc.predicates)
                .filter_map(|pred| {
                    if let WherePredicate::Type(p) = pred {
                        Some(p)
                    } else {
                        None
                    }
                })
                .any(|pred| {
                    // Check if the type is `Self`
                    match &pred.bounded_ty {
                        Type::Path(p) if p.path.is_ident("Self") => {
                            // Check if the bound contains `Sized`
                            pred.bounds.iter().any(|b| match b {
                                TypeParamBound::Trait(TraitBound {
                                    modifier: TraitBoundModifier::None,
                                    path,
                                    ..
                                }) => path.is_ident("Sized"),
                                _ => false,
                            })
                        }
                        _ => false,
                    }
                });

            // Check if the first parameter is `self` by value. In that
            // case, we might require `Self` to be `Sized`.
            let self_value_param = match m.sig.inputs.first() {
                Some(FnArg::Receiver(receiver)) => receiver.reference.is_none(),
                _ => false,
            };

            // Check if return type is `Self`
            let self_value_return = match &m.sig.output {
                ReturnType::Type(_, t) => {
                    if let Type::Path(p) = &**t {
                        p.path.is_ident("Self")
                    } else {
                        false
                    }
                }
                _ => false,
            };

            // TODO: check for `Self` parameter in any other argument.

            // If for this method, `Self` is used in a position that
            // requires `Self: Sized` or this bound is added explicitly, we
            // cannot add the `?Sized` relaxation to the impl body.
            if self_value_param || self_value_return || self_is_bounded_sized {
                sized_required = true;
                break;
            }
        }

        let relaxation = if sized_required {
            quote! {}
        } else {
            quote! { + ?::core::marker::Sized }
        };

        // Check if there are some `Self: Foo` bounds on methods. If so, we
        // need to add those bounds to `T` as well. See issue #11 for more
        // information, but in short: there is no better solution. Method where
        // clauses with `Self: Foo` force us to add `T: Foo` to the impl
        // header, as we otherwise cannot generate a valid impl block.
        let methods = trait_def
            .items
            .iter()
            // Only interested in methods
            .filter_map(|item| {
                if let TraitItem::Fn(m) = item {
                    Some(m)
                } else {
                    None
                }
            });

        let mut additional_bounds = Vec::new();
        for m in methods {
            if should_keep_default_for(m, proxy_type)? {
                continue;
            }

            additional_bounds.extend(
                m.sig
                    .generics
                    .where_clause
                    .iter()
                    .flat_map(|wc| &wc.predicates)
                    .filter_map(|pred| {
                        if let WherePredicate::Type(p) = pred {
                            Some(p)
                        } else {
                            None
                        }
                    })
                    .filter(|p| {
                        // Only `Self:` bounds
                        match &p.bounded_ty {
                            Type::Path(p) => p.path.is_ident("Self"),
                            _ => false,
                        }
                    })
                    .flat_map(|p| &p.bounds)
                    .filter_map(|b| {
                        // We are only interested in trait bounds. That's
                        // because lifetime bounds on `Self` do not need to be
                        // added to the impl header. That's because all values
                        // "derived" from `self` also meet the same lifetime
                        // bound as `self`. In simpler terms: while `self.field`
                        // might not be `Clone` even if `Self: Clone`,
                        // `self.field` is always `: 'a` if `Self: 'a`.
                        match b {
                            TypeParamBound::Trait(t) => Some(t),
                            _ => None,
                        }
                    }),
            );
        }

        // Determine if our proxy type needs a lifetime parameter
        let (mut params, ty_bounds) = match proxy_type {
            ProxyType::Ref | ProxyType::RefMut => (
                quote! { #proxy_lt_param, },
                quote! { : #proxy_lt_param + #trait_path #relaxation #(+ #additional_bounds)* },
            ),
            ProxyType::Box | ProxyType::Rc | ProxyType::Arc => (
                quote! {},
                quote! { : #trait_path #relaxation #(+ #additional_bounds)* },
            ),
            ProxyType::Fn | ProxyType::FnMut | ProxyType::FnOnce => {
                let fn_bound = gen_fn_type_for_trait(proxy_type, trait_def)?;
                (quote! {}, quote! { : #fn_bound })
            }
        };

        // Append all parameters from the trait. Sadly, `impl_generics`
        // includes the angle brackets `< >` so we have to remove them like
        // this.
        let mut tts = impl_generics
            .into_token_stream()
            .into_iter()
            .skip(1) // the opening `<`
            .collect::<Vec<_>>();
        tts.pop(); // the closing `>`

        // Pop a trailing comma if there is one
        // We'll end up conditionally putting one back in shortly
        if tts.last().and_then(|tt| {
            if let TokenTree2::Punct(p) = tt {
                Some(p.as_char())
            } else {
                None
            }
        }) == Some(',')
        {
            tts.pop();
        }

        params.append_all(&tts);

        // Append proxy type parameter (if there aren't any parameters so far,
        // we need to add a comma first).
        let comma = if params.is_empty() || tts.is_empty() {
            quote! {}
        } else {
            quote! { , }
        };
        params.append_all(quote! { #comma #proxy_ty_param #ty_bounds });

        params
    };

    // The tokens after `for` in the impl header (the type the trait is
    // implemented for).
    #[rustfmt::skip]
    let self_ty = match *proxy_type {
        ProxyType::Ref      => quote! { & #proxy_lt_param #proxy_ty_param },
        ProxyType::RefMut   => quote! { & #proxy_lt_param mut #proxy_ty_param },
        ProxyType::Arc      => quote! { alloc::sync::Arc<#proxy_ty_param> },
        ProxyType::Rc       => quote! { alloc::rc::Rc<#proxy_ty_param> },
        ProxyType::Box      => quote! { alloc::boxed::Box<#proxy_ty_param> },
        ProxyType::Fn       => quote! { #proxy_ty_param },
        ProxyType::FnMut    => quote! { #proxy_ty_param },
        ProxyType::FnOnce   => quote! { #proxy_ty_param },
    };

    // If the trait has super traits, we need to add the super trait bound to
    // our self type. This can only be done in the where clause, so we need to
    // combine the existing where clauses with our new predicate in that case.
    let where_clause = if !trait_def.supertraits.is_empty() {
        let mut out = quote! { where };

        if !trait_def.supertraits.is_empty() {
            let supertraits = &trait_def.supertraits;
            out.extend(quote! { #self_ty: #supertraits, });
        }
        if let Some(predicates) = where_clause.map(|c| &c.predicates) {
            out.extend(predicates.into_token_stream());
        }

        out
    } else {
        where_clause.into_token_stream()
    };

    // Combine everything
    Ok(quote! {
        impl<#impl_generics> #trait_path for #self_ty #where_clause
    })
}

/// Generates the Fn-trait type (e.g. `FnMut(u32) -> String`) for the given
/// trait and proxy type (the latter has to be `Fn`, `FnMut` or `FnOnce`!)
///
/// If the trait is unsuitable to be implemented for the given proxy type, an
/// error is emitted.
fn gen_fn_type_for_trait(
    proxy_type: &ProxyType,
    trait_def: &ItemTrait,
) -> syn::Result<TokenStream2> {
    // Only traits with exactly one method can be implemented for Fn-traits.
    // Associated types and consts are also not allowed.
    let method = trait_def.items.first().and_then(|item| {
        if let TraitItem::Fn(m) = item {
            Some(m)
        } else {
            None
        }
    });

    // If this requirement is not satisfied, we emit an error.
    if method.is_none() || trait_def.items.len() > 1 {
        return Err(Error::new(
            trait_def.span(),
            "this trait cannot be auto-implemented for Fn-traits (only traits with exactly \
                one method and no other items are allowed)",
        ));
    }

    // We checked for `None` above
    let method = method.unwrap();
    let sig = &method.sig;

    // Check for forbidden modifier of the method
    if let Some(const_token) = sig.constness {
        return Err(Error::new(
            const_token.span(),
            format_args!(
                "the trait '{}' cannot be auto-implemented for Fn-traits: const methods are not \
                allowed",
                trait_def.ident
            ),
        ));
    }

    if let Some(unsafe_token) = &sig.unsafety {
        return Err(Error::new(
            unsafe_token.span(),
            format_args!(
                "the trait '{}' cannot be auto-implemented for Fn-traits: unsafe methods are not \
                allowed",
                trait_def.ident
            ),
        ));
    }

    if let Some(abi_token) = &sig.abi {
        return Err(Error::new(
            abi_token.span(),
            format_args!(
                "the trait '{}' cannot be implemented for Fn-traits: custom ABIs are not allowed",
                trait_def.ident
            ),
        ));
    }

    // Function traits cannot support generics in their arguments
    // These would require HRTB for types instead of just lifetimes
    let mut r: syn::Result<()> = Ok(());
    for type_param in sig.generics.type_params() {
        let err = Error::new(
            type_param.span(),
            format_args!("the trait '{}' cannot be implemented for Fn-traits: generic arguments are not allowed",
            trait_def.ident),
        );

        if let Err(ref mut current_err) = r {
            current_err.combine(err);
        } else {
            r = Err(err);
        }
    }

    for const_param in sig.generics.const_params() {
        let err = Error::new(
            const_param.span(),
            format_args!("the trait '{}' cannot be implemented for Fn-traits: constant arguments are not allowed",
            trait_def.ident),
        );

        if let Err(ref mut current_err) = r {
            current_err.combine(err);
        } else {
            r = Err(err);
        }
    }
    r?;

    // =======================================================================
    // Check if the trait can be implemented for the given proxy type
    let self_type = SelfType::from_sig(sig);
    let err = match (self_type, proxy_type) {
        // The method needs to have a receiver
        (SelfType::None, _) => Some(("Fn-traits", "no", "")),

        // We can't impl methods with `&mut self` or `&self` receiver for
        // `FnOnce`
        (SelfType::Mut, ProxyType::FnOnce) => {
            Some(("`FnOnce`", "a `&mut self`", " (only `self` is allowed)"))
        }
        (SelfType::Ref, ProxyType::FnOnce) => {
            Some(("`FnOnce`", "a `&self`", " (only `self` is allowed)"))
        }

        // We can't impl methods with `&self` receiver for `FnMut`
        (SelfType::Ref, ProxyType::FnMut) => Some((
            "`FnMut`",
            "a `&self`",
            " (only `self` and `&mut self` are allowed)",
        )),

        // Other combinations are fine
        _ => None,
    };

    if let Some((fn_traits, receiver, allowed)) = err {
        return Err(Error::new(
            method.sig.span(),
            format_args!(
                "the trait '{}' cannot be auto-implemented for {}, because this method has \
                {} receiver{}",
                trait_def.ident, fn_traits, receiver, allowed,
            ),
        ));
    }

    // =======================================================================
    // Generate the full Fn-type

    // The path to the Fn-trait
    let fn_name = match proxy_type {
        ProxyType::Fn => quote! { ::core::ops::Fn },
        ProxyType::FnMut => quote! { ::core::ops::FnMut },
        ProxyType::FnOnce => quote! { ::core::ops::FnOnce },
        _ => panic!("internal error in auto_impl (function contract violation)"),
    };

    // The return type
    let ret = &sig.output;

    // Now it gets a bit complicated. The types of the function signature
    // could contain "local" lifetimes, meaning that they are not declared in
    // the trait definition (or are `'static`). We need to extract all local
    // lifetimes to declare them with HRTB (e.g. `for<'a>`).
    //
    // In Rust 2015 that was easy: we could just take the lifetimes explicitly
    // declared in the function signature. Those were the local lifetimes.
    // Unfortunately, with in-band lifetimes, things get more complicated. We
    // need to take a look at all lifetimes inside the types (arbitrarily deep)
    // and check if they are local or not.
    //
    // In cases where lifetimes are omitted (e.g. `&str`), we don't have a
    // problem. If we just translate that to `for<> Fn(&str)`, it's fine: all
    // omitted lifetimes in an `Fn()` type are automatically declared as HRTB.
    //
    // TODO: Implement this check for in-band lifetimes!
    let local_lifetimes = sig.generics.lifetimes();

    // The input types as comma separated list. We skip the first argument, as
    // this is the receiver argument.
    let mut arg_types = TokenStream2::new();
    for arg in sig.inputs.iter().skip(1) {
        match arg {
            FnArg::Typed(pat) => {
                let ty = &pat.ty;
                arg_types.append_all(quote! { #ty , });
            }

            // We skipped the receiver already.
            FnArg::Receiver(r) => {
                return Err(Error::new(
                    r.span(),
                    "receiver argument that's not the first argument (auto_impl is confused)",
                ));
            }
        }
    }

    Ok(quote! {
        for< #(#local_lifetimes),* > #fn_name (#arg_types) #ret
    })
}

/// Generates the implementation of all items of the given trait. These
/// implementations together are the body of the `impl` block.
fn gen_items(
    proxy_type: &ProxyType,
    trait_def: &ItemTrait,
    proxy_ty_param: &Ident,
) -> syn::Result<Vec<TokenStream2>> {
    trait_def
        .items
        .iter()
        .map(|item| {
            match item {
                TraitItem::Const(c) => gen_const_item(proxy_type, c, trait_def, proxy_ty_param),
                TraitItem::Fn(method) => {
                    gen_method_item(proxy_type, method, trait_def, proxy_ty_param)
                }
                TraitItem::Type(ty) => gen_type_item(proxy_type, ty, trait_def, proxy_ty_param),
                TraitItem::Macro(mac) => {
                    // We cannot resolve the macro invocation and thus cannot know
                    // if it adds additional items to the trait. Thus, we have to
                    // give up.
                    Err(Error::new(
                        mac.span(),
                        "traits with macro invocations in their bodies are not \
                        supported by auto_impl",
                    ))
                }
                TraitItem::Verbatim(v) => {
                    // I don't quite know when this happens, but it's better to
                    // notify the user with a nice error instead of panicking.
                    Err(Error::new(
                        v.span(),
                        "unexpected 'verbatim'-item (auto-impl doesn't know how to handle it)",
                    ))
                }
                _ => {
                    // `syn` enums are `non_exhaustive` to be future-proof. If a
                    // trait contains a kind of item we don't even know about, we
                    // emit an error.
                    Err(Error::new(
                        item.span(),
                        "unknown trait item (auto-impl doesn't know how to handle it)",
                    ))
                }
            }
        })
        .collect()
}

/// Generates the implementation of an associated const item described by
/// `item`. The implementation is returned as token stream.
///
/// If the proxy type is an Fn*-trait, an error is emitted and `Err(())` is
/// returned.
fn gen_const_item(
    proxy_type: &ProxyType,
    item: &TraitItemConst,
    trait_def: &ItemTrait,
    proxy_ty_param: &Ident,
) -> syn::Result<TokenStream2> {
    // A trait with associated consts cannot be implemented for Fn* types.
    if proxy_type.is_fn() {
        return Err(Error::new(
            item.span(),
            format_args!("the trait `{}` cannot be auto-implemented for Fn-traits, because it has \
                associated consts (only traits with a single method can be implemented for Fn-traits)",
            trait_def.ident)
        ));
    }

    // We simply use the associated const from our type parameter.
    let const_name = &item.ident;
    let const_ty = &item.ty;
    let attrs = filter_attrs(&item.attrs);

    Ok(quote! {
        #(#attrs)* const #const_name: #const_ty = #proxy_ty_param::#const_name;
    })
}

/// Generates the implementation of an associated type item described by `item`.
/// The implementation is returned as token stream.
///
/// If the proxy type is an Fn*-trait, an error is emitted and `Err(())` is
/// returned.
fn gen_type_item(
    proxy_type: &ProxyType,
    item: &TraitItemType,
    trait_def: &ItemTrait,
    proxy_ty_param: &Ident,
) -> syn::Result<TokenStream2> {
    // A trait with associated types cannot be implemented for Fn* types.
    if proxy_type.is_fn() {
        return Err(Error::new(
            item.span(),
            format_args!("the trait `{}` cannot be auto-implemented for Fn-traits, because it has \
                associated types (only traits with a single method can be implemented for Fn-traits)",
                trait_def.ident)
        ));
    }

    // We simply use the associated type from our type parameter.
    let assoc_name = &item.ident;
    let attrs = filter_attrs(&item.attrs);
    let generics = &item.generics;

    Ok(quote! {
        #(#attrs)* type #assoc_name #generics = #proxy_ty_param::#assoc_name #generics;
    })
}

/// Generates the implementation of a method item described by `item`. The
/// implementation is returned as token stream.
///
/// This function also performs sanity checks, e.g. whether the proxy type can
/// be used to implement the method. If any error occurs, the error is
/// immediately emitted and `Err(())` is returned.
fn gen_method_item(
    proxy_type: &ProxyType,
    item: &TraitItemFn,
    trait_def: &ItemTrait,
    proxy_ty_param: &Ident,
) -> syn::Result<TokenStream2> {
    // If this method has a `#[auto_impl(keep_default_for(...))]` attribute for
    // the given proxy type, we don't generate anything for this impl block.
    if should_keep_default_for(item, proxy_type)? {
        return if item.default.is_some() {
            Ok(TokenStream2::new())
        } else {
            Err(Error::new(
                item.sig.span(),
                format_args!(
                    "the method `{}` has the attribute `keep_default_for` but is not a default \
                    method (no body is provided)",
                    item.sig.ident
                ),
            ))
        };
    }

    // Determine the kind of the method, determined by the self type.
    let sig = &item.sig;
    let self_arg = SelfType::from_sig(sig);
    let attrs = filter_attrs(&item.attrs);

    // Check self type and proxy type combination
    check_receiver_compatible(proxy_type, self_arg, &trait_def.ident, sig.span())?;

    // Generate the list of argument used to call the method.
    let (inputs, args) = get_arg_list(sig.inputs.iter())?;

    // Construct a signature we'll use to generate the proxy method impl
    // This is _almost_ the same as the original, except we use the inputs constructed
    // alongside the args. These may be slightly different than the original trait.
    let sig = Signature {
        constness: item.sig.constness,
        asyncness: item.sig.asyncness,
        unsafety: item.sig.unsafety,
        abi: item.sig.abi.clone(),
        fn_token: item.sig.fn_token,
        ident: item.sig.ident.clone(),
        generics: item.sig.generics.clone(),
        paren_token: item.sig.paren_token,
        inputs,
        variadic: item.sig.variadic.clone(),
        output: item.sig.output.clone(),
    };

    // Build the turbofish type parameters. We need to pass type parameters
    // explicitly as they cannot be inferred in all cases (e.g. something like
    // `mem::size_of`). However, we don't explicitly specify lifetime
    // parameters. Most lifetime parameters are so called late-bound lifetimes
    // (ones that stick to input parameters) and Rust prohibits us from
    // specifying late-bound lifetimes explicitly (which is not a problem,
    // because those can always be correctly inferred). It would be possible to
    // explicitly specify early-bound lifetimes, but this is hardly useful.
    // Early-bound lifetimes are lifetimes that are only attached to the return
    // type. Something like:
    //
    //     fn foo<'a>() -> &'a i32
    //
    // It's hard to imagine how such a function would even work. So since those
    // functions are really rare and special, we won't support them. In
    // particular, for us to determine if a lifetime parameter is early- or
    // late-bound would be *really* difficult.
    //
    // So we just specify type parameters. In the future, however, we need to
    // add support for const parameters. But those are not remotely stable yet,
    // so we can wait a bit still.
    let generic_types = sig
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Type(param) => {
                let name = &param.ident;
                Some(quote! { #name , })
            }
            GenericParam::Const(param) => {
                let name = &param.ident;
                Some(quote! { #name , })
            }
            GenericParam::Lifetime(_) => None,
        })
        .collect::<TokenStream2>();

    let generic_types = if generic_types.is_empty() {
        generic_types
    } else {
        quote! { ::<#generic_types> }
    };

    // Generate the body of the function. This mainly depends on the self type,
    // but also on the proxy type.
    let fn_name = &sig.ident;
    let await_token = sig.asyncness.map(|_| quote! { .await });

    let body = match self_arg {
        // Fn proxy types get a special treatment
        _ if proxy_type.is_fn() => {
            quote! { ({self})(#args) #await_token }
        }

        // No receiver
        SelfType::None => {
            // The proxy type is a reference, smart pointer or Box.
            quote! { #proxy_ty_param::#fn_name #generic_types(#args) #await_token }
        }

        // Receiver `self` (by value)
        SelfType::Value => {
            // The proxy type is a Box.
            quote! { #proxy_ty_param::#fn_name #generic_types(*self, #args) #await_token }
        }

        // `&self` or `&mut self` receiver
        SelfType::Ref | SelfType::Mut => {
            // The proxy type could be anything in the `Ref` case, and `&mut`
            // or Box in the `Mut` case.
            quote! { #proxy_ty_param::#fn_name #generic_types(self, #args) #await_token }
        }
    };

    // Combine body with signature
    Ok(quote! { #(#attrs)* #sig { #body }})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelfType {
    None,
    Ref,
    Mut,
    Value,
}

impl SelfType {
    fn from_sig(sig: &Signature) -> Self {
        match sig.inputs.iter().next() {
            Some(FnArg::Receiver(r)) => {
                if r.reference.is_none() {
                    SelfType::Value
                } else if r.mutability.is_none() {
                    SelfType::Ref
                } else {
                    SelfType::Mut
                }
            }
            _ => SelfType::None,
        }
    }

    fn as_str(&self) -> Option<&'static str> {
        match *self {
            SelfType::None => None,
            SelfType::Ref => Some("&self"),
            SelfType::Mut => Some("&mut self"),
            SelfType::Value => Some("self"),
        }
    }
}

/// Checks if this method can be implemented for the given proxy type. If not,
/// we will emit an error pointing to the method signature.
fn check_receiver_compatible(
    proxy_type: &ProxyType,
    self_arg: SelfType,
    trait_name: &Ident,
    sig_span: Span2,
) -> syn::Result<()> {
    match (proxy_type, self_arg) {
        (ProxyType::Ref, SelfType::Mut) | (ProxyType::Ref, SelfType::Value) => {
            Err(Error::new(
                sig_span,
                format_args!("the trait `{}` cannot be auto-implemented for immutable references, because \
                    this method has a `{}` receiver (only `&self` and no receiver are allowed)",
                    trait_name,
                    self_arg.as_str().unwrap()),
            ))
        }

        (ProxyType::RefMut, SelfType::Value) => {
            Err(Error::new(
                sig_span,
                format_args!("the trait `{}` cannot be auto-implemented for mutable references, because \
                    this method has a `self` receiver (only `&self`, `&mut self` and no receiver are allowed)",
                trait_name,)
            ))
        }

        (ProxyType::Rc, SelfType::Mut)
        | (ProxyType::Rc, SelfType::Value)
        | (ProxyType::Arc, SelfType::Mut)
        | (ProxyType::Arc, SelfType::Value) => {
            let ptr_name = if *proxy_type == ProxyType::Rc {
                "Rc"
            } else {
                "Arc"
            };

            Err(Error::new(
                sig_span,
                format_args!("the trait `{}` cannot be auto-implemented for {}, because \
                    this method has a `{}` receiver (only `&self` and no receiver are allowed)",
                    trait_name,
                    ptr_name,
                    self_arg.as_str().unwrap())
            ))
        }

        (ProxyType::Fn, _) | (ProxyType::FnMut, _) | (ProxyType::FnOnce, _) => {
            // The Fn-trait being compatible with the receiver was already
            // checked before (in `gen_fn_type_for_trait()`).
            Ok(())
        }

        _ => Ok(()) // All other combinations are fine
    }
}

/// Generates a list of comma-separated arguments used to call the function.
/// Currently, only simple names are valid and more complex pattern will lead
/// to an error being emitted. `self` parameters are ignored.
fn get_arg_list<'a>(
    original_inputs: impl Iterator<Item = &'a FnArg>,
) -> syn::Result<(Punctuated<FnArg, Token![,]>, TokenStream2)> {
    let mut args = TokenStream2::new();
    let mut inputs = Punctuated::new();

    let mut r: Result<(), Error> = Ok(());
    for arg in original_inputs {
        match arg {
            FnArg::Typed(arg) => {
                // Make sure the argument pattern is a simple name. In
                // principle, we could probably support patterns, but it's
                // not that important now.
                if let Pat::Ident(PatIdent {
                    by_ref: _by_ref,
                    mutability: _mutability,
                    ident,
                    subpat: None,
                    attrs,
                }) = &*arg.pat
                {
                    // Add name plus trailing comma to tokens
                    args.append_all(quote! { #ident , });

                    // Add an input argument that omits the `mut` and `ref` pattern bindings
                    inputs.push(FnArg::Typed(PatType {
                        attrs: arg.attrs.clone(),
                        pat: Box::new(Pat::Ident(PatIdent {
                            attrs: attrs.clone(),
                            by_ref: None,
                            mutability: None,
                            ident: ident.clone(),
                            subpat: None,
                        })),
                        colon_token: arg.colon_token,
                        ty: arg.ty.clone(),
                    }))
                } else {
                    let err = Error::new(
                        arg.pat.span(), "argument patterns are not supported by #[auto-impl]; use a simple name like \"foo\" (but not `_`)"
                    );

                    if let Err(ref mut current_err) = r {
                        current_err.combine(err);
                    } else {
                        r = Err(err);
                    }

                    continue;
                }
            }

            // There is only one such argument. We handle it elsewhere and
            // can ignore it here.
            FnArg::Receiver(arg) => {
                inputs.push(FnArg::Receiver(arg.clone()));
            }
        }
    }

    r.map(|_| (inputs, args))
}

/// Checks if the given method has the attribute `#[auto_impl(keep_default_for(...))]`
/// and if it contains the given proxy type.
fn should_keep_default_for(m: &TraitItemFn, proxy_type: &ProxyType) -> syn::Result<bool> {
    // Get an iterator of just the attribute we are interested in.
    let mut it = m
        .attrs
        .iter()
        .filter(|attr| is_our_attr(attr))
        .map(parse_our_attr);

    // Check the first (and hopefully only) `keep_default_for` attribute.
    let out = match it.next() {
        Some(attr) => {
            // Check if the attribute lists the given proxy type.
            let OurAttr::KeepDefaultFor(proxy_types) = attr?;
            proxy_types.contains(proxy_type)
        }

        // If there is no such attribute, we return `false`
        None => false,
    };

    // Check if there is another such attribute (which we disallow)
    if it.next().is_some() {
        return Err(Error::new(
            m.sig.span(),
            "found two `keep_default_for` attributes on one method",
        ));
    }

    Ok(out)
}

fn filter_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .cloned()
        .collect()
}
