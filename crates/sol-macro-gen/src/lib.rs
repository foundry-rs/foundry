//! This crate contains the logic for Rust bindings generating from Solidity contracts
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod sol_macro_gen;

pub use sol_macro_gen::*;
