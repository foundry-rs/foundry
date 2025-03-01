#![allow(
    clippy::large_enum_variant,
    clippy::too_many_arguments,
    clippy::derive_partial_eq_without_eq
)]

pub const CERTIFICATES: &[u8] = include_bytes!("../data/roots.pem");

#[allow(unused_macros)]
macro_rules! include_proto {
    ($package: tt) => {
        include!(concat!("../genproto", concat!("/", $package, ".rs")));
    };
}

include!("google_apis.rs");
