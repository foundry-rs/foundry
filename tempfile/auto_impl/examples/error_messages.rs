//! This file showcases a few error messages emitted by `auto_impl`. You have
//! to add specific lines to see the error. Then simply compile with:
//!
//! ```
//! cargo build --example error_messages
//! ```
#![allow(unused_imports, dead_code)]

use auto_impl::auto_impl;

// Shows the error message for the case that `#[auto_impl]` was used with
// incorrect proxy types. Only proxy types like `&` and `Box` are allowed. Add
// this next line to see the error!
//#[auto_impl(Boxxi)]
trait Foo {
    fn foo(&self) -> u32;
}

// Shows the error message for the case the `#[auto_impl]` wasn't applied to a
// valid trait (in this case a struct). Add this next line to see the error!
//#[auto_impl(&, Box)]
struct Bar {
    x: u32,
}

fn main() {}
