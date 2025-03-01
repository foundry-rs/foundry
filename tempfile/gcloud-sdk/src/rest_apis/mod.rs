#![allow(
    unused_imports,
    clippy::redundant_clone,
    clippy::large_enum_variant,
    clippy::too_many_arguments,
    clippy::derive_partial_eq_without_eq,
    clippy::manual_non_exhaustive
)] // for generated files

mod rest_api_client;
pub use rest_api_client::*;

pub mod google_rest_apis;
