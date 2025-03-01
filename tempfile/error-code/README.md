# error-code

[![Crates.io](https://img.shields.io/crates/v/error-code.svg)](https://crates.io/crates/error-code)
[![Documentation](https://docs.rs/error-code/badge.svg)](https://docs.rs/crate/error-code/)
[![Build](https://github.com/DoumanAsh/error-code/workflows/Rust/badge.svg)](https://github.com/DoumanAsh/error-code/actions?query=workflow%3ARust)

Error code library provides generic errno/winapi error wrapper

User can define own `Category` if you want to create new error wrapper.

## Usage

```rust
use error_code::ErrorCode;

use std::fs::File;

File::open("non_existing");
println!("{}", ErrorCode::last_system());
```
