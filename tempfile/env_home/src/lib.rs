// Copyright 2024 Peter Tripp
//! env_home is a general purpose crate for determining the current user
//! home directory in a platform independant manner via enviornment variables.
//!
//! This crate is implemented in pure-rust and has no external dependencies.
//!
//! It is meant as a lightweight, drop-in replacement for `std::env::home_dir`
//! provided by the Rust Standard Library which was
//! [deprecated](https://doc.rust-lang.org/std/env/fn.home_dir.html#deprecation)
//! in Rust 1.29.0 (Sept 2018).
//!
//! ## Usage
//! ```rust
//! use env_home::env_home_dir as home_dir;
//! fn main() {
//!     match home_dir() {
//!         Some(path) => println!("User home directory: {}", path.display()),
//!         None => println!("No home found. HOME/USERPROFILE not set or empty"),
//!     }
//! }
//! ```

#[cfg(unix)]
/// Returns the path of the current user’s home directory if known.
///
/// * On Unix, this function will check the `HOME` environment variable
/// * On Windows, it will check the `USERPROFILE` environment variable
/// * On other platforms, this function will always return `None`
/// * If the environment variable is unset, return `None`
/// * If the environment variable is set to an empty string, return `None`
///
/// Note: the behavior of this function differs from
///   [`std::env::home_dir`](https://doc.rust-lang.org/std/env/fn.home_dir.html),
///   [`home::home_dir`](https://docs.rs/home/latest/home/fn.home_dir.html), and
///   [`dirs::home_dir`](https://docs.rs/dirs/latest/dirs/fn.home_dir.html).
///
/// This function returns `None` when the environment variable is set but empty.
/// Those implementations return the empty string `""` instead.
pub fn env_home_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME");
    match home {
        Ok(val) if !val.is_empty() => Some(std::path::PathBuf::from(val)),
        _ => None,
    }
}

#[cfg(windows)]
/// Returns the path of the current user’s home directory if known.
pub fn env_home_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("USERPROFILE");
    match home {
        Ok(val) if !val.is_empty() => Some(std::path::PathBuf::from(val)),
        _ => None,
    }
}

#[cfg(all(not(windows), not(unix)))]
/// Returns the path of the current user’s home directory if known.
pub fn env_home_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::env_home_dir;
    use std::env;
    use std::path::PathBuf;

    /*
    Note! Do not run these tests in parallel, as they modify the environment.
    By default `cargo test` will run tests in parallel (multi-threaded) which
    is unsafe and will cause intermittent panics. To run tests sequentially
    use `cargo test -- --test-threads=1`.

    More info:
    - https://doc.rust-lang.org/std/env/fn.set_var.html
    - https://github.com/rust-lang/rust/issues/27970

    Possible future test cases:
    - Test non-windows/non-unix platforms (WASM, etc.)
    - Test non-utf8 paths (should return None)
    */

    #[cfg(any(unix, windows))]
    #[test]
    fn env_home_test() {
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let old = std::env::var(home_var).unwrap();

        // Sanity checks
        assert_ne!(env_home_dir(), None, "HOME/USERPROFILE is unset");
        assert_eq!(env_home_dir(), Some(PathBuf::from(old.clone())));

        // Test when var unset.
        env::remove_var(home_var);
        assert_eq!(env_home_dir(), None);

        // Test when var set to empty string
        env::set_var(home_var, "");
        assert_eq!(env_home_dir(), None);

        // Tests a sensible platform specific home directory.
        let temp_dir = if cfg!(windows) { "C:\\temp" } else { "/tmp" };
        std::env::set_var(home_var, temp_dir);
        assert_eq!(env_home_dir(), Some(std::path::PathBuf::from(temp_dir)));

        env::set_var(home_var, old);
    }
}
