# `auto_impl` [![CI](https://github.com/auto-impl-rs/auto_impl/actions/workflows/ci.yml/badge.svg)](https://github.com/auto-impl-rs/auto_impl/actions/workflows/ci.yml) [![Crates.io](https://img.shields.io/crates/v/auto_impl.svg)](https://crates.io/crates/auto_impl) [![docs](https://docs.rs/auto_impl/badge.svg)](https://docs.rs/auto_impl)

A proc-macro attribute for automatically implementing a trait for references,
some common smart pointers and closures.

# Usage

This library requires Rust 1.56.0 or newer. This library doesn't leave any public API in your code.

Add `auto_impl` to your `Cargo.toml` and just use it in your crate:

```rust
// In Rust 2015 you still need `extern crate auto_impl;` at your crate root
use auto_impl::auto_impl;
```

Add an `auto_impl` attribute to traits you want to automatically implement for wrapper types. Here is a small example:

```rust
// This will generate two additional impl blocks: one for `&T` and one
// for `Box<T>` where `T: Foo`.
#[auto_impl(&, Box)]
trait Foo {
    fn foo(&self);
}

impl Foo for i32 {
    fn foo(&self) {}
}

fn requires_foo(_: impl Foo) {}


requires_foo(0i32);  // works: through the impl we defined above
requires_foo(&0i32); // works: through the generated impl
requires_foo(Box::new(0i32)); // works: through the generated impl
```

For more explanations, please see [**the documentation**](https://docs.rs/auto_impl) and for more examples, see 
[the examples folder](https://github.com/auto-impl-rs/auto_impl/tree/master/examples).

# Alternatives

This library implements a fraction of a very broad and complex usecase. It's mostly useful for applications that 
define traits for components, and want to be able to abstract over the storage for those traits. If it doesn't offer 
some functionality you need, check out the [`impl-tools`](https://github.com/kas-gui/impl-tools/) project.

---

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
