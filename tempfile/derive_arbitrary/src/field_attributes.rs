use crate::ARBITRARY_ATTRIBUTE_NAME;
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::quote;
use syn::{spanned::Spanned, *};

/// Determines how a value for a field should be constructed.
#[cfg_attr(test, derive(Debug))]
pub enum FieldConstructor {
    /// Assume that Arbitrary is defined for the type of this field and use it (default)
    Arbitrary,

    /// Places `Default::default()` as a field value.
    Default,

    /// Use custom function or closure to generate a value for a field.
    With(TokenStream),

    /// Set a field always to the given value.
    Value(TokenStream),
}

pub fn determine_field_constructor(field: &Field) -> Result<FieldConstructor> {
    let opt_attr = fetch_attr_from_field(field)?;
    let ctor = match opt_attr {
        Some(attr) => parse_attribute(attr)?,
        None => FieldConstructor::Arbitrary,
    };
    Ok(ctor)
}

fn fetch_attr_from_field(field: &Field) -> Result<Option<&Attribute>> {
    let found_attributes: Vec<_> = field
        .attrs
        .iter()
        .filter(|a| {
            let path = a.path();
            let name = quote!(#path).to_string();
            name == ARBITRARY_ATTRIBUTE_NAME
        })
        .collect();
    if found_attributes.len() > 1 {
        let name = field.ident.as_ref().unwrap();
        let msg = format!(
            "Multiple conflicting #[{ARBITRARY_ATTRIBUTE_NAME}] attributes found on field `{name}`"
        );
        return Err(syn::Error::new(field.span(), msg));
    }
    Ok(found_attributes.into_iter().next())
}

fn parse_attribute(attr: &Attribute) -> Result<FieldConstructor> {
    if let Meta::List(ref meta_list) = attr.meta {
        parse_attribute_internals(meta_list)
    } else {
        let msg = format!("#[{ARBITRARY_ATTRIBUTE_NAME}] must contain a group");
        Err(syn::Error::new(attr.span(), msg))
    }
}

fn parse_attribute_internals(meta_list: &MetaList) -> Result<FieldConstructor> {
    let mut tokens_iter = meta_list.tokens.clone().into_iter();
    let token = tokens_iter.next().ok_or_else(|| {
        let msg = format!("#[{ARBITRARY_ATTRIBUTE_NAME}] cannot be empty.");
        syn::Error::new(meta_list.span(), msg)
    })?;
    match token.to_string().as_ref() {
        "default" => Ok(FieldConstructor::Default),
        "with" => {
            let func_path = parse_assigned_value("with", tokens_iter, meta_list.span())?;
            Ok(FieldConstructor::With(func_path))
        }
        "value" => {
            let value = parse_assigned_value("value", tokens_iter, meta_list.span())?;
            Ok(FieldConstructor::Value(value))
        }
        _ => {
            let msg = format!("Unknown option for #[{ARBITRARY_ATTRIBUTE_NAME}]: `{token}`");
            Err(syn::Error::new(token.span(), msg))
        }
    }
}

// Input:
//     = 2 + 2
// Output:
//     2 + 2
fn parse_assigned_value(
    opt_name: &str,
    mut tokens_iter: impl Iterator<Item = TokenTree>,
    default_span: Span,
) -> Result<TokenStream> {
    let eq_sign = tokens_iter.next().ok_or_else(|| {
        let msg = format!(
            "Invalid syntax for #[{ARBITRARY_ATTRIBUTE_NAME}], `{opt_name}` is missing assignment."
        );
        syn::Error::new(default_span, msg)
    })?;

    if eq_sign.to_string() == "=" {
        Ok(tokens_iter.collect())
    } else {
        let msg = format!("Invalid syntax for #[{ARBITRARY_ATTRIBUTE_NAME}], expected `=` after `{opt_name}`, got: `{eq_sign}`");
        Err(syn::Error::new(eq_sign.span(), msg))
    }
}
