# env_home rust crate

A pure-Rust crate for determining user home directories via environment variables
in a platform independent manner with no external dependencies.

Check `HOME` on Unix and `USERPROFILE` on Windows.

## Description

env_home is a general purpose crate for determining the current user
home directory via environment variables.

It can be used as drop-in replacement for
[`std::env::home_dir` (deprecated)](https://doc.rust-lang.org/std/env/fn.home_dir.html)
from the rust standard library.

Unlike `std::env::home_dir` this crate **only** looks at environment variables
and does attempt to fallback on platform specific APIs. As a result implementation
of `env_home_dir` is [very simple](src/lib.rs) with no dependencies on other crates.

This functionality is comparable to Golang's [os.UserHomeDir()](https://pkg.go.dev/os#UserHomeDir)
or Python's [Path.home()](https://docs.python.org/3/library/pathlib.html#pathlib.Path.home).

## env_home::env_home_dir Behavior

The API of this crate is a single function `env_home_dir`
which attempts to fetch a user's home directory from environment variables
in a platform independent way supporting Windows and Unix (Linux/MacOS/BSD/WSL, etc).

| Platform                          | Environment Variable | Example           |
| --------------------------------- | -------------------- | ----------------- |
| MacOS, Linux or other Unix        | `HOME`               | `/home/user`      |
| Windows Subsystem for Linux (WSL) | `HOME`               | `/home/user`      |
| Windows Native                    | `USERPROFILE`        | `C:\\Users\\user` |
| Others (WASM, etc)                | N/A                  | None              |

1. If the environment variable is unset, `None` is returned.
2. If the environment variable is set to an empty string, `None` is returned.
3. On non-unix / non-windows platforms (like WASM) that don't implement
   a home directory `None` will be returned.
4. If the environment variable is set to a non-empty string, the value is returned as a `PathBuf`.

That's it.

If you need a more full-service crate consider using the [dirs](https://crates.io/crates/dirs) crate.

## Usage

```shell
cargo add env_home
```

Crate exports a single function `env_home_dir` that returns `Option<PathBuf>`

```rust
use env_home::env_home_dir as home_dir;
fn main() {
    match home_dir() {
        Some(path) => println!("User home directory: {}", path.display()),
        None => println!("No home found. HOME/USERPROFILE not set or empty"),
    }
}
```

See the [std::path::PathBuf documentation](https://doc.rust-lang.org/std/path/struct.PathBuf.html)
for more information on how to use `PathBuf` objects.

## Differences with `std::env::home_dir`

env_home_dir returns `None` instead of `""` when `HOME` or `USERPROFILE` is set to an empty string.

I believe
[`std::env::home_dir`](https://doc.rust-lang.org/std/env/fn.home_dir.html)
was trying to be too smart. It calls Platform specific APIs like
([GetUserProfileDirectoryW](https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-getuserprofiledirectoryw)
on Windows or [getpwuid_r](https://linux.die.net/man/3/getpwuid_r) on Unix
as a fallback when `HOME` or `USERPROFILE` environment variables are not set.
We just give up and return `None`.

This crate exists because the behavior of
[`home_dir`](https://doc.rust-lang.org/std/env/fn.home_dir.html)
provided by the standard library may be unexpected on Windows.
And thus was
[deprecated](https://doc.rust-lang.org/std/env/fn.home_dir.html#deprecation)
and has remained broken / unfixed since Rust 1.29.0 (Sept 2018).

## As an alternative to the `home` crate

Although many projects have switched from `std::env::home_dir` to `home::home_dir` provided
by the [home](https://crates.io/crates/home) crate because it was maintained by the cargo team
and thus presumably more "official". The Cargo team has clarified that the `home` crate is
not intended for general use:

> "the cargo team doesn't want to take on the maintenance of home as a general-purpose crate for the community" [...]
> "we are thinking of documenting that home is not intended for anything but use inside of cargo and rustup, and suggest people use some other crate instead."
> [source](https://github.com/rust-lang/cargo/issues/12297)

As a result the `home` crate refuses to compile for WASM target and they have have no plans to fix this.

env_home crate implements a fallback no-op which returns `None`
on non-windows / non-unix platforms like WASM.

## Other Notes

Using
[std::env::set_var](https://doc.rust-lang.org/std/env/fn.set_var.html) to alter your environment
at runtime is unsafe in multi-threaded applications. Full stop.
It may result in random panics or undefined behavior. You have have been warned.

Bonus: cargo runs tests in parallel threads by-default, so even if you app is not multi-threaded
if you have tests that invoke `std::env::set_var` be sure to set `RUST_TEST_THREADS=1`
or use `cargo test -- --test-threads=1` or your tests may intermittently panic and fail.

See [rust-lang/rust#27970](https://github.com/rust-lang/rust/issues/27970) and
[Setenv is not Thread Safe and C Doesn't Want to Fix It](https://www.evanjones.ca/setenv-is-not-thread-safe.html)
for more.

## License

Copyright (c) 2024 Peter Tripp

This project is licensed under the [MIT License](LICENSE-MIT)
or [Apache License, Version 2.0](LICENSE-APACHE) at your option.
