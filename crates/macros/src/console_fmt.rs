use proc_macro2::{Delimiter, Group, Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Fields, Member, Token, Type, punctuated::Punctuated,
};

pub fn console_fmt(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;
    let tokens = match &input.data {
        Data::Struct(s) => derive_struct(s, name),
        Data::Enum(e) => derive_enum(e),
        Data::Union(_) => return quote!(compile_error!("Unions are unsupported");),
    };
    quote! {
        impl ConsoleFmt for #name {
            #tokens
        }
    }
}

fn derive_struct(s: &DataStruct, name: &Ident) -> TokenStream {
    let imp = impl_struct(s, name).unwrap_or_else(|| quote!(String::new()));
    quote! {
        fn fmt(&self, _spec: FormatSpec) -> String {
            #imp
        }
    }
}

fn impl_struct(s: &DataStruct, name: &Ident) -> Option<TokenStream> {
    if s.fields.is_empty() {
        return None;
    }

    if matches!(s.fields, Fields::Unit) {
        return None;
    }

    let members = s.fields.members().collect::<Vec<_>>();
    let fields = s.fields.iter().collect::<Vec<_>>();

    // Detect table call structs: name must start with "table" (from the ABI function name) and
    // all fields must be Vec<T> types (Solidity arrays). Both conditions together prevent
    // accidental table rendering for unrelated structs that happen to have Vec fields.
    let is_table = name.to_string().starts_with("table")
        && !fields.is_empty()
        && fields.iter().all(|f| match &f.ty {
            Type::Path(path) => path.path.segments.last().is_some_and(|seg| seg.ident == "Vec"),
            _ => false,
        });
    if is_table {
        let member_ref = |m: &Member| match m {
            Member::Named(ident) => quote!(&self.#ident),
            Member::Unnamed(idx) => quote!(&self.#idx),
        };
        let imp = if members.len() == 1 {
            let vals = member_ref(&members[0]);
            quote! {
                let values: ::std::vec::Vec<&dyn ConsoleFmt> =
                    (#vals).iter().map(|v| v as &dyn ConsoleFmt).collect();
                console_table_format(None, &values)
            }
        } else {
            let keys = member_ref(&members[0]);
            let vals = member_ref(&members[1]);
            quote! {
                let keys: ::std::vec::Vec<&dyn ConsoleFmt> =
                    (#keys).iter().map(|v| v as &dyn ConsoleFmt).collect();
                let values: ::std::vec::Vec<&dyn ConsoleFmt> =
                    (#vals).iter().map(|v| v as &dyn ConsoleFmt).collect();
                console_table_format(Some(&keys), &values)
            }
        };
        return Some(imp);
    }

    let first_ty = match &fields.first().unwrap().ty {
        Type::Path(path) => path.path.segments.last().unwrap().ident.to_string(),
        _ => String::new(),
    };

    let args: Punctuated<TokenStream, Token![,]> = members
        .into_iter()
        .map(|member| match member {
            Member::Named(ident) => quote!(&self.#ident),
            Member::Unnamed(idx) => quote!(&self.#idx),
        })
        .collect();

    let imp = if first_ty == "String" {
        // console_format(arg1, [...rest])
        let mut args = args.pairs();
        let first = args.next().unwrap();
        let first = first.value();
        quote! {
            console_format((#first).as_str(), &[#(#args)*])
        }
    } else {
        // console_format("", [...args])
        quote! {
            console_format("", &[#args])
        }
    };

    Some(imp)
}

/// Delegates to variants.
fn derive_enum(e: &DataEnum) -> TokenStream {
    let arms = e.variants.iter().map(|variant| {
        let name = &variant.ident;
        let (fields, delimiter) = match &variant.fields {
            Fields::Named(fields) => (fields.named.iter(), Delimiter::Brace),
            Fields::Unnamed(fields) => (fields.unnamed.iter(), Delimiter::Parenthesis),
            Fields::Unit => return quote!(),
        };

        let fields: Punctuated<Ident, Token![,]> = fields
            .enumerate()
            .map(|(i, field)| field.ident.clone().unwrap_or_else(|| format_ident!("__var_{i}")))
            .collect();

        if fields.len() != 1 {
            unimplemented!("Enum variant with more than 1 field")
        }

        let field = fields.into_iter().next().unwrap();
        let fields = Group::new(delimiter, quote!(#field));
        quote! {
            Self::#name #fields => ConsoleFmt::fmt(#field, spec),
        }
    });

    quote! {
        fn fmt(&self, spec: FormatSpec) -> String {
            match self {
                #(#arms)*

                #[expect(unreachable_code)]
                _ => String::new(),
            }
        }
    }
}
