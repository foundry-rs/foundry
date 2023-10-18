use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Attribute, Data, DataStruct, DeriveInput, Error, Result};

// Skip warnings for these items.
const ALLOWED_ITEMS: &[&str] = &["CheatCodeError", "VmErrors"];

pub fn derive_cheatcode(input: &DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let name_s = name.to_string();
    match &input.data {
        Data::Struct(s) if name_s.ends_with("Call") => return derive_struct(name, s, &input.attrs),
        Data::Enum(e) if name_s.ends_with("Calls") => return derive_enum(name, e),
        _ => {}
    }

    if name_s.ends_with("Return") || ALLOWED_ITEMS.contains(&name_s.as_str()) {
        if let Data::Struct(data) = &input.data {
            check_named_fields(data, name);
        }
        return Ok(TokenStream::new())
    }

    if get_docstring(&input.attrs).trim().is_empty() {
        emit_warning!(input.ident, "missing documentation for an item");
    }
    match &input.data {
        Data::Struct(s) => {
            for field in s.fields.iter() {
                if get_docstring(&field.attrs).trim().is_empty() {
                    emit_warning!(field.ident, "missing documentation for a field");
                }
            }
        }
        Data::Enum(e) => {
            for variant in e.variants.iter() {
                if get_docstring(&variant.attrs).trim().is_empty() {
                    emit_warning!(variant.ident, "missing documentation for a variant");
                }
            }
        }
        _ => {}
    }
    Ok(TokenStream::new())
}

/// Implements `CheatcodeDef` for a struct.
fn derive_struct(name: &Ident, data: &DataStruct, attrs: &[Attribute]) -> Result<TokenStream> {
    let mut group = None::<Ident>;
    let mut status = None::<Ident>;
    let mut safety = None::<Ident>;
    for attr in attrs.iter().filter(|a| a.path().is_ident("cheatcode")) {
        attr.meta.require_list()?.parse_nested_meta(|meta| {
            let path = meta.path.get_ident().ok_or_else(|| meta.error("expected ident"))?;
            let path_s = path.to_string();
            match path_s.as_str() {
                "group" if group.is_none() => group = Some(meta.value()?.parse()?),
                "status" if status.is_none() => status = Some(meta.value()?.parse()?),
                "safety" if safety.is_none() => safety = Some(meta.value()?.parse()?),
                _ => return Err(meta.error("unexpected attribute")),
            };
            Ok(())
        })?;
    }
    let group = group.ok_or_else(|| {
        syn::Error::new(name.span(), "missing #[cheatcode(group = ...)] attribute")
    })?;
    let status = status.unwrap_or_else(|| Ident::new("Stable", Span::call_site()));
    let safety = if let Some(safety) = safety {
        quote!(Safety::#safety)
    } else {
        let panic = quote_spanned! {name.span()=>
            panic!("cannot determine safety from the group, add a `#[cheatcode(safety = ...)]` attribute")
        };
        quote! {
            match Group::#group.safety() {
                Some(s) => s,
                None => #panic,
            }
        }
    };

    check_named_fields(data, name);

    let id = name.to_string();
    let id = id.strip_suffix("Call").expect("function struct ends in Call");

    let doc = get_docstring(attrs);
    let (signature, selector, declaration, description) = func_docstring(&doc);

    let (visibility, mutability) = parse_function_attrs(declaration, name.span())?;
    let visibility = Ident::new(visibility, Span::call_site());
    let mutability = Ident::new(mutability, Span::call_site());

    if description.is_empty() {
        emit_warning!(name.span(), "missing documentation for a cheatcode")
    }
    let description = description.replace("\n ", "\n");

    Ok(quote! {
        impl CheatcodeDef for #name {
            const CHEATCODE: &'static Cheatcode<'static> = &Cheatcode {
                id: #id,
                declaration: #declaration,
                visibility: Visibility::#visibility,
                mutability: Mutability::#mutability,
                signature: #signature,
                selector: #selector,
                selector_bytes: <Self as ::alloy_sol_types::SolCall>::SELECTOR,
                description: #description,
                group: Group::#group,
                status: Status::#status,
                safety: #safety,
            };
        }
    })
}

/// Generates the `CHEATCODES` constant and implements `CheatcodeImpl` dispatch for an enum.
fn derive_enum(name: &Ident, input: &syn::DataEnum) -> Result<TokenStream> {
    if input.variants.iter().any(|v| v.fields.len() != 1) {
        return Err(syn::Error::new(name.span(), "expected all variants to have a single field"))
    }

    // keep original order for matching
    let variants_names = input.variants.iter().map(|v| &v.ident);

    let mut variants = input.variants.iter().collect::<Vec<_>>();
    variants.sort_by(|a, b| a.ident.cmp(&b.ident));
    let variant_tys = variants.iter().map(|v| {
        assert_eq!(v.fields.len(), 1);
        &v.fields.iter().next().unwrap().ty
    });
    Ok(quote! {
        /// All the cheatcodes in [this contract](self).
        pub const CHEATCODES: &'static [&'static Cheatcode<'static>] = &[#(<#variant_tys as CheatcodeDef>::CHEATCODE,)*];

        #[cfg(feature = "impls")]
        impl #name {
            pub(crate) fn apply<DB: crate::impls::DatabaseExt>(&self, ccx: &mut crate::impls::CheatsCtxt<DB>) -> crate::impls::Result {
                match self {
                    #(Self::#variants_names(c) => crate::impls::Cheatcode::apply_traced(c, ccx),)*
                }
            }
        }
    })
}

fn check_named_fields(data: &DataStruct, ident: &Ident) {
    for field in data.fields.iter() {
        if field.ident.is_none() {
            emit_warning!(ident, "all params must be named");
        }
    }
}

/// Flattens all the `#[doc = "..."]` attributes into a single string.
fn get_docstring(attrs: &[syn::Attribute]) -> String {
    let mut doc = String::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue
        }
        let syn::Meta::NameValue(syn::MetaNameValue {
            value: syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }),
            ..
        }) = &attr.meta
        else {
            continue
        };

        let value = s.value();
        if !value.is_empty() {
            if !doc.is_empty() {
                doc.push('\n');
            }
            doc.push_str(&value);
        }
    }
    doc
}

/// Returns `(signature, hex_selector, declaration, description)` from a given `sol!`-generated
/// docstring for a function.
///
/// # Examples
///
/// The following docstring (string literals are joined with newlines):
/// ```text
/// "Function with signature `foo(uint256)` and selector `0x1234abcd`."
/// "```solidity"
/// "function foo(uint256 x) external view returns (bool y);"
/// "```"
/// "Description of the function."
/// ```
///
/// Will return:
/// ```text
/// (
///     "foo(uint256)",
///     "0x1234abcd",
///     "function foo(uint256 x) external view returns (bool y);",
///     "Description of the function."
/// )
/// ```
fn func_docstring(doc: &str) -> (&str, &str, &str, &str) {
    let expected_start = "Function with signature `";
    let start = doc.find(expected_start).expect("no auto docstring");
    let (descr_before, auto) = doc.split_at(start);

    let mut lines = auto.lines();
    let mut next = || lines.next().expect("unexpected end of docstring");

    let sig_line = next();
    let example_start = next();
    assert_eq!(example_start, "```solidity");
    let declaration = next();
    let example_end = next();
    assert_eq!(example_end, "```");

    let n = expected_start.len();
    let mut sig_end = n;
    sig_end += sig_line[n..].find('`').unwrap();
    let sig = &sig_line[n..sig_end];
    assert!(!sig.starts_with('`') && !sig.ends_with('`'));

    let selector_end = sig_line.rfind('`').unwrap();
    let selector = sig_line[sig_end..selector_end].strip_prefix("` and selector `").unwrap();
    assert!(!selector.starts_with('`') && !selector.ends_with('`'));
    assert!(selector.starts_with("0x"));

    let description = match doc.find("```\n") {
        Some(i) => &doc[i + 4..],
        None => descr_before,
    };

    (sig, selector, declaration, description.trim())
}

/// Returns `(visibility, mutability)` from a given Solidity function declaration.
fn parse_function_attrs(f: &str, span: Span) -> Result<(&str, &str)> {
    let Some(ext_start) = f.find("external") else {
        return Err(Error::new(span, "functions must have `external` visibility"))
    };
    let visibility = "External";

    let f = &f[ext_start..];
    let mutability = if f.contains("view") {
        "View"
    } else if f.contains("pure") {
        "Pure"
    } else {
        "None"
    };
    Ok((visibility, mutability))
}
