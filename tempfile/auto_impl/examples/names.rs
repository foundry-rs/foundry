//! Example to demonstrate how `auto_impl` chooses a name for the type and
//! lifetime parameter.
//!
//! For documentation and compiler errors it would be nice to have very simple
//! names for type and lifetime parameters:
//!
//! ```rust
//! // not nice
//! impl<'auto_impl_lifetime, AutoImplT> Foo for &'auto_impl_lifetime AutoImplT { ...}
//!
//! // better
//! impl<'a, T> Foo for &'a T { ... }
//! ```
//!
//! `auto_impl` tries the full alphabet, picking a name that is not yet taken.
//! "Not taken" means that the name is not used anywhere in the `impl` block.
//! Right now, we are a bit careful and mark all names as "taken" that are used
//! in the trait def -- apart from names only appearing in the body of provided
//! methods.
//!
//! In the trait below, a bunch of names are already "taken":
//! - type names: T--Z and A--G (H is not taken, because it only appear in the
//!   default method body)
//! - lifetime names: 'a--'c
//!
//! Thus, the names `H` and `'d` are used. Run `cargo expand --example names`
//! to see the output.


// This code is really ugly on purpose...
#![allow(non_snake_case, dead_code, unused_variables, clippy::extra_unused_lifetimes, clippy::let_unit_value, clippy::redundant_allocation)]
#![cfg_attr(rustfmt, rustfmt::skip)]

use auto_impl::auto_impl;



struct X {}
trait Z {}

struct C {}
struct E<T>(Vec<T>);
struct F {}

struct G<T>(Vec<T>);
struct H {}

#[auto_impl(&)]
trait U<'a, V> {
    const W: Option<Box<&'static X>>;

    type Y: Z;

    fn A<'b, 'c>(&self, B: C, D: E<&[F; 1]>) -> G<fn((H,))> {
        let H = ();
        unimplemented!()
    }
}

fn main() {}
