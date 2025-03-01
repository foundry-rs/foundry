use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, Parser},
    spanned::Spanned,
    Expr, ExprArray, Ident, Token, Type,
};

use crate::{tokens, Macro};

/// Values from parsed options shared between `#[divan::bench]` and
/// `#[divan::bench_group]`.
///
/// The `crate` option is not included because it is only needed to get proper
/// access to `__private`.
pub(crate) struct AttrOptions {
    /// `divan::__private`.
    pub private_mod: proc_macro2::TokenStream,

    /// Custom name for the benchmark or group.
    pub name_expr: Option<Expr>,

    /// `IntoIterator` from which to provide runtime arguments.
    pub args_expr: Option<Expr>,

    /// Options for generic functions.
    pub generic: GenericOptions,

    /// The `BenchOptions.counters` field and its value, followed by a comma.
    pub counters: proc_macro2::TokenStream,

    /// Options used directly as `BenchOptions` fields.
    ///
    /// Option reuse is handled by the compiler ensuring `BenchOptions` fields
    /// are not repeated.
    pub bench_options: Vec<(Ident, Expr)>,
}

impl AttrOptions {
    pub fn parse(tokens: TokenStream, target_macro: Macro) -> Result<Self, TokenStream> {
        let macro_name = target_macro.name();

        let mut divan_crate = None::<syn::Path>;
        let mut name_expr = None::<Expr>;
        let mut args_expr = None::<Expr>;
        let mut bench_options = Vec::new();

        let mut counters = Vec::<(proc_macro2::TokenStream, Option<&str>)>::new();
        let mut counters_ident = None::<Ident>;

        let mut seen_bytes_count = false;
        let mut seen_chars_count = false;
        let mut seen_cycles_count = false;
        let mut seen_items_count = false;

        let mut generic = GenericOptions::default();

        let attr_parser = syn::meta::parser(|meta| {
            macro_rules! error {
                ($($t:tt)+) => {
                    return Err(meta.error(format_args!($($t)+)))
                };
            }

            let Some(ident) = meta.path.get_ident() else {
                error!("unsupported '{macro_name}' option");
            };

            let ident_name = ident.to_string();
            let ident_name = ident_name.strip_prefix("r#").unwrap_or(&ident_name);

            let repeat_error = || error!("repeated '{macro_name}' option '{ident_name}'");
            let unsupported_error = || error!("unsupported '{macro_name}' option '{ident_name}'");

            macro_rules! parse {
                ($storage:expr) => {
                    if $storage.is_none() {
                        $storage = Some(meta.value()?.parse()?);
                    } else {
                        return repeat_error();
                    }
                };
            }

            match ident_name {
                "crate" => parse!(divan_crate),
                "name" => parse!(name_expr),
                "types" => {
                    match target_macro {
                        Macro::Bench { fn_sig } => {
                            if fn_sig.generics.type_params().next().is_none() {
                                error!("generic type required for '{macro_name}' option '{ident_name}'");
                            }
                        }
                        _ => return unsupported_error(),
                    }

                    parse!(generic.types);
                }
                "consts" => {
                    match target_macro {
                        Macro::Bench { fn_sig } => {
                            if fn_sig.generics.const_params().next().is_none() {
                                error!("generic const required for '{macro_name}' option '{ident_name}'");
                            }
                        }
                        _ => return unsupported_error(),
                    }

                    parse!(generic.consts);
                }
                "args" => {
                    match target_macro {
                        Macro::Bench { fn_sig } => {
                            if !matches!(fn_sig.inputs.len(), 1 | 2) {
                                return Err(meta.error(format_args!("function argument required for '{macro_name}' option '{ident_name}'")));
                            }
                        }
                        _ => return unsupported_error(),
                    }

                    parse!(args_expr);
                }
                "counter" => {
                    if counters_ident.is_some() {
                        return repeat_error();
                    }
                    let value: Expr = meta.value()?.parse()?;
                    counters.push((value.into_token_stream(), None));
                    counters_ident = Some(Ident::new("counters", ident.span()));
                }
                "counters" => {
                    if counters_ident.is_some() {
                        return repeat_error();
                    }
                    let values: ExprArray = meta.value()?.parse()?;
                    counters.extend(
                        values.elems.into_iter().map(|elem| (elem.into_token_stream(), None)),
                    );
                    counters_ident = Some(ident.clone());
                }

                "bytes_count" if seen_bytes_count => return repeat_error(),
                "chars_count" if seen_chars_count => return repeat_error(),
                "cycles_count" if seen_cycles_count => return repeat_error(),
                "items_count" if seen_items_count => return repeat_error(),

                "bytes_count" | "chars_count" | "cycles_count" | "items_count" => {
                    let name = match ident_name {
                        "bytes_count" => {
                            seen_bytes_count = true;
                            "BytesCount"
                        }
                        "chars_count" => {
                            seen_chars_count = true;
                            "CharsCount"
                        }
                        "cycles_count" => {
                            seen_cycles_count = true;
                            "CyclesCount"
                        }
                        "items_count" => {
                            seen_items_count = true;
                            "ItemsCount"
                        }
                        _ => unreachable!(),
                    };

                    let value: Expr = meta.value()?.parse()?;
                    counters.push((value.into_token_stream(), Some(name)));
                    counters_ident = Some(Ident::new("counters", proc_macro2::Span::call_site()));
                }

                _ => {
                    let value: Expr = match meta.value() {
                        Ok(value) => value.parse()?,

                        // If the option is missing `=`, use a `true` literal.
                        Err(_) => Expr::Lit(syn::ExprLit {
                            lit: syn::LitBool::new(true, meta.path.span()).into(),
                            attrs: Vec::new(),
                        }),
                    };

                    bench_options.push((ident.clone(), value));
                }
            }

            Ok(())
        });

        match attr_parser.parse(tokens) {
            Ok(()) => {}
            Err(error) => return Err(error.into_compile_error().into()),
        }

        let divan_crate = divan_crate.unwrap_or_else(|| syn::parse_quote!(::divan));
        let private_mod = quote! { #divan_crate::__private };

        let counters = counters.iter().map(|(expr, type_name)| match type_name {
            Some(type_name) => {
                let type_name = Ident::new(type_name, proc_macro2::Span::call_site());
                quote! {
                    // We do a scoped import for the expression to override any
                    // local `From` trait.
                    {
                        use ::std::convert::From as _;

                        #divan_crate::counter::#type_name::from(#expr)
                    }
                }
            }
            None => expr.to_token_stream(),
        });

        let counters = counters_ident
            .map(|ident| {
                quote! {
                    #ident: #private_mod::new_counter_set() #(.with(#counters))* ,
                }
            })
            .unwrap_or_default();

        Ok(Self { private_mod, name_expr, args_expr, generic, counters, bench_options })
    }

    /// Produces a function expression for creating `LazyLock<BenchOptions>`.
    ///
    /// If the `#[ignore]` attribute is specified, this be provided its
    /// identifier to set `BenchOptions` using its span. Doing this instead of
    /// creating the `ignore` identifier ourselves improves compiler error
    /// diagnostics.
    pub fn bench_options_fn(
        &self,
        ignore_attr_ident: Option<&syn::Path>,
    ) -> proc_macro2::TokenStream {
        fn is_lit_array(expr: &Expr) -> bool {
            let Expr::Array(expr) = expr else {
                return false;
            };
            expr.elems.iter().all(|elem| matches!(elem, Expr::Lit { .. }))
        }

        let private_mod = &self.private_mod;
        let option_some = tokens::option_some();

        // Directly set fields on `BenchOptions`. This simplifies things by:
        // - Having a single source of truth
        // - Making unknown options a compile error
        //
        // We use `..` (struct update syntax) to ensure that no option is set
        // twice, even if raw identifiers are used. This also has the accidental
        // benefit of Rust Analyzer recognizing fields and emitting suggestions
        // with docs and type info.
        if self.bench_options.is_empty() && self.counters.is_empty() && ignore_attr_ident.is_none()
        {
            tokens::option_none()
        } else {
            let options_iter = self.bench_options.iter().map(|(option, value)| {
                let option_name = option.to_string();
                let option_name = option_name.strip_prefix("r#").unwrap_or(&option_name);

                let wrapped_value: proc_macro2::TokenStream;
                let value: &dyn ToTokens = match option_name {
                    "threads" => {
                        wrapped_value = if is_lit_array(value) {
                            // If array of literals, just use `&[...]`.
                            quote! { ::std::borrow::Cow::Borrowed(&#value) }
                        } else {
                            quote! { #private_mod::IntoThreads::into_threads(#value) }
                        };

                        &wrapped_value
                    }

                    // If the option is a `Duration`, use `IntoDuration` to be
                    // polymorphic over `Duration` or `u64`/`f64` seconds.
                    "min_time" | "max_time" => {
                        wrapped_value =
                            quote! { #private_mod::IntoDuration::into_duration(#value) };
                        &wrapped_value
                    }

                    _ => value,
                };

                quote! { #option: #option_some(#value), }
            });

            let ignore = match ignore_attr_ident {
                Some(ignore_attr_ident) => quote! { #ignore_attr_ident: #option_some(true), },
                None => Default::default(),
            };

            let counters = &self.counters;

            quote! {
                #option_some(::std::sync::LazyLock::new(|| {
                    #[allow(clippy::needless_update)]
                    #private_mod::BenchOptions {
                        #(#options_iter)*

                        // Ignore comes after options so that options take
                        // priority in compiler error diagnostics.
                        #ignore

                        #counters

                        ..::std::default::Default::default()
                    }
                }))
            }
        }
    }
}

/// Options for generic functions.
#[derive(Default)]
pub struct GenericOptions {
    /// Generic types over which to instantiate benchmark functions.
    pub types: Option<GenericTypes>,

    /// `const` array/slice over which to instantiate benchmark functions.
    pub consts: Option<Expr>,
}

impl GenericOptions {
    /// Returns `true` if set exclusively to either:
    /// - `types = []`
    /// - `consts = []`
    pub fn is_empty(&self) -> bool {
        match (&self.types, &self.consts) {
            (Some(types), None) => types.is_empty(),
            (None, Some(Expr::Array(consts))) => consts.elems.is_empty(),
            _ => false,
        }
    }

    /// Returns an iterator of multiple `Some` for types, or a single `None` if
    /// there are no types.
    pub fn types_iter(&self) -> Box<dyn Iterator<Item = Option<&dyn ToTokens>> + '_> {
        match &self.types {
            None => Box::new(std::iter::once(None)),
            Some(GenericTypes::List(types)) => {
                Box::new(types.iter().map(|t| Some(t as &dyn ToTokens)))
            }
        }
    }
}

/// Generic types over which to instantiate benchmark functions.
pub enum GenericTypes {
    /// List of types, e.g. `[i32, String, ()]`.
    List(Vec<proc_macro2::TokenStream>),
}

impl Parse for GenericTypes {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::bracketed!(content in input);

        Ok(Self::List(
            content
                .parse_terminated(Type::parse, Token![,])?
                .into_iter()
                .map(|ty| ty.into_token_stream())
                .collect(),
        ))
    }
}

impl GenericTypes {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::List(list) => list.is_empty(),
        }
    }
}
