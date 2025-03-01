//! The crunchy unroller - deterministically unroll constant loops. For number "crunching".
//!
//! The Rust optimizer will unroll constant loops that don't use the loop variable, like this:
//!
//! ```ignore
//! for _ in 0..100 {
//!     println!("Hello!");
//! }
//! ```
//!
//! However, using the loop variable will cause it to never unroll the loop. This is unfortunate because it means that you can't
//! constant-fold the loop variable, and if you end up stomping on the registers it will have to do a load for each iteration.
//! This crate ensures that your code is unrolled and const-folded. It only works on literals,
//! unfortunately, but there's a work-around:
//!
//! ```ignore
//! debug_assert_eq!(MY_CONSTANT, 100);
//! unroll! {
//!     for i in 0..100 {
//!         println!("Iteration {}", i);
//!     }
//! }
//! ```
//! This means that your tests will catch if you redefine the constant.
//!
//! To default maximum number of loops to unroll is `64`, but that can be easily increased using the cargo features:
//!
//! * `limit_128`
//! * `limit_256`
//! * `limit_512`
//! * `limit_1024`
//! * `limit_2048`

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(target_os = "windows")]
include!(concat!(env!("OUT_DIR"), "\\lib.rs"));

#[cfg(not(target_os = "windows"))]
include!(concat!(env!("OUT_DIR"), "/lib.rs"));
