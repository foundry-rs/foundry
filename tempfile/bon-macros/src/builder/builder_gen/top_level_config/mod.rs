mod on;

pub(crate) use on::OnConfig;

use crate::parsing::{ItemSigConfig, ItemSigConfigParsing, SpannedKey};
use crate::util::prelude::*;
use darling::FromMeta;
use syn::punctuated::Punctuated;

fn parse_finish_fn(meta: &syn::Meta) -> Result<ItemSigConfig> {
    ItemSigConfigParsing {
        meta,
        reject_self_mentions: Some("builder struct's impl block"),
    }
    .parse()
}

fn parse_builder_type(meta: &syn::Meta) -> Result<ItemSigConfig> {
    ItemSigConfigParsing {
        meta,
        reject_self_mentions: Some("builder struct"),
    }
    .parse()
}

fn parse_state_mod(meta: &syn::Meta) -> Result<ItemSigConfig> {
    ItemSigConfigParsing {
        meta,
        reject_self_mentions: Some("builder's state module"),
    }
    .parse()
}

fn parse_start_fn(meta: &syn::Meta) -> Result<ItemSigConfig> {
    ItemSigConfigParsing {
        meta,
        reject_self_mentions: None,
    }
    .parse()
}

#[derive(Debug, FromMeta)]
pub(crate) struct TopLevelConfig {
    /// Overrides the path to the `bon` crate. This is usedfule when the macro is
    /// wrapped in another macro that also reexports `bon`.
    #[darling(rename = "crate", default, map = Some, with = crate::parsing::parse_bon_crate_path)]
    pub(crate) bon: Option<syn::Path>,

    #[darling(default, with = parse_start_fn)]
    pub(crate) start_fn: ItemSigConfig,

    #[darling(default, with = parse_finish_fn)]
    pub(crate) finish_fn: ItemSigConfig,

    #[darling(default, with = parse_builder_type)]
    pub(crate) builder_type: ItemSigConfig,

    #[darling(default, with = parse_state_mod)]
    pub(crate) state_mod: ItemSigConfig,

    #[darling(multiple, with = crate::parsing::parse_non_empty_paren_meta_list)]
    pub(crate) on: Vec<OnConfig>,

    /// Specifies the derives to apply to the builder.
    #[darling(default, with = crate::parsing::parse_non_empty_paren_meta_list)]
    pub(crate) derive: DerivesConfig,
}

impl TopLevelConfig {
    pub(crate) fn parse_for_fn(meta_list: &[darling::ast::NestedMeta]) -> Result<Self> {
        let me = Self::parse_for_any(meta_list)?;

        if me.start_fn.name.is_none() {
            let ItemSigConfig { name: _, vis, docs } = &me.start_fn;

            let unexpected_param = None
                .or_else(|| vis.as_ref().map(SpannedKey::key))
                .or_else(|| docs.as_ref().map(SpannedKey::key));

            if let Some(unexpected_param) = unexpected_param {
                bail!(
                    unexpected_param,
                    "#[builder(start_fn({unexpected_param}))] requires that you \
                    also specify #[builder(start_fn(name))] which makes the starting \
                    function not to replace the positional function under the #[builder] \
                    attribute; by default (without the explicit #[builder(start_fn(name))]) \
                    the name, visibility and documentation of the positional \
                    function are all copied to the starting function, and the positional \
                    function under the #[builder] attribute becomes private with \
                    #[doc(hidden)] and it's renamed (the name is not guaranteed \
                    to be stable) to make it inaccessible even within the current module",
                );
            }
        }

        Ok(me)
    }

    pub(crate) fn parse_for_struct(meta_list: &[darling::ast::NestedMeta]) -> Result<Self> {
        Self::parse_for_any(meta_list)
    }

    fn parse_for_any(meta_list: &[darling::ast::NestedMeta]) -> Result<Self> {
        // This is a temporary hack. We only allow `on(_, required)` as the
        // first `on(...)` clause. Instead we should implement an extended design:
        // https://github.com/elastio/bon/issues/152
        let mut on_configs = meta_list
            .iter()
            .enumerate()
            .filter_map(|(i, meta)| match meta {
                darling::ast::NestedMeta::Meta(syn::Meta::List(meta))
                    if meta.path.is_ident("on") =>
                {
                    Some((i, meta))
                }
                _ => None,
            })
            .peekable();

        while let Some((i, _)) = on_configs.next() {
            if let Some((j, next_on)) = on_configs.peek() {
                if *j != i + 1 {
                    bail!(
                        next_on,
                        "this `on(...)` clause is out of order; all `on(...)` \
                        clauses must be consecutive; there shouldn't be any \
                        other parameters between them",
                    )
                }
            }
        }

        let me = Self::from_list(meta_list)?;

        if let Some(on) = me.on.iter().skip(1).find(|on| on.required.is_present()) {
            bail!(
                &on.required.span(),
                "`required` can only be specified in the first `on(...)` clause; \
                this restriction may be lifted in the future",
            );
        }

        if let Some(first_on) = me.on.first().filter(|on| on.required.is_present()) {
            if !matches!(first_on.type_pattern, syn::Type::Infer(_)) {
                bail!(
                    &first_on.type_pattern,
                    "`required` can only be used with the wildcard type pattern \
                    i.e. `on(_, required)`; this restriction may be lifted in the future",
                );
            }
        }

        Ok(me)
    }
}

#[derive(Debug, Clone, Default, FromMeta)]
pub(crate) struct DerivesConfig {
    #[darling(rename = "Clone")]
    pub(crate) clone: Option<DeriveConfig>,

    #[darling(rename = "Debug")]
    pub(crate) debug: Option<DeriveConfig>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DeriveConfig {
    pub(crate) bounds: Option<Punctuated<syn::WherePredicate, syn::Token![,]>>,
}

impl FromMeta for DeriveConfig {
    fn from_meta(meta: &syn::Meta) -> Result<Self> {
        if let syn::Meta::Path(_) = meta {
            return Ok(Self { bounds: None });
        }

        meta.require_list()?.require_parens_delim()?;

        #[derive(FromMeta)]
        struct Parsed {
            #[darling(with = crate::parsing::parse_paren_meta_list_with_terminated)]
            bounds: Punctuated<syn::WherePredicate, syn::Token![,]>,
        }

        let Parsed { bounds } = Parsed::from_meta(meta)?;

        Ok(Self {
            bounds: Some(bounds),
        })
    }
}
