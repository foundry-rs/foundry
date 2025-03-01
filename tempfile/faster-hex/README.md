# faster-hex

[![License]](#license)
[![crate-badge]](https://crates.io/crates/faster-hex)

[crate-badge]: https://img.shields.io/crates/v/faster-hex.svg
[license]: https://img.shields.io/badge/License-MIT-green.svg

This program implements hex encoding a slice into a predetermined
destination using various different instruction sets.

## Benchmark

### Running
Runs benchmark
```
cargo bench
```

### Results
Machine: MacBook Pro (Early 2015) (2.7 GHz Intel Core i5)

Rust: rustc 1.31.0 (abe02cefd 2018-12-04)

Compare with [hex](https://crates.io/crates/hex):

* Encoding ~10x over
* Decoding ~10x over

Compare with [rustc-hex](https://crates.io/crates/rustc-hex):

* Encoding ~2.5x over
* Decoding ~7x over

## Examples
Encode to hex

```rust
use faster_hex::hex_string;

let result = hex_string(b"Hello world!");
assert_eq!(result, "48656c6c6f20776f726c6421");
```
Encode to upper case hex
```rust
use faster_hex::hex_string_upper;

let result = hex_string_upper(b"Hello world!");
assert_eq!(result, "48656C6C6F20776F726C6421");
```

Decode
```rust
use faster_hex::hex_decode;

let src = b"48656c6c6f20776f726c6421";
let mut dst = vec![0; src.len() / 2];
hex_decode(src, &mut dst).unwrap();
assert_eq!(dst, b"Hello world!");
```
Decode with case check
```rust
use faster_hex::{hex_decode_with_case, CheckCase};

let src = b"48656c6c6f20776f726c6421";
let mut dst = vec![0; src.len() / 2];

assert!(hex_decode_with_case(src, &mut dst, CheckCase::Lower).is_ok());
assert_eq!(dst, b"Hello world!");

assert!(hex_decode_with_case(src, &mut dst, CheckCase::None).is_ok());
assert_eq!(dst, b"Hello world!");

assert!(hex_decode_with_case(src, &mut dst, CheckCase::Upper).is_err());
```

Serde feature
```rust

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Simple {
  #[serde(with = "faster_hex")]
  foo: Vec<u8>,
  #[serde(with = "faster_hex::nopfx_lowercase")]
  bar: Vec<u8>,
}
```


## Notice

Major version zero (0.y.z) is for initial development. Anything MAY change at any time. The public API SHOULD NOT be considered stable.

MINOR version when make incompatible API changes before 1.0.0.


## License

This project is licensed under the [MIT license](LICENSE).

### Third party software

This product includes copies and modifications of software developed by third parties:

* [src/encode.rs](src/encode.rs) is based on
  [stdsimd](https://github.com/rust-lang-nursery/stdsimd), licensed
  under the MIT license or the Apache License (Version 2.0).
* [src/decode.rs](src/decode.rs) avx2 decode is modified from [fast-hex](https://github.com/zbjornson/fast-hex)

See the source code files for more details.

Copies of third party licenses can be found in [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY).
