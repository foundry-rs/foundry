use crate::util::prelude::*;
use darling::FromMeta;
use syn::parse::Parse;
use syn::spanned::Spanned;
use syn::visit::Visit;

#[derive(Debug)]
pub(crate) struct OnConfig {
    pub(crate) type_pattern: syn::Type,
    pub(crate) into: darling::util::Flag,
    pub(crate) overwritable: darling::util::Flag,
    pub(crate) required: darling::util::Flag,
}

impl Parse for OnConfig {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let type_pattern = input.parse()?;

        let _ = input.parse::<syn::Token![,]>()?;
        let rest: TokenStream = input.parse()?;

        #[derive(FromMeta)]
        struct Parsed {
            into: darling::util::Flag,
            overwritable: darling::util::Flag,
            required: darling::util::Flag,
        }

        let parsed = Parsed::from_meta(&syn::parse_quote!(on(#rest)))?;

        if !cfg!(feature = "experimental-overwritable") && parsed.overwritable.is_present() {
            return Err(syn::Error::new(
                parsed.overwritable.span(),
                "🔬 `overwritable` attribute is experimental and requires \
                 \"experimental-overwritable\" cargo feature to be enabled; \
                 we would be glad to make this attribute stable if you find it useful; \
                 please leave a 👍 reaction under the issue https://github.com/elastio/bon/issues/149 \
                 to help us measure the demand for this feature; it would be \
                 double-awesome if you could also describe your use case in \
                 a comment under the issue for us to understand how it's used \
                 in practice",
            ));
        }

        {
            // Validate that at least some option was enabled.
            // This lives in a separate block to make sure that if a new
            // field is added to `Parsed` and unused here, then a compiler
            // warning is emitted.
            let Parsed {
                into,
                overwritable,
                required,
            } = &parsed;
            let flags = [
                ("into", into),
                ("overwritable", overwritable),
                ("required", required),
            ];

            if flags.iter().all(|(_, flag)| !flag.is_present()) {
                let flags = flags.iter().map(|(name, _)| format!("`{name}`")).join(", ");
                let err = format!(
                    "this #[builder(on(type_pattern, ...))] contains no options \
                    to override the default behavior for the selected setters \
                    like {flags}, so it does nothing"
                );

                return Err(syn::Error::new_spanned(&rest, err));
            }
        }

        struct FindAttr {
            attr: Option<Span>,
        }

        impl Visit<'_> for FindAttr {
            fn visit_attribute(&mut self, attr: &'_ syn::Attribute) {
                self.attr.get_or_insert(attr.span());
            }
        }

        let mut find_attr = FindAttr { attr: None };
        find_attr.visit_type(&type_pattern);

        if let Some(attr) = find_attr.attr {
            return Err(syn::Error::new(
                attr,
                "nested attributes are not allowed in the type pattern of \
                #[builder(on(type_pattern, ...))]",
            ));
        }

        // The validation is done in the process of matching the types. To make
        // sure that matching traverses the full pattern we match it with itself.
        let type_pattern_matches_itself = type_pattern.matches(&type_pattern)?;

        assert!(
            type_pattern_matches_itself,
            "BUG: the type pattern does not match itself: {type_pattern:#?}"
        );

        let Parsed {
            into,
            overwritable,
            required,
        } = parsed;

        Ok(Self {
            type_pattern,
            into,
            overwritable,
            required,
        })
    }
}

impl FromMeta for OnConfig {
    fn from_meta(meta: &syn::Meta) -> Result<Self> {
        let meta = match meta {
            syn::Meta::List(meta) => meta,
            _ => bail!(
                meta,
                "expected an attribute of form `on(type_pattern, ...)`"
            ),
        };

        let me = syn::parse2(meta.tokens.clone())?;

        Ok(me)
    }
}
