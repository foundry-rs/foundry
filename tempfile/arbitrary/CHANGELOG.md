## Unreleased

Released YYYY-MM-DD.

### Added

* TODO (or remove section if none)

### Changed

* TODO (or remove section if none)

### Deprecated

* TODO (or remove section if none)

### Removed

* TODO (or remove section if none)

### Fixed

* TODO (or remove section if none)

### Security

* TODO (or remove section if none)

--------------------------------------------------------------------------------

## 1.4.0

Released 2024-10-30.

### Added

* Added an `Arbitrary` implementation for `PhantomPinned`.
* Added the `Unstructured::choose_iter` helper method.
* Added `#[arbitrary(skip)]` for `enum` variants in the derive macro.
* Added the `Arbitrary::try_size_hint` trait method.

### Changed

* Implement `Arbitrary` for `PhantomData<A>` even when `A` does not implement
  `Arbitrary` and when `A` is `?Sized`.
* Make `usize`'s underlying encoding independent of machine word size so that
  corpora are more portable.

### Fixed

* Make `derive(Arbitrary)` work for local definitions of `struct Option`.

--------------------------------------------------------------------------------

## 1.3.2

Released 2023-10-30.

### Added

* Added `Arbitrary` implementations for `Arc<[T]>` and
  `Rc<[T]>`. [#160](https://github.com/rust-fuzz/arbitrary/pull/160)

--------------------------------------------------------------------------------

## 1.3.1

Released 2023-10-11.

### Fixed

* Fixed an issue with generating collections of collections in
  `arbitrary_take_rest` where `<Vec<Vec<u8>>>::arbitrary_take_rest` would never
  generate `vec![vec![]]` for example. See
  [#159](https://github.com/rust-fuzz/arbitrary/pull/159) for details.

--------------------------------------------------------------------------------

## 1.3.0

Released 2023-03-13.

### Added

* Added the ability to manually specify derived trait bounds for
  `Arbitrary`. See [#138](https://github.com/rust-fuzz/arbitrary/pull/138) for
  details.

### Fixed

* Fixed minimal versions correctness for `syn`.

--------------------------------------------------------------------------------

## 1.2.3

Released 2023-01-20.

### Fixed

* The `derive(Arbitrary)` will now annotate the generated `impl`s with a `#[automatically_derived]`
  attribute to indicate to e.g. clippy that lints should not fire for the code within the derived
  implementation.

## 1.2.2

Released 2023-01-03.

### Fixed

* Ensured that `arbitrary` and `derive_arbitrary` versions are synced up so that
  they don't, e.g., emit generated code that depends on newer versions of
  `arbitrary` than the one currently in
  use. [#134](https://github.com/rust-fuzz/arbitrary/issues/134)

## 1.2.1

### Fixed

* Fixed an issue where `std::thread_local!` macro invocations in derive code
  were not fully prefixed, causing confusing build errors in certain situations.

## 1.2.0

Released 2022-10-20.

### Added

* Support custom arbitrary implementation for fields on
  derive. [#129](https://github.com/rust-fuzz/arbitrary/pull/129)

--------------------------------------------------------------------------------

## 1.1.6

Released 2022-09-20.

### Fixed

* Fixed a potential panic due to an off-by-one error in the `Arbitrary`
  implementation for `std::ops::Bound<T>`.

--------------------------------------------------------------------------------

## 1.1.5

Released 2022-09-08.

### Added

* Implemented `Arbitrary` for `std::ops::Bound<T>`.

### Fixed

* Fixed a bug where `Unstructured::int_in_range` could return out-of-range
  integers when generating arbitrary signed integers.

--------------------------------------------------------------------------------

## 1.1.4

Released 2022-08-29.

### Added

* Implemented `Arbitrary` for `Rc<str>` and `Arc<str>`

### Changed

* Allow overriding the error type in `arbitrary::Result`
* The `Unstructured::arbitrary_loop` method will consume fewer bytes of input
  now.

### Fixed

* Fixed a bug where `Unstructured::int_in_range` could return out-of-range
  integers.

--------------------------------------------------------------------------------

## 1.1.3

Released 2022-06-23.

### Fixed

* Fixed some potential (but highly unlikely) name-clashes inside
  `derive(Arbitrary)`'s generated
  code. [#111](https://github.com/rust-fuzz/arbitrary/pull/111)
* Fixed an edge case where `derive(Arbitrary)` for recursive types that detected
  an overflow would not reset the overflow
  detection. [#111](https://github.com/rust-fuzz/arbitrary/pull/111)

--------------------------------------------------------------------------------

## 1.1.2

Released 2022-06-16.

### Fixed

* Fixed a warning inside `derive(Arbitrary)`-generated
  code. [#110](https://github.com/rust-fuzz/arbitrary/pull/110)

--------------------------------------------------------------------------------

## 1.1.1

Released 2022-06-14.

### Fixed

* Fixed a stack overflow when using `derive(Arbitrary)` with recursive types and
  empty inputs. [#109](https://github.com/rust-fuzz/arbitrary/pull/109)

--------------------------------------------------------------------------------

## 1.1.0

Released 2022-02-09.

### Added

* Added the `Unstructured::ratio` method to generate a boolean that is `true` at
  the given rate.

* Added the `Unstructured::arbitrary_loop` method to call a function an
  arbitrary number of times.

--------------------------------------------------------------------------------

## 1.0.3

Released 2021-11-20.

### Fixed

* Fixed documentation for `Unstructured::fill_bytes`. We forgot to update this
  way back in [#53](https://github.com/rust-fuzz/arbitrary/pull/53) when the
  behavior changed.

--------------------------------------------------------------------------------

## 1.0.2

Released 2021-08-25.

### Added

* `Arbitrary` impls for `HashMap`s and `HashSet`s with custom `Hasher`s
  [#87](https://github.com/rust-fuzz/arbitrary/pull/87)

--------------------------------------------------------------------------------

## 1.0.1

Released 2021-05-20.

### Added

* `Arbitrary` impls for `NonZeroX` types [#79](https://github.com/rust-fuzz/arbitrary/pull/79)
* `Arbitrary` impls for all arrays using const generics [#55](https://github.com/rust-fuzz/arbitrary/pull/55)
* `Arbitrary` impls for `Ipv4Addr` and `Ipv6Addr` [#84](https://github.com/rust-fuzz/arbitrary/pull/84)

### Fixed

* Use fewer bytes for `Unstructured::int_in_range()` [#80](https://github.com/rust-fuzz/arbitrary/pull/80)
* Use correct range for `char` generation [#83](https://github.com/rust-fuzz/arbitrary/pull/83)

--------------------------------------------------------------------------------

## 1.0.0

Released 2020-02-24.

See 1.0.0-rc1 and 1.0.0-rc2 for changes since 0.4.7, which was the last main
line release.

--------------------------------------------------------------------------------

## 1.0.0-rc2

Released 2021-02-09.

### Added

* The `Arbitrary` trait is now implemented for `&[u8]`. [#67](https://github.com/rust-fuzz/arbitrary/pull/67)

### Changed

* Rename `Unstructured#get_bytes` to `Unstructured#bytes`. [#70](https://github.com/rust-fuzz/arbitrary/pull/70)
* Passing an empty slice of choices to `Unstructured#choose` returns an error. Previously it would panic. [71](https://github.com/rust-fuzz/arbitrary/pull/71)

--------------------------------------------------------------------------------

## 1.0.0-rc1

Released 2020-11-25.

### Added

* The `Arbitrary` trait is now implemented for `&str`. [#63](https://github.com/rust-fuzz/arbitrary/pull/63)

### Changed

* The `Arbitrary` trait now has a lifetime parameter, allowing `Arbitrary` implementations that borrow from the raw input (e.g. the new `&str` implementaton). The `derive(Arbitrary)` macro also supports deriving `Arbitrary` on types with lifetimes now. [#63](https://github.com/rust-fuzz/arbitrary/pull/63)

### Removed

* The `shrink` method on the `Arbitrary` trait has been removed.

  We have found that, in practice, using [internal reduction](https://drmaciver.github.io/papers/reduction-via-generation-preview.pdf) via approaches like `cargo fuzz tmin`, where the raw input bytes are reduced rather than the `T: Arbitrary` type constructed from those raw bytes, has the best efficiency-to-maintenance ratio. To the best of our knowledge, no one is relying on or using the `Arbitrary::shrink` method. If you *are* using and relying on the `Arbitrary::shrink` method, please reach out by [dropping a comment here](https://github.com/rust-fuzz/arbitrary/issues/62) and explaining how you're using it and what your use case is. We'll figure out what the best solution is, including potentially adding shrinking functionality back to the `arbitrary` crate.

--------------------------------------------------------------------------------

## 0.4.7

Released 2020-10-14.

### Added

* Added an optimization to avoid unnecessarily consuming bytes from the
  underlying data when there is only one possible choice in
  `Unstructured::{int_in_range, choose, etc..}`.

* Added license files to the derive crate.

### Changed

* The `Arbitrary` implementation for `std::time::Duration` should now be faster
  and produce durations with a more-uniform distribution of nanoseconds.

--------------------------------------------------------------------------------

## 0.4.6

Released 2020-08-22.

### Added

* Added the `Unstructured::peek_bytes` method.

### Changed

* Test case reduction via `cargo fuzz tmin` should be much more effective at
  reducing the sizes of collections now. (See
  [#53](https://github.com/rust-fuzz/arbitrary/pull/53) and the commit messages
  for details.)

* Fuzzing with mutation-based fuzzers (like libFuzzer) should be more efficient
  now. (See [#53](https://github.com/rust-fuzz/arbitrary/pull/53) and the commit
  messages for details)

--------------------------------------------------------------------------------

## 0.4.5

Released 2020-06-18.

### Added

* Implement `Arbitrary` for zero length arrays.
* Implement `Arbitrary` for `Range` and `RangeInclusive`.

--------------------------------------------------------------------------------

## 0.4.4

Released 2020-04-29.

### Fixed

* Fixed the custom derive for enums when used via its full path (like
  `#[derive(arbitrary::Arbitrary)]` rather than like `#[derive(Arbitrary)]`).


## 0.4.3

Released 2020-04-28.

### Fixed

* Fixed the custom derive when used via its full path (like
  `#[derive(arbitrary::Arbitrary)]` rather than like `#[derive(Arbitrary)]`).

--------------------------------------------------------------------------------

## 0.4.2

Released 2020-04-17.

### Changed

* We forgot to release a new version of the `derive_arbitrary` crate last
  release. This release fixes that and so the `synstructure` dependency is
  finally actually removed in the cargo releases.

--------------------------------------------------------------------------------

## 0.4.1

Released 2020-03-18.

### Removed

* Removed an internal dependency on the `synstructure` crate when the `derive`
  feature is enabled. This should not have any visible downstream effects other
  than faster build times!

--------------------------------------------------------------------------------

## 0.4.0

Released 2020-01-22.

This is technically a breaking change, but we expect that nearly everyone should
be able to upgrade without any compilation errors. The only exception is if you
were implementing the `Arbitrary::size_hint` method by hand. If so, see the
"changed" section below and the [API docs for
`Arbitrary::shrink`](https://docs.rs/arbitrary/0.4.0/arbitrary/trait.Arbitrary.html#method.size_hint)
for details.

### Added

* Added [the `arbitary::size_hint::recursion_guard` helper
  function][recursion_guard] for guarding against infinite recursion in
  `size_hint` implementations for recursive types.

### Changed

* The `Arbitrary::size_hint` signature now takes a `depth: usize`
  parameter. This should be passed along unmodified to any nested calls of other
  `size_hint` methods. If you're implementing `size_hint` for a recursive type
  (like a linked list or tree) or a generic type with type parameters, you
  should use [the new `arbitrary::size_hint::recursion_guard` helper
  function][recursion_guard].

### Fixed

* Fixed infinite recursion in generated `size_hint` implementations
  from `#[derive(Arbitrary)]` for recursive types.

[recursion_guard]: https://docs.rs/arbitrary/0.4.0/arbitrary/size_hint/fn.recursion_guard.html

--------------------------------------------------------------------------------

## 0.3.2

Released 2020-01-16.

### Changed

* Updated the custom derive's dependencies.

--------------------------------------------------------------------------------

## 0.3.2

Released 2020-01-15.

### Fixed

* Fixed an over-eager assertion condition in `Unstructured::int_in_range` that
  would incorrectly trigger when given valid ranges of length one.

--------------------------------------------------------------------------------

## 0.3.1

Released 2020-01-14.

### Fixed

* Fixed some links and version numbers in README.

--------------------------------------------------------------------------------

## 0.3.0

Released 2020-01-14.

### Added

* Added the `"derive"` cargo feature, to enable `#[derive(Arbitrary)]` for
  custom types. Enabling this feature re-exports functionality from the
  `derive_arbitrary` crate.
* The custom derive for `Arbitrary` implements the shrink method for you now.
* All implementations of `Arbitrary` for `std` types implement shrinking now.
* Added the `Arbitrary::arbitrary_take_rest` method allows an `Arbitrary`
  implementation to consume all of the rest of the remaining raw input. It has a
  default implementation that forwards to `Arbitrary::arbitrary` and the custom
  derive creates a smart implementation for your custom types.
* Added the `Arbitrary::size_hint` method for hinting how many raw bytes an
  implementation needs to construct itself. This has a default implementation,
  but the custom derive creates a smart implementation for your custom types.
* Added the `Unstructured::choose` method to choose one thing among a set of
  choices.
* Added the `Unstructured::arbitrary_len` method to get an arbitrary length for
  a collection of some arbitrary type.
* Added the `Unstructured::arbitrary_iter` method to create an iterator of
  arbitrary instance of some type.

### Changed

* The `Arbitrary` trait was simplified a bit.
* `Unstructured` is a concrete type now, not a trait.
* Switched to Rust 2018 edition.

### Removed

* `RingBuffer` and `FiniteBuffer` are removed. Use `Unstructured` instead.

### Fixed

* Better `Arbitrary` implementation for `char`.
* Better `Arbitrary` implementation for `String`.

--------------------------------------------------------------------------------

## 0.2.0

--------------------------------------------------------------------------------

## 0.1.0
