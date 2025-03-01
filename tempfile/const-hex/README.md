# const-hex

[![github](https://img.shields.io/badge/github-danipopes/const--hex-8da0cb?style=for-the-badge&labelColor=555555&logo=github)](https://github.com/danipopes/const-hex)
[![crates.io](https://img.shields.io/crates/v/const-hex.svg?style=for-the-badge&color=fc8d62&logo=rust)](https://crates.io/crates/const-hex)
[![docs.rs](https://img.shields.io/badge/docs.rs-const--hex-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs)](https://docs.rs/const-hex)
[![build status](https://img.shields.io/github/actions/workflow/status/danipopes/const-hex/ci.yml?branch=master&style=for-the-badge)](https://github.com/danipopes/const-hex/actions?query=branch%3Amaster)

This crate provides a fast conversion of byte arrays to hexadecimal strings,
both at compile time, and at run time.

It aims to be a drop-in replacement for the [`hex`] crate, as well as extending
the API with [const-eval], a [const-generics formatting buffer][buffer], similar
to [`itoa`]'s, and more.

_Version requirement: rustc 1.64+_

[const-eval]: https://docs.rs/const-hex/latest/const_hex/fn.const_encode.html
[buffer]: https://docs.rs/const-hex/latest/const_hex/struct.Buffer.html
[`itoa`]: https://docs.rs/itoa/latest/itoa/struct.Buffer.html

## Performance

This crate offers performance comparable to [`faster-hex`] on `x86`/`x86-64`
architectures but outperforms it on other platforms, as [`faster-hex`] is
only optimized for `x86`/`x86-64`.

This crate is 10 to 50 times faster than [`hex`] in encoding and decoding, and
100+ times faster than `libstd` in formatting.

The following benchmarks were ran on an AMD Ryzen 9 7950X, compiled with
`1.83.0-nightly (9ff5fc4ff 2024-10-03)` on `x86_64-unknown-linux-gnu`.

You can run these benchmarks with `cargo bench --features std` on a nightly
compiler.

```log
test check::const_hex::bench1_32b             ... bench:           3.07 ns/iter (+/- 0.62)
test check::const_hex::bench2_256b            ... bench:          15.65 ns/iter (+/- 0.46)
test check::const_hex::bench3_2k              ... bench:         120.12 ns/iter (+/- 4.08)
test check::const_hex::bench4_16k             ... bench:         935.28 ns/iter (+/- 19.27)
test check::const_hex::bench5_128k            ... bench:       7,442.57 ns/iter (+/- 4,921.92)
test check::const_hex::bench6_1m              ... bench:      59,889.93 ns/iter (+/- 1,471.89)
test check::faster_hex::bench1_32b            ... bench:           2.79 ns/iter (+/- 0.08)
test check::faster_hex::bench2_256b           ... bench:          14.97 ns/iter (+/- 0.40)
test check::faster_hex::bench3_2k             ... bench:         122.31 ns/iter (+/- 5.01)
test check::faster_hex::bench4_16k            ... bench:         984.16 ns/iter (+/- 11.57)
test check::faster_hex::bench5_128k           ... bench:       7,855.54 ns/iter (+/- 61.75)
test check::faster_hex::bench6_1m             ... bench:      63,171.43 ns/iter (+/- 3,022.35)
test check::naive::bench1_32b                 ... bench:          17.05 ns/iter (+/- 2.25)
test check::naive::bench2_256b                ... bench:         188.65 ns/iter (+/- 6.14)
test check::naive::bench3_2k                  ... bench:       2,050.15 ns/iter (+/- 313.23)
test check::naive::bench4_16k                 ... bench:      16,852.37 ns/iter (+/- 983.27)
test check::naive::bench5_128k                ... bench:     521,793.20 ns/iter (+/- 18,279.50)
test check::naive::bench6_1m                  ... bench:   4,007,801.65 ns/iter (+/- 80,974.34)

test decode::const_hex::bench1_32b            ... bench:          17.57 ns/iter (+/- 0.53)
test decode::const_hex::bench2_256b           ... bench:          39.20 ns/iter (+/- 3.36)
test decode::const_hex::bench3_2k             ... bench:         236.98 ns/iter (+/- 3.22)
test decode::const_hex::bench4_16k            ... bench:       1,708.26 ns/iter (+/- 38.29)
test decode::const_hex::bench5_128k           ... bench:      13,258.62 ns/iter (+/- 665.24)
test decode::const_hex::bench6_1m             ... bench:     108,937.41 ns/iter (+/- 6,453.24)
test decode::faster_hex::bench1_32b           ... bench:          17.25 ns/iter (+/- 0.14)
test decode::faster_hex::bench2_256b          ... bench:          55.01 ns/iter (+/- 1.33)
test decode::faster_hex::bench3_2k            ... bench:         253.37 ns/iter (+/- 6.11)
test decode::faster_hex::bench4_16k           ... bench:       1,864.45 ns/iter (+/- 25.81)
test decode::faster_hex::bench5_128k          ... bench:      14,664.17 ns/iter (+/- 268.45)
test decode::faster_hex::bench6_1m            ... bench:     118,576.16 ns/iter (+/- 2,564.34)
test decode::hex::bench1_32b                  ... bench:         107.13 ns/iter (+/- 7.28)
test decode::hex::bench2_256b                 ... bench:         666.06 ns/iter (+/- 16.19)
test decode::hex::bench3_2k                   ... bench:       5,044.12 ns/iter (+/- 147.09)
test decode::hex::bench4_16k                  ... bench:      40,003.46 ns/iter (+/- 999.15)
test decode::hex::bench5_128k                 ... bench:     797,007.70 ns/iter (+/- 10,044.26)
test decode::hex::bench6_1m                   ... bench:   6,409,293.90 ns/iter (+/- 102,747.28)
test decode::rustc_hex::bench1_32b            ... bench:         139.10 ns/iter (+/- 5.50)
test decode::rustc_hex::bench2_256b           ... bench:         852.05 ns/iter (+/- 24.91)
test decode::rustc_hex::bench3_2k             ... bench:       6,086.40 ns/iter (+/- 109.08)
test decode::rustc_hex::bench4_16k            ... bench:      48,171.36 ns/iter (+/- 11,681.51)
test decode::rustc_hex::bench5_128k           ... bench:     893,339.65 ns/iter (+/- 46,849.14)
test decode::rustc_hex::bench6_1m             ... bench:   7,147,395.90 ns/iter (+/- 235,201.49)

test decode_to_slice::const_hex::bench1_32b   ... bench:           5.17 ns/iter (+/- 0.25)
test decode_to_slice::const_hex::bench2_256b  ... bench:          27.20 ns/iter (+/- 1.37)
test decode_to_slice::const_hex::bench3_2k    ... bench:         213.70 ns/iter (+/- 3.63)
test decode_to_slice::const_hex::bench4_16k   ... bench:       1,704.88 ns/iter (+/- 22.26)
test decode_to_slice::const_hex::bench5_128k  ... bench:      13,310.03 ns/iter (+/- 78.15)
test decode_to_slice::const_hex::bench6_1m    ... bench:     107,783.54 ns/iter (+/- 2,276.99)
test decode_to_slice::faster_hex::bench1_32b  ... bench:           6.71 ns/iter (+/- 0.05)
test decode_to_slice::faster_hex::bench2_256b ... bench:          29.46 ns/iter (+/- 0.41)
test decode_to_slice::faster_hex::bench3_2k   ... bench:         223.09 ns/iter (+/- 2.95)
test decode_to_slice::faster_hex::bench4_16k  ... bench:       1,758.51 ns/iter (+/- 14.19)
test decode_to_slice::faster_hex::bench5_128k ... bench:      13,838.49 ns/iter (+/- 252.65)
test decode_to_slice::faster_hex::bench6_1m   ... bench:     114,228.23 ns/iter (+/- 2,169.09)
test decode_to_slice::hex::bench1_32b         ... bench:          38.06 ns/iter (+/- 2.13)
test decode_to_slice::hex::bench2_256b        ... bench:         311.96 ns/iter (+/- 34.52)
test decode_to_slice::hex::bench3_2k          ... bench:       2,659.48 ns/iter (+/- 470.05)
test decode_to_slice::hex::bench4_16k         ... bench:      22,164.21 ns/iter (+/- 5,764.35)
test decode_to_slice::hex::bench5_128k        ... bench:     628,509.50 ns/iter (+/- 14,196.11)
test decode_to_slice::hex::bench6_1m          ... bench:   5,191,809.60 ns/iter (+/- 160,102.40)

test encode::const_hex::bench1_32b            ... bench:           7.06 ns/iter (+/- 0.15)
test encode::const_hex::bench2_256b           ... bench:          12.38 ns/iter (+/- 0.68)
test encode::const_hex::bench3_2k             ... bench:          74.18 ns/iter (+/- 1.46)
test encode::const_hex::bench4_16k            ... bench:         471.42 ns/iter (+/- 12.26)
test encode::const_hex::bench5_128k           ... bench:       3,756.98 ns/iter (+/- 76.00)
test encode::const_hex::bench6_1m             ... bench:      30,716.17 ns/iter (+/- 795.98)
test encode::faster_hex::bench1_32b           ... bench:          17.42 ns/iter (+/- 0.28)
test encode::faster_hex::bench2_256b          ... bench:          39.66 ns/iter (+/- 3.48)
test encode::faster_hex::bench3_2k            ... bench:          98.34 ns/iter (+/- 2.60)
test encode::faster_hex::bench4_16k           ... bench:         618.01 ns/iter (+/- 9.90)
test encode::faster_hex::bench5_128k          ... bench:       4,874.14 ns/iter (+/- 44.36)
test encode::faster_hex::bench6_1m            ... bench:      42,883.20 ns/iter (+/- 1,099.36)
test encode::hex::bench1_32b                  ... bench:         102.11 ns/iter (+/- 1.75)
test encode::hex::bench2_256b                 ... bench:         726.20 ns/iter (+/- 11.22)
test encode::hex::bench3_2k                   ... bench:       5,707.51 ns/iter (+/- 49.69)
test encode::hex::bench4_16k                  ... bench:      45,401.65 ns/iter (+/- 838.45)
test encode::hex::bench5_128k                 ... bench:     363,538.00 ns/iter (+/- 40,336.74)
test encode::hex::bench6_1m                   ... bench:   3,048,496.30 ns/iter (+/- 223,992.59)
test encode::rustc_hex::bench1_32b            ... bench:          54.28 ns/iter (+/- 2.53)
test encode::rustc_hex::bench2_256b           ... bench:         321.72 ns/iter (+/- 22.16)
test encode::rustc_hex::bench3_2k             ... bench:       2,474.80 ns/iter (+/- 204.16)
test encode::rustc_hex::bench4_16k            ... bench:      19,710.76 ns/iter (+/- 647.22)
test encode::rustc_hex::bench5_128k           ... bench:     158,282.15 ns/iter (+/- 2,594.36)
test encode::rustc_hex::bench6_1m             ... bench:   1,267,268.20 ns/iter (+/- 18,166.59)

test encode_to_slice::const_hex::bench1_32b   ... bench:           1.57 ns/iter (+/- 0.01)
test encode_to_slice::const_hex::bench2_256b  ... bench:           6.81 ns/iter (+/- 1.47)
test encode_to_slice::const_hex::bench3_2k    ... bench:          58.47 ns/iter (+/- 5.82)
test encode_to_slice::const_hex::bench4_16k   ... bench:         503.93 ns/iter (+/- 5.83)
test encode_to_slice::const_hex::bench5_128k  ... bench:       3,959.53 ns/iter (+/- 85.04)
test encode_to_slice::const_hex::bench6_1m    ... bench:      32,631.90 ns/iter (+/- 1,135.39)
test encode_to_slice::faster_hex::bench1_32b  ... bench:           4.37 ns/iter (+/- 0.13)
test encode_to_slice::faster_hex::bench2_256b ... bench:           8.13 ns/iter (+/- 0.14)
test encode_to_slice::faster_hex::bench3_2k   ... bench:          52.45 ns/iter (+/- 1.09)
test encode_to_slice::faster_hex::bench4_16k  ... bench:         474.66 ns/iter (+/- 8.71)
test encode_to_slice::faster_hex::bench5_128k ... bench:       3,545.60 ns/iter (+/- 75.68)
test encode_to_slice::faster_hex::bench6_1m   ... bench:      29,818.05 ns/iter (+/- 1,475.47)
test encode_to_slice::hex::bench1_32b         ... bench:          12.11 ns/iter (+/- 0.31)
test encode_to_slice::hex::bench2_256b        ... bench:         120.39 ns/iter (+/- 1.18)
test encode_to_slice::hex::bench3_2k          ... bench:         996.18 ns/iter (+/- 10.03)
test encode_to_slice::hex::bench4_16k         ... bench:       8,130.43 ns/iter (+/- 137.96)
test encode_to_slice::hex::bench5_128k        ... bench:      65,671.00 ns/iter (+/- 612.36)
test encode_to_slice::hex::bench6_1m          ... bench:     518,929.65 ns/iter (+/- 2,202.04)

test format::const_hex::bench1_32b            ... bench:          10.01 ns/iter (+/- 0.09)
test format::const_hex::bench2_256b           ... bench:          26.68 ns/iter (+/- 0.49)
test format::const_hex::bench3_2k             ... bench:         121.17 ns/iter (+/- 3.15)
test format::const_hex::bench4_16k            ... bench:       1,125.29 ns/iter (+/- 9.99)
test format::const_hex::bench5_128k           ... bench:       8,998.22 ns/iter (+/- 162.09)
test format::const_hex::bench6_1m             ... bench:      78,056.40 ns/iter (+/- 1,584.60)
test format::std::bench1_32b                  ... bench:         373.63 ns/iter (+/- 3.41)
test format::std::bench2_256b                 ... bench:       2,885.37 ns/iter (+/- 43.81)
test format::std::bench3_2k                   ... bench:      23,561.13 ns/iter (+/- 2,439.06)
test format::std::bench4_16k                  ... bench:     192,680.53 ns/iter (+/- 20,613.31)
test format::std::bench5_128k                 ... bench:   1,552,147.50 ns/iter (+/- 32,175.10)
test format::std::bench6_1m                   ... bench:  12,348,138.40 ns/iter (+/- 291,827.38)
```

## Acknowledgements

- [`hex`] for the initial encoding/decoding implementations
- [`faster-hex`] for the `x86`/`x86-64` check and decode implementations
- [dtolnay]/[itoa] for the initial crate/library API layout

[`hex`]: https://crates.io/crates/hex
[`faster-hex`]: https://crates.io/crates/faster-hex
[dtolnay]: https://github.com/dtolnay
[itoa]: https://github.com/dtolnay/itoa

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in these crates by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
</sub>
