//! Macros for [Divan](https://github.com/nvzqz/divan), a statistically-comfy
//! benchmarking library brought to you by [Nikolai Vazquez](https://hachyderm.io/@nikolai).
//!
//! See [`divan`](https://docs.rs/divan) crate for documentation.

use proc_macro::TokenStream;
use quote::{quote, ToTokens};

mod attr_options;
mod tokens;

use attr_options::*;
use syn::{Expr, FnArg};

#[derive(Clone, Copy)]
enum Macro<'a> {
    Bench { fn_sig: &'a syn::Signature },
    BenchGroup,
}

impl Macro<'_> {
    fn name(&self) -> &'static str {
        match self {
            Self::Bench { .. } => "bench",
            Self::BenchGroup => "bench_group",
        }
    }
}

/// Lists of comma-separated `#[cfg]` parameters.
mod systems {
    use super::*;

    pub fn elf() -> proc_macro2::TokenStream {
        quote! {
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "haiku",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd"
        }
    }

    pub fn mach_o() -> proc_macro2::TokenStream {
        quote! {
            target_os = "ios",
            target_os = "macos",
            target_os = "tvos",
            target_os = "watchos"
        }
    }
}

/// Attributes applied to a `static` containing a pointer to a function to run
/// before `main`.
fn pre_main_attrs() -> proc_macro2::TokenStream {
    let elf = systems::elf();
    let mach_o = systems::mach_o();

    quote! {
        #[used]
        #[cfg_attr(windows, link_section = ".CRT$XCU")]
        #[cfg_attr(any(#elf), link_section = ".init_array")]
        #[cfg_attr(any(#mach_o), link_section = "__DATA,__mod_init_func,mod_init_funcs")]
    }
}

fn unsupported_error(attr_name: &str) -> proc_macro2::TokenStream {
    let elf = systems::elf();
    let mach_o = systems::mach_o();

    let error = format!("Unsupported target OS for `#[divan::{attr_name}]`");

    quote! {
        #[cfg(not(any(windows, #elf, #mach_o)))]
        ::std::compile_error!(#error);
    }
}

#[proc_macro_attribute]
pub fn bench(options: TokenStream, item: TokenStream) -> TokenStream {
    let option_none = tokens::option_none();
    let option_some = tokens::option_some();

    let fn_item = item.clone();
    let fn_item = syn::parse_macro_input!(fn_item as syn::ItemFn);
    let fn_sig = &fn_item.sig;

    let attr = Macro::Bench { fn_sig };
    let attr_name = attr.name();

    let options = match AttrOptions::parse(options, attr) {
        Ok(options) => options,
        Err(compile_error) => return compile_error,
    };

    // Items needed by generated code.
    let AttrOptions { private_mod, .. } = &options;

    let fn_ident = &fn_sig.ident;
    let fn_name = fn_ident.to_string();
    let fn_name_pretty = fn_name.strip_prefix("r#").unwrap_or(&fn_name);

    // Find any `#[ignore]` attribute so that we can use its span to help
    // compiler diagnostics.
    let ignore_attr_ident =
        fn_item.attrs.iter().map(|attr| attr.meta.path()).find(|path| path.is_ident("ignore"));

    // If the function is `extern "ABI"`, it is wrapped in a Rust-ABI function.
    let is_extern_abi = fn_sig.abi.is_some();

    let fn_args = &fn_sig.inputs;

    let type_param: Option<(usize, &syn::TypeParam)> = fn_sig
        .generics
        .params
        .iter()
        .enumerate()
        .filter_map(|(i, param)| match param {
            syn::GenericParam::Type(param) => Some((i, param)),
            _ => None,
        })
        .next();

    let const_param: Option<(usize, &syn::ConstParam)> = fn_sig
        .generics
        .params
        .iter()
        .enumerate()
        .filter_map(|(i, param)| match param {
            syn::GenericParam::Const(param) => Some((i, param)),
            _ => None,
        })
        .next();

    let is_type_before_const = match (type_param, const_param) {
        (Some((t, _)), Some((c, _))) => t < c,
        _ => false,
    };

    // Prefixed with "__" to prevent IDEs from recommending using this symbol.
    //
    // The static is local to intentionally cause a compile error if this
    // attribute is used multiple times on the same function.
    let static_ident = syn::Ident::new(
        &format!("__DIVAN_BENCH_{}", fn_name_pretty.to_uppercase()),
        fn_ident.span(),
    );

    let meta = entry_meta_expr(&fn_name, &options, ignore_attr_ident);

    let bench_entry_runner = quote! { #private_mod::BenchEntryRunner };

    // Creates a `__DIVAN_ARGS` global variable to be used in the entry.
    let bench_args_global = if options.args_expr.is_some() {
        quote! {
            static __DIVAN_ARGS: #private_mod::BenchArgs = #private_mod::BenchArgs::new();
        }
    } else {
        Default::default()
    };

    // The last argument type is used as the only `args` item type because we
    // currently only support one runtime argument.
    let last_arg_type = if options.args_expr.is_some() {
        fn_args.last().map(|arg| match arg {
            FnArg::Receiver(arg) => &*arg.ty,
            FnArg::Typed(arg) => &*arg.ty,
        })
    } else {
        None
    };

    let last_arg_type_tokens = last_arg_type
        .map(|ty| match ty {
            // Remove lifetime from references to not use the lifetime outside
            // of its declaration. This allows benchmarks to take arguments with
            // lifetimes.
            syn::Type::Reference(ty) if ty.lifetime.is_some() => {
                let mut ty = ty.clone();
                ty.lifetime = None;
                ty.to_token_stream()
            }

            _ => ty.to_token_stream(),
        })
        .unwrap_or_default();

    // Some argument literals need an explicit type.
    let arg_return_tokens = options
        .args_expr
        .as_ref()
        .map(|args| match args {
            // Empty array.
            Expr::Array(args) if args.elems.is_empty() => quote! {
                -> [#last_arg_type_tokens; 0]
            },

            _ => Default::default(),
        })
        .unwrap_or_default();

    // Creates a function expr for the benchmarking function, optionally
    // monomorphized with generic parameters.
    let make_bench_fn = |generics: &[&dyn ToTokens]| {
        let mut fn_expr = if generics.is_empty() {
            // Use identifier as-is.
            fn_ident.to_token_stream()
        } else {
            // Apply generic arguments.
            quote! { #fn_ident::< #(#generics),* > }
        };

        // Handle function arguments.
        match (fn_args.len(), &options.args_expr) {
            // Simple benchmark with no arguments provided.
            (0, None) => {
                // Wrap in Rust ABI.
                if is_extern_abi {
                    fn_expr = quote! { || #fn_expr() };
                }

                quote! {
                    #bench_entry_runner::Plain(|divan /* Bencher */| divan.bench(#fn_expr))
                }
            }

            // `args` option used without function arguments; handled earlier in
            // `AttrOptions::parse`.
            (0, Some(_)) => unreachable!(),

            // `Bencher` function argument.
            (1, None) => {
                // Wrap in Rust ABI.
                if is_extern_abi {
                    fn_expr = quote! { |divan /* Bencher */| #fn_expr(divan) };
                }

                quote! { #bench_entry_runner::Plain(#fn_expr) }
            }

            // Function argument comes from `args` option.
            (1, Some(args)) => quote! {
                #bench_entry_runner::Args(|| __DIVAN_ARGS.runner(
                    || #arg_return_tokens { #args },

                    |arg| #private_mod::ToStringHelper(arg).to_string(),

                    |divan, __divan_arg| divan.bench(|| #fn_expr(
                        #private_mod::Arg::<#last_arg_type_tokens>::get(__divan_arg)
                    )),
                ))
            },

            // `Bencher` and `args` option function arguments.
            (2, Some(args)) => quote! {
                #bench_entry_runner::Args(|| __DIVAN_ARGS.runner(
                    || #arg_return_tokens { #args },

                    |arg| #private_mod::ToStringHelper(arg).to_string(),

                    |divan, __divan_arg| #fn_expr(
                        divan,
                        #private_mod::Arg::<#last_arg_type_tokens>::get(__divan_arg),
                    ),
                ))
            },

            // Ensure `args` is set if arguments are provided after `Bencher`.
            (_, None) => quote! {
                ::std::compile_error!(::std::concat!(
                    "expected 'args' option containing '",
                    ::std::stringify!(#last_arg_type_tokens),
                    "'",
                ))
            },

            // `args` option used with unsupported number of arguments; handled
            // earlier in `AttrOptions::parse`.
            (_, Some(_)) => unreachable!(),
        }
    };

    let pre_main_attrs = pre_main_attrs();
    let unsupported_error = unsupported_error(attr_name);

    // Creates a `GroupEntry` static for generic benchmarks.
    let make_generic_group = |generic_benches: proc_macro2::TokenStream| {
        let entry = quote! {
            #private_mod::GroupEntry {
                meta: #meta,
                generic_benches: #option_some({ #generic_benches }),
            }
        };

        quote! {
            #unsupported_error

            // Push this static into `GROUP_ENTRIES` before `main` is called.
            static #static_ident: #private_mod::GroupEntry = {
                {
                    // Add `push` to the initializer section.
                    #pre_main_attrs
                    static PUSH: extern "C" fn() = push;

                    extern "C" fn push() {
                        static NODE: #private_mod::EntryList<#private_mod::GroupEntry>
                            = #private_mod::EntryList::new(&#static_ident);

                        #private_mod::GROUP_ENTRIES.push(&NODE);
                    }
                }

                // All generic entries share the same `BenchArgs` instance for
                // efficiency and to ensure all entries use the same values, or
                // at least the same names in the case of interior mutability.
                #bench_args_global

                #entry
            };
        }
    };

    // Creates a `GenericBenchEntry` expr for a generic benchmark instance.
    let make_generic_bench_entry =
        |ty: Option<&dyn ToTokens>, const_value: Option<&dyn ToTokens>| {
            let generic_const_value = const_value.map(|const_value| quote!({ #const_value }));

            let generics: Vec<&dyn ToTokens> = {
                let mut generics = Vec::new();

                generics.extend(generic_const_value.as_ref().map(|t| t as &dyn ToTokens));
                generics.extend(ty);

                if is_type_before_const {
                    generics.reverse();
                }

                generics
            };

            let bench_fn = make_bench_fn(&generics);

            let type_value = match ty {
                Some(ty) => quote! {
                    #option_some(#private_mod::EntryType::new::<#ty>())
                },
                None => option_none.clone(),
            };

            let const_value = match const_value {
                Some(const_value) => quote! {
                    #option_some(#private_mod::EntryConst::new(&#const_value))
                },
                None => option_none.clone(),
            };

            quote! {
                #private_mod::GenericBenchEntry {
                    group: &#static_ident,
                    bench: #bench_fn,
                    ty: #type_value,
                    const_value: #const_value,
                }
            }
        };

    let generated_items: proc_macro2::TokenStream = match &options.generic.consts {
        // Only specified `types = []` or `consts = []`; generate nothing.
        _ if options.generic.is_empty() => Default::default(),

        None => match &options.generic.types {
            // No generics; generate a simple benchmark entry.
            None => {
                let bench_fn = make_bench_fn(&[]);

                let entry = quote! {
                    #private_mod::BenchEntry {
                        meta: #meta,
                        bench: #bench_fn,
                    }
                };

                quote! {
                    // Push this static into `BENCH_ENTRIES` before `main` is
                    // called.
                    static #static_ident: #private_mod::BenchEntry = {
                        {
                            // Add `push` to the initializer section.
                            #pre_main_attrs
                            static PUSH: extern "C" fn() = push;

                            extern "C" fn push() {
                                static NODE: #private_mod::EntryList<#private_mod::BenchEntry>
                                    = #private_mod::EntryList::new(&#static_ident);

                                #private_mod::BENCH_ENTRIES.push(&NODE);
                            }
                        }

                        #bench_args_global

                        #entry
                    };
                }
            }

            // Generate a benchmark group entry with generic benchmark entries.
            Some(GenericTypes::List(generic_types)) => {
                let generic_benches =
                    generic_types.iter().map(|ty| make_generic_bench_entry(Some(&ty), None));

                make_generic_group(quote! {
                    &[&[#(#generic_benches),*]]
                })
            }
        },

        // Generate a benchmark group entry with generic benchmark entries.
        Some(Expr::Array(generic_consts)) => {
            let consts_count = generic_consts.elems.len();
            let const_type = &const_param.unwrap().1.ty;

            let generic_benches = options.generic.types_iter().map(|ty| {
                let generic_benches = (0..consts_count).map(move |i| {
                    let const_value = quote! { __DIVAN_CONSTS[#i] };
                    make_generic_bench_entry(ty, Some(&const_value))
                });

                // `static` is necessary because `EntryConst` uses interior
                // mutability to cache the `ToString` result.
                quote! {
                    static __DIVAN_GENERIC_BENCHES: [#private_mod::GenericBenchEntry; #consts_count] = [#(#generic_benches),*];
                    &__DIVAN_GENERIC_BENCHES
                }
            });

            make_generic_group(quote! {
                // We refer to our own slice because it:
                // - Type-checks values, even if `generic_benches` is empty
                //   because the user set `types = []`
                // - Prevents re-computing constants, which can slightly improve
                //   compile time given that Miri is slow
                const __DIVAN_CONSTS: &[#const_type] = &#generic_consts;

                &[#({ #generic_benches }),*]
            })
        }

        // Generate a benchmark group entry with generic benchmark entries over
        // an expression of constants.
        //
        // This is limited to a maximum of 20 because we need some constant to
        // instantiate each function instance.
        Some(generic_consts) => {
            // The maximum number of elements for non-array expressions.
            const MAX_EXTERN_COUNT: usize = 20;

            let const_type = &const_param.unwrap().1.ty;

            let generic_benches = options.generic.types_iter().map(|ty| {
                let generic_benches = (0..MAX_EXTERN_COUNT).map(move |i| {
                    let const_value = quote! {
                        // Fallback to the first constant if out of bounds.
                        __DIVAN_CONSTS[if #i < __DIVAN_CONST_COUNT { #i } else { 0 }]
                    };
                    make_generic_bench_entry(ty, Some(&const_value))
                });

                // `static` is necessary because `EntryConst` uses interior
                // mutability to cache the `ToString` result.
                quote! {
                    static __DIVAN_GENERIC_BENCHES: [#private_mod::GenericBenchEntry; __DIVAN_CONST_COUNT]
                        = match #private_mod::shrink_array([#(#generic_benches),*]) {
                            Some(array) => array,
                            _ => panic!("external 'consts' cannot contain more than 20 values"),
                        };

                    &__DIVAN_GENERIC_BENCHES
                }
            });

            make_generic_group(quote! {
                const __DIVAN_CONST_COUNT: usize = __DIVAN_CONSTS.len();
                const __DIVAN_CONSTS: &[#const_type] = &#generic_consts;

                &[#({ #generic_benches }),*]
            })
        }
    };

    // Append our generated code to the existing token stream.
    let mut result = item;
    result.extend(TokenStream::from(generated_items));
    result
}

#[proc_macro_attribute]
pub fn bench_group(options: TokenStream, item: TokenStream) -> TokenStream {
    let attr = Macro::BenchGroup;
    let attr_name = attr.name();

    let options = match AttrOptions::parse(options, attr) {
        Ok(options) => options,
        Err(compile_error) => return compile_error,
    };

    // Items needed by generated code.
    let AttrOptions { private_mod, .. } = &options;

    let option_none = tokens::option_none();

    // TODO: Make module parsing cheaper by parsing only the necessary parts.
    let mod_item = item.clone();
    let mod_item = syn::parse_macro_input!(mod_item as syn::ItemMod);

    let mod_ident = &mod_item.ident;
    let mod_name = mod_ident.to_string();
    let mod_name_pretty = mod_name.strip_prefix("r#").unwrap_or(&mod_name);

    // Find any `#[ignore]` attribute so that we can use its span to help
    // compiler diagnostics.
    //
    // TODO: Fix `unused_attributes` warning when using `#[ignore]` on a module.
    let ignore_attr_ident =
        mod_item.attrs.iter().map(|attr| attr.meta.path()).find(|path| path.is_ident("ignore"));

    // Prefixed with "__" to prevent IDEs from recommending using this symbol.
    //
    // By having the static be local, we cause a compile error if this attribute
    // is used multiple times on the same function.
    let static_ident = syn::Ident::new(
        &format!("__DIVAN_GROUP_{}", mod_name_pretty.to_uppercase()),
        mod_ident.span(),
    );

    let meta = entry_meta_expr(&mod_name, &options, ignore_attr_ident);

    let pre_main_attrs = pre_main_attrs();
    let unsupported_error = unsupported_error(attr_name);

    let generated_items = quote! {
        #unsupported_error

        // Push this static into `GROUP_ENTRIES` before `main` is called.
        static #static_ident: #private_mod::EntryList<#private_mod::GroupEntry> = {
            {
                // Add `push` to the initializer section.
                #pre_main_attrs
                static PUSH: extern "C" fn() = push;

                extern "C" fn push() {
                    #private_mod::GROUP_ENTRIES.push(&#static_ident);
                }
            }

            #private_mod::EntryList::new({
                static #static_ident: #private_mod::GroupEntry = #private_mod::GroupEntry {
                    meta: #meta,
                    generic_benches: #option_none,
                };

                &#static_ident
            })
        };
    };

    // Append our generated code to the existing token stream.
    let mut result = item;
    result.extend(TokenStream::from(generated_items));
    result
}

/// Constructs an `EntryMeta` expression.
fn entry_meta_expr(
    raw_name: &str,
    options: &AttrOptions,
    ignore_attr_ident: Option<&syn::Path>,
) -> proc_macro2::TokenStream {
    let AttrOptions { private_mod, .. } = &options;

    let raw_name_pretty = raw_name.strip_prefix("r#").unwrap_or(raw_name);

    let display_name: &dyn ToTokens = match &options.name_expr {
        Some(name) => name,
        None => &raw_name_pretty,
    };

    let bench_options = options.bench_options_fn(ignore_attr_ident);

    quote! {
        #private_mod::EntryMeta {
            raw_name: #raw_name,
            display_name: #display_name,
            bench_options: #bench_options,
            module_path: ::std::module_path!(),

            // `Span` location info is nightly-only, so use macros.
            location: #private_mod::EntryLocation {
                file: ::std::file!(),
                line: ::std::line!(),
                col: ::std::column!(),
            },
        }
    }
}
