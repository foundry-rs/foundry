//! Private compile failure tests.
//!
//! # Repeated Options
//!
//! Options repeated in `#[divan::bench]` should cause a compile error, even if
//! they use raw identifiers. The initial implementation allowed raw identifiers
//! to slip through because `syn::Ident` does not consider them to be equal to
//! the normal form without the `r#` prefix.
//!
//! We don't include `r#crate` here because it's not a valid identifier.
//!
//! ```compile_fail
//! #[divan::bench(name = "x", r#name = "x")]
//! fn bench() {}
//! ```
//!
//! ```compile_fail
//! #[divan::bench(sample_count = 1, r#sample_count = 1)]
//! fn bench() {}
//! ```
//!
//! ```compile_fail
//! #[divan::bench(sample_size = 1, r#sample_size = 1)]
//! fn bench() {}
//! ```
//!
//! # Type Checking
//!
//! The following won't produce any benchmarks because `types = []`. However, we
//! still want to ensure that values in `consts = [...]` match the generic
//! const's type of `i32`.
//!
//! ```compile_fail
//! #[divan::bench(types = [], consts = ['a', 'b', 'c'])]
//! fn bench<T, const C: i32>() {}
//! ```
