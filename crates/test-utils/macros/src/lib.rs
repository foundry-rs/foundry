//! # foundry-test-macros
//!
//! Internal Foundry testing proc-macros.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Error, Expr, FnArg, Pat, Result, Signature, Type, Visibility,
    parse::{Parse, ParseStream},
    parse_quote,
};

/// Defines a Forge integration test.
///
/// The function may take a `prj: _` argument for the `TestProject` and a `cmd: _` argument for the
/// `forge` `TestCommand`, in any order. Either can be left out. An `async` function runs on
/// a multi-threaded Tokio runtime.
///
/// The attribute optionally takes the project's `PathStyle`, which defaults to
/// `PathStyle::Dapptools`.
///
/// ```ignore
/// #[forgetest]
/// fn can_build(prj: _, cmd: _) {
///     cmd.arg("build").assert_success();
/// }
///
/// #[forgetest(PathStyle::HardHat)]
/// async fn can_build_hardhat(cmd: _) {
///     cmd.arg("build").assert_success();
/// }
/// ```
#[proc_macro_attribute]
pub fn forgetest(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr, item, Kind::Forge)
}

/// Same as [`macro@forgetest`], but initializes the project with `forge init` first.
#[proc_macro_attribute]
pub fn forgetest_init(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr, item, Kind::ForgeInit)
}

/// Same as [`macro@forgetest`], but `cmd` is a `cast` command.
#[proc_macro_attribute]
pub fn casttest(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr, item, Kind::Cast)
}

/// A function whose body is kept as unparsed tokens.
struct TestFn {
    attrs: Vec<Attribute>,
    vis: Visibility,
    sig: Signature,
    block: TokenStream2,
}

impl Parse for TestFn {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        Ok(Self {
            attrs: input.call(Attribute::parse_outer)?,
            vis: input.parse()?,
            sig: input.parse()?,
            block: input.parse()?,
        })
    }
}

enum Kind {
    Forge,
    ForgeInit,
    Cast,
}

fn expand(attr: TokenStream, item: TokenStream, kind: Kind) -> TokenStream {
    test_fn(attr, item, kind).unwrap_or_else(Error::into_compile_error).into()
}

fn test_fn(attr: TokenStream, item: TokenStream, kind: Kind) -> Result<TokenStream2> {
    let style = (!attr.is_empty()).then(|| syn::parse::<Expr>(attr)).transpose()?;
    let TestFn { attrs, vis, mut sig, block } = syn::parse(item)?;

    // The body runs in an inner function that takes the requested arguments, so that unused ones
    // are linted.
    let mut inner = sig.clone();
    let mut args = Vec::new();
    for input in &mut inner.inputs {
        let FnArg::Typed(arg) = input else {
            return Err(Error::new_spanned(input, "expected `prj: _` or `cmd: _`"));
        };
        let Pat::Ident(pat) = &mut *arg.pat else {
            return Err(Error::new_spanned(&arg.pat, "expected `prj` or `cmd`"));
        };
        if !matches!(*arg.ty, Type::Infer(_)) {
            return Err(Error::new_spanned(&arg.ty, "expected `_`"));
        }
        let ty = match pat.ident.to_string().as_str() {
            "prj" => parse_quote!(::foundry_test_utils::TestProject),
            "cmd" => parse_quote!(::foundry_test_utils::TestCommand),
            _ => return Err(Error::new_spanned(&pat.ident, "expected `prj` or `cmd`")),
        };
        args.push(pat.ident.clone());
        pat.mutability = Some(syn::Token![mut](pat.ident.span()));
        arg.attrs.push(parse_quote!(#[allow(unused_mut)]));
        *arg.ty = ty;
    }
    sig.inputs.clear();

    let ident = &sig.ident;
    let name = ident.to_string();
    let style = style.unwrap_or_else(|| {
        parse_quote!(::foundry_test_utils::foundry_compilers::PathStyle::Dapptools)
    });
    let setup = match kind {
        Kind::Forge | Kind::ForgeInit => quote!(setup_forge),
        Kind::Cast => quote!(setup_cast),
    };
    let init = matches!(kind, Kind::ForgeInit)
        .then(|| quote!(::foundry_test_utils::util::initialize(prj.root());));
    let (test, dot_await) = if sig.asyncness.is_some() {
        (quote!(#[::tokio::test(flavor = "multi_thread")]), Some(quote!(.await)))
    } else {
        (quote!(#[test]), None)
    };

    Ok(quote! {
        #(#attrs)*
        #test
        #vis #sig {
            #inner #block
            let (prj, cmd) = ::foundry_test_utils::util::#setup(#name, #style);
            #init
            #ident(#(#args),*) #dot_await
        }
    })
}
