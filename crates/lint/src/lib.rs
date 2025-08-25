#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![allow(elided_lifetimes_in_paths)]

// Feature.
use solar_interface as _;

pub mod inline_config;
pub mod linter;
pub mod sol;
