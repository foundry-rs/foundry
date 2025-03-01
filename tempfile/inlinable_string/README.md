# `inlinable_string`

[![](http://meritbadge.herokuapp.com/inlinable_string)![](https://img.shields.io/crates/d/inlinable_string.png)](https://crates.io/crates/inlinable_string)

[![Build Status](https://travis-ci.org/fitzgen/inlinable_string.png?branch=master)](https://travis-ci.org/fitzgen/inlinable_string)

[![Coverage Status](https://coveralls.io/repos/fitzgen/inlinable_string/badge.svg?branch=master&service=github)](https://coveralls.io/github/fitzgen/inlinable_string?branch=master)

The `inlinable_string` crate provides the `InlinableString` type &mdash; an
owned, grow-able UTF-8 string that stores small strings inline and avoids
heap-allocation &mdash; and the `StringExt` trait which abstracts string
operations over both `std::string::String` and `InlinableString` (or even your
own custom string type).

`StringExt`'s API is mostly identical to `std::string::String`; unstable and
deprecated methods are not included. A `StringExt` implementation is provided
for both `std::string::String` and `InlinableString`. This enables
`InlinableString` to generally work as a drop-in replacement for
`std::string::String` and `&StringExt` to work with references to either type.

## But is it actually faster than using `std::string::String`?

Here are some current (micro)benchmark results. I encourage you to verify them
yourself by running `cargo bench --feature nightly` with a nightly Rust! I am
also very open to adding more realistic and representative benchmarks! Share
some ideas with me!

Constructing from a large `&str`:

```
test benches::bench_inlinable_string_from_large                ... bench:          32 ns/iter (+/- 6)
test benches::bench_std_string_from_large                      ... bench:          31 ns/iter (+/- 10)
```

Constructing from a small `&str`:

```
test benches::bench_inlinable_string_from_small                ... bench:           1 ns/iter (+/- 0)
test benches::bench_std_string_from_small                      ... bench:          26 ns/iter (+/- 14)
```

Pushing a large `&str` onto an empty string:

```
test benches::bench_inlinable_string_push_str_large_onto_empty ... bench:          37 ns/iter (+/- 12)
test benches::bench_std_string_push_str_large_onto_empty       ... bench:          30 ns/iter (+/- 9)
```

Pushing a small `&str` onto an empty string:

```
test benches::bench_inlinable_string_push_str_small_onto_empty ... bench:          11 ns/iter (+/- 4)
test benches::bench_std_string_push_str_small_onto_empty       ... bench:          23 ns/iter (+/- 10)
```

Pushing a large `&str` onto a large string:

```
test benches::bench_inlinable_string_push_str_large_onto_large ... bench:          80 ns/iter (+/- 24)
test benches::bench_std_string_push_str_large_onto_large       ... bench:          78 ns/iter (+/- 23)
```

Pushing a small `&str` onto a small string:

```
test benches::bench_inlinable_string_push_str_small_onto_small ... bench:          17 ns/iter (+/- 6)
test benches::bench_std_string_push_str_small_onto_small       ... bench:          60 ns/iter (+/- 15)
```

TLDR: If your string's size tends to stay within `INLINE_STRING_CAPACITY`, then
`InlinableString` is much faster. Crossing the threshold and forcing a promotion
from inline storage to heap allocation will slow it down more than
`std::string::String` and you can see the expected drop off in such cases, but
that is generally a one time cost. Once the strings are already larger than
`INLINE_STRING_CAPACITY`, then the performance difference is
negligible. However, take all this with a grain of salt! These are very micro
benchmarks and your (hashtag) Real World workload may differ greatly!

## Install

Either

    $ cargo add inlinable_string

or add this to your `Cargo.toml`:

    [dependencies]
    inlinable_string = "0.1.0"

## Documentation

[Documentation](http://fitzgen.github.io/inlinable_string/inlinable_string/index.html)
