use proc_macro2::{Delimiter, Group, Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{
    punctuated::Punctuated, Data, DataEnum, DataStruct, DeriveInput, Field, Fields, Path, Token,
    Type,
};

pub fn console_fmt(input: DeriveInput) -> TokenStream {
    let krate = crate::krate();
    let name = input.ident;
    let tokens = match input.data {
        Data::Struct(s) => derive_struct(s, &krate),
        Data::Enum(e) => derive_enum(e, &krate),
        Data::Union(_) => return quote!(compile_error!("Unions are unsupported");),
    };
    quote! {
        impl #krate::ConsoleFmt for #name {
            #tokens
        }
    }
}

fn derive_struct(s: DataStruct, krate: &Path) -> TokenStream {
    let imp = impl_struct(s, krate).unwrap_or_else(|| quote!(String::new()));
    quote! {
        fn fmt(&self, _spec: #krate::FormatSpec) -> String {
            #imp
        }
    }
}

fn impl_struct(s: DataStruct, krate: &Path) -> Option<TokenStream> {
    let fields: Punctuated<Field, Token![,]> = match s.fields {
        Fields::Named(fields) => fields.named.into_iter(),
        Fields::Unnamed(fields) => fields.unnamed.into_iter(),
        Fields::Unit => return None,
    }
    .collect();

    let n = fields.len();
    if n == 0 {
        return None
    }

    let first_ty = match &fields.first().unwrap().ty {
        Type::Path(path) => path.path.segments.last().unwrap().ident.to_string(),
        _ => String::new(),
    };

    let args: Punctuated<TokenStream, Token![,]> = fields
        .into_iter()
        .enumerate()
        .map(|(i, field)| {
            let ident = field.ident.unwrap_or_else(|| format_ident!("{i}"));
            quote!(&self.#ident)
        })
        .collect();

    let imp = if first_ty == "String" {
        // console_format(arg1, [...rest])
        let mut args = args.pairs();
        let first = args.next().unwrap();
        let first = first.value();
        let n = n - 1;
        quote! {
            let args: [&dyn #krate::ConsoleFmt; #n] = [#(#args)*];
            #krate::console_format((#first).as_str(), args)
        }
    } else {
        // console_format("", [...args])
        quote! {
            let args: [&dyn #krate::ConsoleFmt; #n] = [#args];
            #krate::console_format("", args)
        }
    };

    Some(imp)
}

/// Delegates to variants.
fn derive_enum(e: DataEnum, krate: &Path) -> TokenStream {
    let arms = e.variants.into_iter().map(|variant| {
        let name = variant.ident;
        let (fields, delimiter) = match variant.fields {
            Fields::Named(fields) => (fields.named.into_iter(), Delimiter::Brace),
            Fields::Unnamed(fields) => (fields.unnamed.into_iter(), Delimiter::Parenthesis),
            Fields::Unit => return quote!(),
        };

        let fields: Punctuated<Ident, Token![,]> = fields
            .enumerate()
            .map(|(i, field)| field.ident.unwrap_or_else(|| format_ident!("__var_{i}")))
            .collect();

        if fields.len() != 1 {
            unimplemented!("Enum variant with more than 1 field")
        }

        let field = fields.into_iter().next().unwrap();
        let fields = Group::new(delimiter, quote!(#field));
        quote! {
            Self::#name #fields => #krate::ConsoleFmt::fmt(#field, spec),
        }
    });

    quote! {
        fn fmt(&self, spec: #krate::FormatSpec) -> String {
            match self {
                #(#arms)*

                #[allow(unreachable_code)]
                _ => String::new(),
            }
        }
    }
}
