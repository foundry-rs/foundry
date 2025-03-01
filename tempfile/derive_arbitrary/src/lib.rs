extern crate proc_macro;

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::*;

mod container_attributes;
mod field_attributes;
mod variant_attributes;

use container_attributes::ContainerAttributes;
use field_attributes::{determine_field_constructor, FieldConstructor};
use variant_attributes::not_skipped;

const ARBITRARY_ATTRIBUTE_NAME: &str = "arbitrary";
const ARBITRARY_LIFETIME_NAME: &str = "'arbitrary";

#[proc_macro_derive(Arbitrary, attributes(arbitrary))]
pub fn derive_arbitrary(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(tokens as syn::DeriveInput);
    expand_derive_arbitrary(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_derive_arbitrary(input: syn::DeriveInput) -> Result<TokenStream> {
    let container_attrs = ContainerAttributes::from_derive_input(&input)?;

    let (lifetime_without_bounds, lifetime_with_bounds) =
        build_arbitrary_lifetime(input.generics.clone());

    let recursive_count = syn::Ident::new(
        &format!("RECURSIVE_COUNT_{}", input.ident),
        Span::call_site(),
    );

    let arbitrary_method =
        gen_arbitrary_method(&input, lifetime_without_bounds.clone(), &recursive_count)?;
    let size_hint_method = gen_size_hint_method(&input)?;
    let name = input.ident;

    // Apply user-supplied bounds or automatic `T: ArbitraryBounds`.
    let generics = apply_trait_bounds(
        input.generics,
        lifetime_without_bounds.clone(),
        &container_attrs,
    )?;

    // Build ImplGeneric with a lifetime (https://github.com/dtolnay/syn/issues/90)
    let mut generics_with_lifetime = generics.clone();
    generics_with_lifetime
        .params
        .push(GenericParam::Lifetime(lifetime_with_bounds));
    let (impl_generics, _, _) = generics_with_lifetime.split_for_impl();

    // Build TypeGenerics and WhereClause without a lifetime
    let (_, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        const _: () = {
            ::std::thread_local! {
                #[allow(non_upper_case_globals)]
                static #recursive_count: ::core::cell::Cell<u32> = ::core::cell::Cell::new(0);
            }

            #[automatically_derived]
            impl #impl_generics arbitrary::Arbitrary<#lifetime_without_bounds> for #name #ty_generics #where_clause {
                #arbitrary_method
                #size_hint_method
            }
        };
    })
}

// Returns: (lifetime without bounds, lifetime with bounds)
// Example: ("'arbitrary", "'arbitrary: 'a + 'b")
fn build_arbitrary_lifetime(generics: Generics) -> (LifetimeParam, LifetimeParam) {
    let lifetime_without_bounds =
        LifetimeParam::new(Lifetime::new(ARBITRARY_LIFETIME_NAME, Span::call_site()));
    let mut lifetime_with_bounds = lifetime_without_bounds.clone();

    for param in generics.params.iter() {
        if let GenericParam::Lifetime(lifetime_def) = param {
            lifetime_with_bounds
                .bounds
                .push(lifetime_def.lifetime.clone());
        }
    }

    (lifetime_without_bounds, lifetime_with_bounds)
}

fn apply_trait_bounds(
    mut generics: Generics,
    lifetime: LifetimeParam,
    container_attrs: &ContainerAttributes,
) -> Result<Generics> {
    // If user-supplied bounds exist, apply them to their matching type parameters.
    if let Some(config_bounds) = &container_attrs.bounds {
        let mut config_bounds_applied = 0;
        for param in generics.params.iter_mut() {
            if let GenericParam::Type(type_param) = param {
                if let Some(replacement) = config_bounds
                    .iter()
                    .flatten()
                    .find(|p| p.ident == type_param.ident)
                {
                    *type_param = replacement.clone();
                    config_bounds_applied += 1;
                } else {
                    // If no user-supplied bounds exist for this type, delete the original bounds.
                    // This mimics serde.
                    type_param.bounds = Default::default();
                    type_param.default = None;
                }
            }
        }
        let config_bounds_supplied = config_bounds
            .iter()
            .map(|bounds| bounds.len())
            .sum::<usize>();
        if config_bounds_applied != config_bounds_supplied {
            return Err(Error::new(
                Span::call_site(),
                format!(
                    "invalid `{}` attribute. too many bounds, only {} out of {} are applicable",
                    ARBITRARY_ATTRIBUTE_NAME, config_bounds_applied, config_bounds_supplied,
                ),
            ));
        }
        Ok(generics)
    } else {
        // Otherwise, inject a `T: Arbitrary` bound for every parameter.
        Ok(add_trait_bounds(generics, lifetime))
    }
}

// Add a bound `T: Arbitrary` to every type parameter T.
fn add_trait_bounds(mut generics: Generics, lifetime: LifetimeParam) -> Generics {
    for param in generics.params.iter_mut() {
        if let GenericParam::Type(type_param) = param {
            type_param
                .bounds
                .push(parse_quote!(arbitrary::Arbitrary<#lifetime>));
        }
    }
    generics
}

fn with_recursive_count_guard(
    recursive_count: &syn::Ident,
    expr: impl quote::ToTokens,
) -> impl quote::ToTokens {
    quote! {
        let guard_against_recursion = u.is_empty();
        if guard_against_recursion {
            #recursive_count.with(|count| {
                if count.get() > 0 {
                    return Err(arbitrary::Error::NotEnoughData);
                }
                count.set(count.get() + 1);
                Ok(())
            })?;
        }

        let result = (|| { #expr })();

        if guard_against_recursion {
            #recursive_count.with(|count| {
                count.set(count.get() - 1);
            });
        }

        result
    }
}

fn gen_arbitrary_method(
    input: &DeriveInput,
    lifetime: LifetimeParam,
    recursive_count: &syn::Ident,
) -> Result<TokenStream> {
    fn arbitrary_structlike(
        fields: &Fields,
        ident: &syn::Ident,
        lifetime: LifetimeParam,
        recursive_count: &syn::Ident,
    ) -> Result<TokenStream> {
        let arbitrary = construct(fields, |_idx, field| gen_constructor_for_field(field))?;
        let body = with_recursive_count_guard(recursive_count, quote! { Ok(#ident #arbitrary) });

        let arbitrary_take_rest = construct_take_rest(fields)?;
        let take_rest_body =
            with_recursive_count_guard(recursive_count, quote! { Ok(#ident #arbitrary_take_rest) });

        Ok(quote! {
            fn arbitrary(u: &mut arbitrary::Unstructured<#lifetime>) -> arbitrary::Result<Self> {
                #body
            }

            fn arbitrary_take_rest(mut u: arbitrary::Unstructured<#lifetime>) -> arbitrary::Result<Self> {
                #take_rest_body
            }
        })
    }

    fn arbitrary_variant(
        index: u64,
        enum_name: &Ident,
        variant_name: &Ident,
        ctor: TokenStream,
    ) -> TokenStream {
        quote! { #index => #enum_name::#variant_name #ctor }
    }

    fn arbitrary_enum_method(
        recursive_count: &syn::Ident,
        unstructured: TokenStream,
        variants: &[TokenStream],
    ) -> impl quote::ToTokens {
        let count = variants.len() as u64;
        with_recursive_count_guard(
            recursive_count,
            quote! {
                // Use a multiply + shift to generate a ranged random number
                // with slight bias. For details, see:
                // https://lemire.me/blog/2016/06/30/fast-random-shuffling
                Ok(match (u64::from(<u32 as arbitrary::Arbitrary>::arbitrary(#unstructured)?) * #count) >> 32 {
                    #(#variants,)*
                    _ => unreachable!()
                })
            },
        )
    }

    fn arbitrary_enum(
        DataEnum { variants, .. }: &DataEnum,
        enum_name: &Ident,
        lifetime: LifetimeParam,
        recursive_count: &syn::Ident,
    ) -> Result<TokenStream> {
        let filtered_variants = variants.iter().filter(not_skipped);

        // Check attributes of all variants:
        filtered_variants
            .clone()
            .try_for_each(check_variant_attrs)?;

        // From here on, we can assume that the attributes of all variants were checked.
        let enumerated_variants = filtered_variants
            .enumerate()
            .map(|(index, variant)| (index as u64, variant));

        // Construct `match`-arms for the `arbitrary` method.
        let variants = enumerated_variants
            .clone()
            .map(|(index, Variant { fields, ident, .. })| {
                construct(fields, |_, field| gen_constructor_for_field(field))
                    .map(|ctor| arbitrary_variant(index, enum_name, ident, ctor))
            })
            .collect::<Result<Vec<TokenStream>>>()?;

        // Construct `match`-arms for the `arbitrary_take_rest` method.
        let variants_take_rest = enumerated_variants
            .map(|(index, Variant { fields, ident, .. })| {
                construct_take_rest(fields)
                    .map(|ctor| arbitrary_variant(index, enum_name, ident, ctor))
            })
            .collect::<Result<Vec<TokenStream>>>()?;

        // Most of the time, `variants` is not empty (the happy path),
        //   thus `variants_take_rest` will be used,
        //   so no need to move this check before constructing `variants_take_rest`.
        // If `variants` is empty, this will emit a compiler-error.
        (!variants.is_empty())
            .then(|| {
                // TODO: Improve dealing with `u` vs. `&mut u`.
                let arbitrary = arbitrary_enum_method(recursive_count, quote! { u }, &variants);
                let arbitrary_take_rest = arbitrary_enum_method(recursive_count, quote! { &mut u }, &variants_take_rest);

                quote! {
                    fn arbitrary(u: &mut arbitrary::Unstructured<#lifetime>) -> arbitrary::Result<Self> {
                        #arbitrary
                    }

                    fn arbitrary_take_rest(mut u: arbitrary::Unstructured<#lifetime>) -> arbitrary::Result<Self> {
                        #arbitrary_take_rest
                    }
                }
            })
            .ok_or_else(|| Error::new_spanned(
                enum_name,
                "Enum must have at least one variant, that is not skipped"
            ))
    }

    let ident = &input.ident;
    match &input.data {
        Data::Struct(data) => arbitrary_structlike(&data.fields, ident, lifetime, recursive_count),
        Data::Union(data) => arbitrary_structlike(
            &Fields::Named(data.fields.clone()),
            ident,
            lifetime,
            recursive_count,
        ),
        Data::Enum(data) => arbitrary_enum(data, ident, lifetime, recursive_count),
    }
}

fn construct(
    fields: &Fields,
    ctor: impl Fn(usize, &Field) -> Result<TokenStream>,
) -> Result<TokenStream> {
    let output = match fields {
        Fields::Named(names) => {
            let names: Vec<TokenStream> = names
                .named
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let name = f.ident.as_ref().unwrap();
                    ctor(i, f).map(|ctor| quote! { #name: #ctor })
                })
                .collect::<Result<_>>()?;
            quote! { { #(#names,)* } }
        }
        Fields::Unnamed(names) => {
            let names: Vec<TokenStream> = names
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, f)| ctor(i, f).map(|ctor| quote! { #ctor }))
                .collect::<Result<_>>()?;
            quote! { ( #(#names),* ) }
        }
        Fields::Unit => quote!(),
    };
    Ok(output)
}

fn construct_take_rest(fields: &Fields) -> Result<TokenStream> {
    construct(fields, |idx, field| {
        determine_field_constructor(field).map(|field_constructor| match field_constructor {
            FieldConstructor::Default => quote!(::core::default::Default::default()),
            FieldConstructor::Arbitrary => {
                if idx + 1 == fields.len() {
                    quote! { arbitrary::Arbitrary::arbitrary_take_rest(u)? }
                } else {
                    quote! { arbitrary::Arbitrary::arbitrary(&mut u)? }
                }
            }
            FieldConstructor::With(function_or_closure) => quote!((#function_or_closure)(&mut u)?),
            FieldConstructor::Value(value) => quote!(#value),
        })
    })
}

fn gen_size_hint_method(input: &DeriveInput) -> Result<TokenStream> {
    let size_hint_fields = |fields: &Fields| {
        fields
            .iter()
            .map(|f| {
                let ty = &f.ty;
                determine_field_constructor(f).map(|field_constructor| {
                    match field_constructor {
                        FieldConstructor::Default | FieldConstructor::Value(_) => {
                            quote!(Ok((0, Some(0))))
                        }
                        FieldConstructor::Arbitrary => {
                            quote! { <#ty as arbitrary::Arbitrary>::try_size_hint(depth) }
                        }

                        // Note that in this case it's hard to determine what size_hint must be, so size_of::<T>() is
                        // just an educated guess, although it's gonna be inaccurate for dynamically
                        // allocated types (Vec, HashMap, etc.).
                        FieldConstructor::With(_) => {
                            quote! { Ok((::core::mem::size_of::<#ty>(), None)) }
                        }
                    }
                })
            })
            .collect::<Result<Vec<TokenStream>>>()
            .map(|hints| {
                quote! {
                    Ok(arbitrary::size_hint::and_all(&[
                        #( #hints? ),*
                    ]))
                }
            })
    };
    let size_hint_structlike = |fields: &Fields| {
        size_hint_fields(fields).map(|hint| {
            quote! {
                #[inline]
                fn size_hint(depth: usize) -> (usize, ::core::option::Option<usize>) {
                    Self::try_size_hint(depth).unwrap_or_default()
                }

                #[inline]
                fn try_size_hint(depth: usize) -> ::core::result::Result<(usize, ::core::option::Option<usize>), arbitrary::MaxRecursionReached> {
                    arbitrary::size_hint::try_recursion_guard(depth, |depth| #hint)
                }
            }
        })
    };
    match &input.data {
        Data::Struct(data) => size_hint_structlike(&data.fields),
        Data::Union(data) => size_hint_structlike(&Fields::Named(data.fields.clone())),
        Data::Enum(data) => data
            .variants
            .iter()
            .filter(not_skipped)
            .map(|Variant { fields, .. }| {
                // The attributes of all variants are checked in `gen_arbitrary_method` above
                //   and can therefore assume that they are valid.
                size_hint_fields(fields)
            })
            .collect::<Result<Vec<TokenStream>>>()
            .map(|variants| {
                quote! {
                    fn size_hint(depth: usize) -> (usize, ::core::option::Option<usize>) {
                        Self::try_size_hint(depth).unwrap_or_default()
                    }
                    #[inline]
                    fn try_size_hint(depth: usize) -> ::core::result::Result<(usize, ::core::option::Option<usize>), arbitrary::MaxRecursionReached> {
                        Ok(arbitrary::size_hint::and(
                            <u32 as arbitrary::Arbitrary>::try_size_hint(depth)?,
                            arbitrary::size_hint::try_recursion_guard(depth, |depth| {
                                Ok(arbitrary::size_hint::or_all(&[ #( #variants? ),* ]))
                            })?,
                        ))
                    }
                }
            }),
    }
}

fn gen_constructor_for_field(field: &Field) -> Result<TokenStream> {
    let ctor = match determine_field_constructor(field)? {
        FieldConstructor::Default => quote!(::core::default::Default::default()),
        FieldConstructor::Arbitrary => quote!(arbitrary::Arbitrary::arbitrary(u)?),
        FieldConstructor::With(function_or_closure) => quote!((#function_or_closure)(u)?),
        FieldConstructor::Value(value) => quote!(#value),
    };
    Ok(ctor)
}

fn check_variant_attrs(variant: &Variant) -> Result<()> {
    for attr in &variant.attrs {
        if attr.path().is_ident(ARBITRARY_ATTRIBUTE_NAME) {
            return Err(Error::new_spanned(
                attr,
                format!(
                    "invalid `{}` attribute. it is unsupported on enum variants. try applying it to a field of the variant instead",
                    ARBITRARY_ATTRIBUTE_NAME
                ),
            ));
        }
    }
    Ok(())
}
