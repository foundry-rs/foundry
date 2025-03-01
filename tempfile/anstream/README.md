# anstream

> A simple cross platform library for writing colored text to a terminal.

*A portmanteau of "ansi stream"*

[![Documentation](https://img.shields.io/badge/docs-master-blue.svg)][Documentation]
![License](https://img.shields.io/crates/l/anstream.svg)
[![Crates Status](https://img.shields.io/crates/v/anstream.svg)](https://crates.io/crates/anstream)

Specialized `stdout` and `stderr` that accept ANSI escape codes and adapt them
based on the terminal's capabilities.

`anstream::adapter::strip_str` may also be of interest on its own for low
overhead stripping of ANSI escape codes.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

[Crates.io]: https://crates.io/crates/anstream
[Documentation]: https://docs.rs/anstream
