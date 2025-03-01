use heck::{ToKebabCase, ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{punctuated::Punctuated, Attribute, Error, LitBool, LitStr, Path, Result, Token};

const DUPLICATE_ERROR: &str = "duplicate attribute";
const UNKNOWN_ERROR: &str = "unknown `sol` attribute";

/// Wraps the argument in a doc attribute.
pub fn mk_doc(s: impl quote::ToTokens) -> TokenStream {
    quote!(#[doc = #s])
}

/// Returns `true` if the attribute is `#[doc = "..."]`.
pub fn is_doc(attr: &Attribute) -> bool {
    attr.path().is_ident("doc")
}

/// Returns `true` if the attribute is `#[derive(...)]`.
pub fn is_derive(attr: &Attribute) -> bool {
    attr.path().is_ident("derive")
}

/// Returns an iterator over all the `#[doc = "..."]` attributes.
pub fn docs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|a| is_doc(a))
}

/// Flattens all the `#[doc = "..."]` attributes into a single string.
pub fn docs_str(attrs: &[Attribute]) -> String {
    let mut doc = String::new();
    for attr in docs(attrs) {
        let syn::Meta::NameValue(syn::MetaNameValue {
            value: syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }),
            ..
        }) = &attr.meta
        else {
            continue;
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

/// Returns an iterator over all the `#[derive(...)]` attributes.
pub fn derives(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|a| is_derive(a))
}

/// Returns an iterator over all the rust `::` paths in the `#[derive(...)]`
/// attributes.
pub fn derives_mapped(attrs: &[Attribute]) -> impl Iterator<Item = Path> + '_ {
    derives(attrs).flat_map(parse_derives)
}

/// Parses the `#[derive(...)]` attributes into a list of paths.
pub fn parse_derives(attr: &Attribute) -> Punctuated<Path, Token![,]> {
    attr.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated).unwrap_or_default()
}

// When adding a new attribute:
// 1. add a field to this struct,
// 2. add a match arm in the `parse` function below,
// 3. add test cases in the `tests` module at the bottom of this file,
// 4. implement the attribute in your `SolInputExpander` implementation,
// 5. document the attribute in the [`sol!`] macro docs.

/// `#[sol(...)]` attributes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SolAttrs {
    /// `#[sol(rpc)]`
    pub rpc: Option<bool>,
    /// `#[sol(abi)]`
    pub abi: Option<bool>,
    /// `#[sol(all_derives)]`
    pub all_derives: Option<bool>,
    /// `#[sol(extra_methods)]`
    pub extra_methods: Option<bool>,
    /// `#[sol(docs)]`
    pub docs: Option<bool>,

    /// `#[sol(alloy_sol_types = alloy_core::sol_types)]`
    pub alloy_sol_types: Option<Path>,
    /// `#[sol(alloy_contract = alloy_contract)]`
    pub alloy_contract: Option<Path>,

    // TODO: Implement
    /// UNIMPLEMENTED: `#[sol(rename = "new_name")]`
    pub rename: Option<LitStr>,
    // TODO: Implement
    /// UNIMPLEMENTED: `#[sol(rename_all = "camelCase")]`
    pub rename_all: Option<CasingStyle>,

    /// `#[sol(bytecode = "0x1234")]`
    pub bytecode: Option<LitStr>,
    /// `#[sol(deployed_bytecode = "0x1234")]`
    pub deployed_bytecode: Option<LitStr>,

    /// UDVT only `#[sol(type_check = "my_function")]`
    pub type_check: Option<LitStr>,
}

impl SolAttrs {
    /// Parse the `#[sol(...)]` attributes from a list of attributes.
    pub fn parse(attrs: &[Attribute]) -> Result<(Self, Vec<Attribute>)> {
        let mut this = Self::default();
        let mut others = Vec::with_capacity(attrs.len());
        for attr in attrs {
            if !attr.path().is_ident("sol") {
                others.push(attr.clone());
                continue;
            }

            attr.meta.require_list()?.parse_nested_meta(|meta| {
                let path = meta.path.get_ident().ok_or_else(|| meta.error("expected ident"))?;
                let s = path.to_string();

                macro_rules! match_ {
                    ($($l:ident => $e:expr),* $(,)?) => {
                        match s.as_str() {
                            $(
                                stringify!($l) => if this.$l.is_some() {
                                    return Err(meta.error(DUPLICATE_ERROR))
                                } else {
                                    this.$l = Some($e);
                                },
                            )*
                            _ => return Err(meta.error(UNKNOWN_ERROR)),
                        }
                    };
                }

                // `path` => true, `path = <bool>` => <bool>
                let bool = || {
                    if let Ok(input) = meta.value() {
                        input.parse::<LitBool>().map(|lit| lit.value)
                    } else {
                        Ok(true)
                    }
                };

                // `path = <path>`
                let path = || meta.value()?.parse::<Path>();

                // `path = "<str>"`
                let lit = || meta.value()?.parse::<LitStr>();

                // `path = "0x<hex>"`
                let bytes = || {
                    let lit = lit()?;
                    if let Err(e) = hex::check(lit.value()) {
                        let msg = format!("invalid hex value: {e}");
                        return Err(Error::new(lit.span(), msg));
                    }
                    Ok(lit)
                };

                match_! {
                    rpc => bool()?,
                    abi => bool()?,
                    all_derives => bool()?,
                    extra_methods => bool()?,
                    docs => bool()?,

                    alloy_sol_types => path()?,
                    alloy_contract => path()?,

                    rename => lit()?,
                    rename_all => CasingStyle::from_lit(&lit()?)?,

                    bytecode => bytes()?,
                    deployed_bytecode => bytes()?,

                    type_check => lit()?,
                };
                Ok(())
            })?;
        }
        Ok((this, others))
    }
}

/// Trait for items that contain `#[sol(...)]` attributes among other
/// attributes. This is usually a shortcut  for [`SolAttrs::parse`].
pub trait ContainsSolAttrs {
    /// Get the list of attributes.
    fn attrs(&self) -> &[Attribute];

    /// Parse the `#[sol(...)]` attributes from the list of attributes.
    fn split_attrs(&self) -> syn::Result<(SolAttrs, Vec<Attribute>)> {
        SolAttrs::parse(self.attrs())
    }
}

impl ContainsSolAttrs for syn_solidity::File {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemContract {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemEnum {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemError {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemEvent {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemFunction {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemStruct {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

impl ContainsSolAttrs for syn_solidity::ItemUdt {
    fn attrs(&self) -> &[Attribute] {
        &self.attrs
    }
}

/// Defines the casing for the attributes long representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CasingStyle {
    /// Indicate word boundaries with uppercase letter, excluding the first
    /// word.
    Camel,
    /// Keep all letters lowercase and indicate word boundaries with hyphens.
    Kebab,
    /// Indicate word boundaries with uppercase letter, including the first
    /// word.
    Pascal,
    /// Keep all letters uppercase and indicate word boundaries with
    /// underscores.
    ScreamingSnake,
    /// Keep all letters lowercase and indicate word boundaries with
    /// underscores.
    Snake,
    /// Keep all letters lowercase and remove word boundaries.
    Lower,
    /// Keep all letters uppercase and remove word boundaries.
    Upper,
    /// Use the original attribute name defined in the code.
    Verbatim,
}

impl CasingStyle {
    fn from_lit(name: &LitStr) -> Result<Self> {
        let normalized = name.value().to_upper_camel_case().to_lowercase();
        let s = match normalized.as_ref() {
            "camel" | "camelcase" => Self::Camel,
            "kebab" | "kebabcase" => Self::Kebab,
            "pascal" | "pascalcase" => Self::Pascal,
            "screamingsnake" | "screamingsnakecase" => Self::ScreamingSnake,
            "snake" | "snakecase" => Self::Snake,
            "lower" | "lowercase" => Self::Lower,
            "upper" | "uppercase" => Self::Upper,
            "verbatim" | "verbatimcase" => Self::Verbatim,
            s => return Err(Error::new(name.span(), format!("unsupported casing: {s}"))),
        };
        Ok(s)
    }

    /// Apply the casing style to the given string.
    #[allow(dead_code)]
    pub fn apply(self, s: &str) -> String {
        match self {
            Self::Pascal => s.to_upper_camel_case(),
            Self::Kebab => s.to_kebab_case(),
            Self::Camel => s.to_lower_camel_case(),
            Self::ScreamingSnake => s.to_shouty_snake_case(),
            Self::Snake => s.to_snake_case(),
            Self::Lower => s.to_snake_case().replace('_', ""),
            Self::Upper => s.to_shouty_snake_case().replace('_', ""),
            Self::Verbatim => s.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    macro_rules! test_sol_attrs {
        ($($group:ident { $($t:tt)* })+) => {$(
            #[test]
            fn $group() {
                test_sol_attrs! { $($t)* }
            }
        )+};

        ($( $(#[$attr:meta])* => $expected:expr ),+ $(,)?) => {$(
            run_test(
                &[$(stringify!(#[$attr])),*],
                $expected
            );
        )+};
    }

    macro_rules! sol_attrs {
        ($($id:ident : $e:expr),* $(,)?) => {
            SolAttrs {
                $($id: Some($e),)*
                ..Default::default()
            }
        };
    }

    struct OuterAttribute(Vec<Attribute>);

    impl syn::parse::Parse for OuterAttribute {
        fn parse(input: syn::parse::ParseStream<'_>) -> Result<Self> {
            input.call(Attribute::parse_outer).map(Self)
        }
    }

    fn run_test(
        attrs_s: &'static [&'static str],
        expected: std::result::Result<SolAttrs, &'static str>,
    ) {
        let attrs: Vec<Attribute> =
            attrs_s.iter().flat_map(|s| syn::parse_str::<OuterAttribute>(s).unwrap().0).collect();
        match (SolAttrs::parse(&attrs), expected) {
            (Ok((actual, _)), Ok(expected)) => assert_eq!(actual, expected, "{attrs_s:?}"),
            (Err(actual), Err(expected)) => {
                let actual = actual.to_string();
                if !actual.contains(expected) {
                    assert_eq!(actual, expected, "{attrs_s:?}")
                }
            }
            (a, b) => panic!("assertion failed: `{a:?} != {b:?}`: {attrs_s:?}"),
        }
    }

    test_sol_attrs! {
        top_level {
            #[cfg] => Ok(SolAttrs::default()),
            #[cfg()] => Ok(SolAttrs::default()),
            #[cfg = ""] => Ok(SolAttrs::default()),
            #[derive()] #[sol()] => Ok(SolAttrs::default()),
            #[sol()] => Ok(SolAttrs::default()),
            #[sol()] #[sol()] => Ok(SolAttrs::default()),
            #[sol = ""] => Err("expected `(`"),
            #[sol] => Err("expected attribute arguments in parentheses: `sol(...)`"),

            #[sol(() = "")] => Err("unexpected token in nested attribute, expected ident"),
            #[sol(? = "")] => Err("unexpected token in nested attribute, expected ident"),
            #[sol(::a)] => Err("expected ident"),
            #[sol(::a = "")] => Err("expected ident"),
            #[sol(a::b = "")] => Err("expected ident"),
        }

        extra {
            #[sol(all_derives)] => Ok(sol_attrs! { all_derives: true }),
            #[sol(all_derives = true)] => Ok(sol_attrs! { all_derives: true }),
            #[sol(all_derives = false)] => Ok(sol_attrs! { all_derives: false }),
            #[sol(all_derives = "false")] => Err("expected boolean literal"),
            #[sol(all_derives)] #[sol(all_derives)] => Err(DUPLICATE_ERROR),

            #[sol(extra_methods)] => Ok(sol_attrs! { extra_methods: true }),
            #[sol(extra_methods = true)] => Ok(sol_attrs! { extra_methods: true }),
            #[sol(extra_methods = false)] => Ok(sol_attrs! { extra_methods: false }),

            #[sol(docs)] => Ok(sol_attrs! { docs: true }),
            #[sol(docs = true)] => Ok(sol_attrs! { docs: true }),
            #[sol(docs = false)] => Ok(sol_attrs! { docs: false }),

            #[sol(abi)] => Ok(sol_attrs! { abi: true }),
            #[sol(abi = true)] => Ok(sol_attrs! { abi: true }),
            #[sol(abi = false)] => Ok(sol_attrs! { abi: false }),

            #[sol(rpc)] => Ok(sol_attrs! { rpc: true }),
            #[sol(rpc = true)] => Ok(sol_attrs! { rpc: true }),
            #[sol(rpc = false)] => Ok(sol_attrs! { rpc: false }),

            #[sol(alloy_sol_types)] => Err("expected `=`"),
            #[sol(alloy_sol_types = alloy_core::sol_types)] => Ok(sol_attrs! { alloy_sol_types: parse_quote!(alloy_core::sol_types) }),
            #[sol(alloy_sol_types = ::alloy_core::sol_types)] => Ok(sol_attrs! { alloy_sol_types: parse_quote!(::alloy_core::sol_types) }),
            #[sol(alloy_sol_types = alloy::sol_types)] => Ok(sol_attrs! { alloy_sol_types: parse_quote!(alloy::sol_types) }),
            #[sol(alloy_sol_types = ::alloy::sol_types)] => Ok(sol_attrs! { alloy_sol_types: parse_quote!(::alloy::sol_types) }),

            #[sol(alloy_contract)] => Err("expected `=`"),
            #[sol(alloy_contract = alloy::contract)] => Ok(sol_attrs! { alloy_contract: parse_quote!(alloy::contract) }),
            #[sol(alloy_contract = ::alloy::contract)] => Ok(sol_attrs! { alloy_contract: parse_quote!(::alloy::contract) }),
        }

        rename {
            #[sol(rename = "foo")] => Ok(sol_attrs! { rename: parse_quote!("foo") }),

            #[sol(rename_all = "foo")] => Err("unsupported casing: foo"),
            #[sol(rename_all = "camelcase")] => Ok(sol_attrs! { rename_all: CasingStyle::Camel }),
            #[sol(rename_all = "camelCase")] #[sol(rename_all = "PascalCase")] => Err(DUPLICATE_ERROR),
        }

        bytecode {
            #[sol(deployed_bytecode = "0x1234")] => Ok(sol_attrs! { deployed_bytecode: parse_quote!("0x1234") }),
            #[sol(bytecode = "0x1234")] => Ok(sol_attrs! { bytecode: parse_quote!("0x1234") }),
            #[sol(bytecode = "1234")] => Ok(sol_attrs! { bytecode: parse_quote!("1234") }),
            #[sol(bytecode = "0x123xyz")] => Err("invalid hex value: "),
            #[sol(bytecode = "12 34")] => Err("invalid hex value: "),
            #[sol(bytecode = "xyz")] => Err("invalid hex value: "),
            #[sol(bytecode = "123")] => Err("invalid hex value: "),
        }

        type_check {
            #[sol(type_check = "my_function")] => Ok(sol_attrs! { type_check: parse_quote!("my_function") }),
            #[sol(type_check = "my_function1")] #[sol(type_check = "my_function2")] => Err(DUPLICATE_ERROR),
        }
    }
}
