//! Internal attributes of the form `#[auto_impl(name(...))]` that can be
//! attached to trait items.

use proc_macro2::{Delimiter, TokenTree};
use syn::{
    spanned::Spanned,
    visit_mut::{visit_item_trait_mut, VisitMut},
    Attribute, Error, Meta, TraitItem,
};

use crate::proxy::{parse_types, ProxyType};

/// Removes all `#[auto_impl]` attributes that are attached to methods of the
/// given trait.
pub(crate) fn remove_our_attrs(trait_def: &mut syn::ItemTrait) -> syn::Result<()> {
    struct AttrRemover(syn::Result<()>);
    impl VisitMut for AttrRemover {
        fn visit_trait_item_mut(&mut self, item: &mut TraitItem) {
            let item_span = item.span();
            let (attrs, is_method) = match item {
                TraitItem::Fn(m) => (&mut m.attrs, true),
                TraitItem::Const(c) => (&mut c.attrs, false),
                TraitItem::Type(t) => (&mut t.attrs, false),
                TraitItem::Macro(m) => (&mut m.attrs, false),
                _ => {
                    let err = syn::Error::new(
                        item.span(),
                        "encountered unexpected `TraitItem`, cannot handle that, sorry!",
                    );

                    if let Err(ref mut current_err) = self.0 {
                        current_err.combine(err);
                    } else {
                        self.0 = Err(err);
                    };

                    return;
                }
            };

            // Make sure non-methods do not have our attributes.
            if !is_method && attrs.iter().any(is_our_attr) {
                let err = syn::Error::new(
                    item_span,
                    "`#[auto_impl]` attributes are only allowed on methods",
                );

                if let Err(ref mut current_err) = self.0 {
                    current_err.combine(err);
                } else {
                    self.0 = Err(err);
                };

                return;
            }

            attrs.retain(|a| !is_our_attr(a));
        }
    }

    let mut visitor = AttrRemover(Ok(()));
    visit_item_trait_mut(&mut visitor, trait_def);

    visitor.0
}

/// Checks if the given attribute is "our" attribute. That means that it's path
/// is `auto_impl`.
pub(crate) fn is_our_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("auto_impl")
}

/// Tries to parse the given attribute as one of our own `auto_impl`
/// attributes. If it's invalid, an error is emitted and `Err(())` is returned.
/// You have to make sure that `attr` is one of our attrs with `is_our_attr`
/// before calling this function!
pub(crate) fn parse_our_attr(attr: &Attribute) -> syn::Result<OurAttr> {
    assert!(is_our_attr(attr));

    // Get the body of the attribute (which has to be a ground, because we
    // required the syntax `auto_impl(...)` and forbid stuff like
    // `auto_impl = ...`).
    let body = match &attr.meta {
        Meta::List(list) => list.tokens.clone(),
        _ => {
            return Err(Error::new(
                attr.span(),
                "expected single group delimited by `()`",
            ));
        }
    };

    let mut it = body.clone().into_iter();

    // Try to extract the name (we require the body to be `name(...)`).
    let name = match it.next() {
        Some(TokenTree::Ident(x)) => x,
        Some(other) => {
            return Err(Error::new(
                other.span(),
                format_args!("expected ident, found '{}'", other),
            ));
        }
        None => {
            return Err(Error::new(attr.span(), "expected ident, found nothing"));
        }
    };

    // Extract the parameters (which again, have to be a group delimited by
    // `()`)
    let params = match it.next() {
        Some(TokenTree::Group(ref g)) if g.delimiter() == Delimiter::Parenthesis => g.stream(),
        Some(other) => {
            return Err(Error::new(
                other.span(),
                format_args!(
                    "expected arguments for '{}' in parenthesis `()`, found `{}`",
                    name, other
                ),
            ));
        }
        None => {
            return Err(Error::new(
                body.span(),
                format_args!(
                    "expected arguments for '{}' in parenthesis `()`, found nothing",
                    name,
                ),
            ));
        }
    };

    // Finally match over the name of the attribute.
    let out = if name == "keep_default_for" {
        let proxy_types = parse_types(params.into());
        OurAttr::KeepDefaultFor(proxy_types)
    } else {
        return Err(Error::new(
            name.span(),
            format_args!(
                "invalid attribute '{}'; only `keep_default_for` is supported",
                name
            ),
        ));
    };

    Ok(out)
}

/// Attributes of the form `#[auto_impl(...)]` that can be attached to items of
/// the trait.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum OurAttr {
    KeepDefaultFor(Vec<ProxyType>),
}
