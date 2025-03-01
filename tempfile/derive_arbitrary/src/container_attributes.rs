use crate::ARBITRARY_ATTRIBUTE_NAME;
use syn::{
    parse::Error, punctuated::Punctuated, DeriveInput, Expr, ExprLit, Lit, Meta, MetaNameValue,
    Token, TypeParam,
};

pub struct ContainerAttributes {
    /// Specify type bounds to be applied to the derived `Arbitrary` implementation instead of the
    /// default inferred bounds.
    ///
    /// ```ignore
    /// #[arbitrary(bound = "T: Default, U: Debug")]
    /// ```
    ///
    /// Multiple attributes will be combined as long as they don't conflict, e.g.
    ///
    /// ```ignore
    /// #[arbitrary(bound = "T: Default")]
    /// #[arbitrary(bound = "U: Default")]
    /// ```
    pub bounds: Option<Vec<Punctuated<TypeParam, Token![,]>>>,
}

impl ContainerAttributes {
    pub fn from_derive_input(derive_input: &DeriveInput) -> Result<Self, Error> {
        let mut bounds = None;

        for attr in &derive_input.attrs {
            if !attr.path().is_ident(ARBITRARY_ATTRIBUTE_NAME) {
                continue;
            }

            let meta_list = match attr.meta {
                Meta::List(ref l) => l,
                _ => {
                    return Err(Error::new_spanned(
                        attr,
                        format!(
                            "invalid `{}` attribute. expected list",
                            ARBITRARY_ATTRIBUTE_NAME
                        ),
                    ))
                }
            };

            for nested_meta in
                meta_list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?
            {
                match nested_meta {
                    Meta::NameValue(MetaNameValue {
                        path,
                        value:
                            Expr::Lit(ExprLit {
                                lit: Lit::Str(bound_str_lit),
                                ..
                            }),
                        ..
                    }) if path.is_ident("bound") => {
                        bounds
                            .get_or_insert_with(Vec::new)
                            .push(bound_str_lit.parse_with(Punctuated::parse_terminated)?);
                    }
                    _ => {
                        return Err(Error::new_spanned(
                            attr,
                            format!(
                                "invalid `{}` attribute. expected `bound = \"..\"`",
                                ARBITRARY_ATTRIBUTE_NAME,
                            ),
                        ))
                    }
                }
            }
        }

        Ok(Self { bounds })
    }
}
