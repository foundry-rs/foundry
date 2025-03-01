#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

extern crate syn_solidity as ast;

/// Tools for working with `#[...]` attributes.
mod attr;
pub use attr::{
    derives_mapped, docs_str, mk_doc, parse_derives, CasingStyle, ContainsSolAttrs, SolAttrs,
};

mod input;
pub use input::{SolInput, SolInputKind};

mod expander;
pub use expander::SolInputExpander;

#[cfg(feature = "json")]
mod json;
#[cfg(feature = "json")]
pub use json::tokens_for_sol;
