use crate::parsing::{ItemSigConfig, ItemSigConfigParsing, SpannedKey};
use crate::util::prelude::*;
use darling::FromMeta;

const DOCS_CONTEXT: &str = "builder struct's impl block";

fn parse_setter_fn(meta: &syn::Meta) -> Result<SpannedKey<ItemSigConfig>> {
    let params = ItemSigConfigParsing {
        meta,
        reject_self_mentions: Some(DOCS_CONTEXT),
    }
    .parse()?;

    SpannedKey::new(meta.path(), params)
}

fn parse_docs(meta: &syn::Meta) -> Result<SpannedKey<Vec<syn::Attribute>>> {
    crate::parsing::parse_docs_without_self_mentions(DOCS_CONTEXT, meta)
}

#[derive(Debug, FromMeta)]
pub(crate) struct SettersConfig {
    pub(crate) name: Option<SpannedKey<syn::Ident>>,
    pub(crate) vis: Option<SpannedKey<syn::Visibility>>,

    #[darling(rename = "doc", default, with = parse_docs, map = Some)]
    pub(crate) docs: Option<SpannedKey<Vec<syn::Attribute>>>,

    #[darling(flatten)]
    pub(crate) fns: SettersFnsConfig,
}

#[derive(Debug, FromMeta)]
pub(crate) struct SettersFnsConfig {
    /// Config for the setter that accepts the value of type T for a member of
    /// type `Option<T>` or with `#[builder(default)]`.
    ///
    /// By default, it's named `{member}` without any prefix or suffix.
    #[darling(default, with = parse_setter_fn, map = Some)]
    pub(crate) some_fn: Option<SpannedKey<ItemSigConfig>>,

    /// The setter that accepts the value of type `Option<T>` for a member of
    /// type `Option<T>` or with `#[builder(default)]`.
    ///
    /// By default, it's named `maybe_{member}`.
    #[darling(default, with = parse_setter_fn, map = Some)]
    pub(crate) option_fn: Option<SpannedKey<ItemSigConfig>>,
}
