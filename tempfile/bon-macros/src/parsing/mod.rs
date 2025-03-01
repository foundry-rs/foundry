mod docs;
mod item_sig;
mod simple_closure;
mod spanned_key;

pub(crate) use docs::*;
pub(crate) use item_sig::*;
pub(crate) use simple_closure::*;
pub(crate) use spanned_key::*;

use crate::util::prelude::*;
use darling::FromMeta;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

pub(crate) fn parse_non_empty_paren_meta_list<T: FromMeta>(meta: &syn::Meta) -> Result<T> {
    require_non_empty_paren_meta_list_or_name_value(meta)?;
    T::from_meta(meta)
}

pub(crate) fn require_non_empty_paren_meta_list_or_name_value(meta: &syn::Meta) -> Result {
    match meta {
        syn::Meta::List(meta) => {
            meta.require_parens_delim()?;

            if meta.tokens.is_empty() {
                bail!(
                    &meta.delimiter.span().join(),
                    "expected parameters in parentheses"
                );
            }
        }
        syn::Meta::Path(path) => bail!(
            &meta,
            "this empty `{0}` attribute is unexpected; \
            remove it or pass parameters in parentheses: \
            `{0}(...)`",
            darling::util::path_to_string(path)
        ),
        syn::Meta::NameValue(_) => {}
    }

    Ok(())
}

/// Utility for parsing with `#[darling(with = ...)]` attribute that allows to
/// parse an arbitrary sequence of items inside of parentheses. For example
/// `foo(a, b, c)`, where `a`, `b`, and `c` are of type `T` and `,` is represented
/// by the token type `P`.
#[allow(dead_code)]
pub(crate) fn parse_paren_meta_list_with_terminated<T, P>(
    meta: &syn::Meta,
) -> Result<Punctuated<T, P>>
where
    T: syn::parse::Parse,
    P: syn::parse::Parse,
{
    let item = std::any::type_name::<T>();
    let punct = std::any::type_name::<P>();

    let name = |val: &str| {
        format!(
            "'{}'",
            val.rsplit("::").next().unwrap_or(val).to_lowercase()
        )
    };

    let meta = match meta {
        syn::Meta::List(meta) => meta,
        _ => bail!(
            &meta,
            "expected a list of {} separated by {}",
            name(item),
            name(punct),
        ),
    };

    meta.require_parens_delim()?;

    let punctuated = Punctuated::parse_terminated.parse2(meta.tokens.clone())?;

    Ok(punctuated)
}

fn parse_path_mod_style(meta: &syn::Meta) -> Result<syn::Path> {
    let expr = match meta {
        syn::Meta::NameValue(meta) => &meta.value,
        _ => bail!(meta, "expected a simple path, like `foo::bar`"),
    };

    Ok(expr.require_path_mod_style()?.clone())
}

pub(crate) fn parse_bon_crate_path(meta: &syn::Meta) -> Result<syn::Path> {
    let path = parse_path_mod_style(meta)?;

    let prefix = &path
        .segments
        .first()
        .ok_or_else(|| err!(&path, "path must have at least one segment"))?
        .ident;

    let is_absolute = path.leading_colon.is_some() || prefix == "crate" || prefix == "$crate";

    if is_absolute {
        return Ok(path);
    }

    if prefix == "super" || prefix == "self" {
        bail!(
            &path,
            "path must not be relative; specify the path that starts with `crate::` \
            instead; if you want to refer to a reexport from an external crate then \
            use a leading colon like `::crate_name::reexport::path::bon`"
        )
    }

    let path_str = darling::util::path_to_string(&path);

    bail!(
        &path,
        "path must be absolute; if you want to refer to a reexport from an external \
        crate then add a leading colon like `::{path_str}`; if the path leads to a module \
        in the current crate, then specify the absolute path with `crate` like \
        `crate::reexport::path::bon` or `$crate::reexport::path::bon` (if within a macro)"
    )
}

// Lint from nightly. `&Option<T>` is used to reduce syntax at the callsite
#[allow(unknown_lints, clippy::ref_option)]
pub(crate) fn reject_syntax<T: Spanned>(name: &'static str, syntax: &Option<T>) -> Result {
    if let Some(syntax) = syntax {
        bail!(syntax, "{name} is not allowed here")
    }

    Ok(())
}
