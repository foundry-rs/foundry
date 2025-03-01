//! Very basic parsing of `rustc` target triples.
//!
//! See the `target-lexicon` crate for a more principled approach to this.

use std::str::FromStr;

use crate::{Error, ErrorKind};

mod apple;
mod generated;
mod llvm;
mod parser;

pub(crate) use parser::TargetInfoParser;

/// Information specific to a `rustc` target.
///
/// See <https://doc.rust-lang.org/cargo/appendix/glossary.html#target>.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TargetInfo<'a> {
    /// The full architecture, including the subarchitecture.
    ///
    /// This differs from `cfg!(target_arch)`, which only specifies the
    /// overall architecture, which is too coarse for certain cases.
    pub full_arch: &'a str,
    /// The overall target architecture.
    ///
    /// This is the same as the value of `cfg!(target_arch)`.
    pub arch: &'a str,
    /// The target vendor.
    ///
    /// This is the same as the value of `cfg!(target_vendor)`.
    pub vendor: &'a str,
    /// The operating system, or `none` on bare-metal targets.
    ///
    /// This is the same as the value of `cfg!(target_os)`.
    pub os: &'a str,
    /// The environment on top of the operating system.
    ///
    /// This is the same as the value of `cfg!(target_env)`.
    pub env: &'a str,
    /// The ABI on top of the operating system.
    ///
    /// This is the same as the value of `cfg!(target_abi)`.
    pub abi: &'a str,
    /// The unversioned LLVM/Clang target triple.
    ///
    /// NOTE: You should never need to match on this explicitly, use the other
    /// fields on [`TargetInfo`] instead.
    pub llvm_target: &'a str,
}

impl FromStr for TargetInfo<'_> {
    type Err = Error;

    /// This will fail when using a custom target triple unknown to `rustc`.
    fn from_str(target_triple: &str) -> Result<Self, Error> {
        if let Ok(index) =
            generated::LIST.binary_search_by_key(&target_triple, |(target_triple, _)| target_triple)
        {
            let (_, info) = &generated::LIST[index];
            Ok(info.clone())
        } else {
            Err(Error::new(
                ErrorKind::UnknownTarget,
                format!(
                    "unknown target `{target_triple}`.

NOTE: `cc-rs` only supports a fixed set of targets when not in a build script.
- If adding a new target, you will need to fork of `cc-rs` until the target
  has landed on nightly and the auto-generated list has been updated. See also
  the `rustc` dev guide on adding a new target:
  https://rustc-dev-guide.rust-lang.org/building/new-target.html
- If using a custom target, prefer to upstream it to `rustc` if possible,
  otherwise open an issue with `cc-rs`:
  https://github.com/rust-lang/cc-rs/issues/new
"
                ),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::TargetInfo;

    // Test tier 1 targets
    #[test]
    fn tier1() {
        let targets = [
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "i686-pc-windows-gnu",
            "i686-pc-windows-msvc",
            "i686-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-gnu",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
        ];

        for target in targets {
            // Check that it parses
            let _ = TargetInfo::from_str(target).unwrap();
        }
    }

    // Various custom target triples not (or no longer) known by `rustc`
    #[test]
    fn cannot_parse_extra() {
        let targets = [
            "aarch64-unknown-none-gnu",
            "aarch64-uwp-windows-gnu",
            "arm-frc-linux-gnueabi",
            "arm-unknown-netbsd-eabi",
            "armv7neon-unknown-linux-gnueabihf",
            "armv7neon-unknown-linux-musleabihf",
            "thumbv7-unknown-linux-gnueabihf",
            "thumbv7-unknown-linux-musleabihf",
            "x86_64-rumprun-netbsd",
            "x86_64-unknown-linux",
        ];

        for target in targets {
            // Check that it does not parse
            let _ = TargetInfo::from_str(target).unwrap_err();
        }
    }
}
