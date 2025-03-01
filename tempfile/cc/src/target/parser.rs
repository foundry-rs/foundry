use std::{env, str::FromStr};

use crate::{
    target::{llvm, TargetInfo},
    utilities::OnceLock,
    Error, ErrorKind,
};

#[derive(Debug)]
struct TargetInfoParserInner {
    full_arch: Box<str>,
    arch: Box<str>,
    vendor: Box<str>,
    os: Box<str>,
    env: Box<str>,
    abi: Box<str>,
    llvm_target: Box<str>,
}

impl TargetInfoParserInner {
    fn from_cargo_environment_variables() -> Result<Self, Error> {
        // `TARGET` must be present.
        //
        // No need to emit `rerun-if-env-changed` for this,
        // as it is controlled by Cargo itself.
        #[allow(clippy::disallowed_methods)]
        let target_triple = env::var("TARGET").map_err(|err| {
            Error::new(
                ErrorKind::EnvVarNotFound,
                format!("failed reading TARGET: {err}"),
            )
        })?;

        // Parse the full architecture name from the target triple.
        let (full_arch, _rest) = target_triple.split_once('-').ok_or(Error::new(
            ErrorKind::InvalidTarget,
            format!("target `{target_triple}` had an unknown architecture"),
        ))?;

        let cargo_env = |name, fallback: Option<&str>| -> Result<Box<str>, Error> {
            // No need to emit `rerun-if-env-changed` for these,
            // as they are controlled by Cargo itself.
            #[allow(clippy::disallowed_methods)]
            match env::var(name) {
                Ok(var) => Ok(var.into_boxed_str()),
                Err(err) => match fallback {
                    Some(fallback) => Ok(fallback.into()),
                    None => Err(Error::new(
                        ErrorKind::EnvVarNotFound,
                        format!("did not find fallback information for target `{target_triple}`, and failed reading {name}: {err}"),
                    )),
                },
            }
        };

        // Prefer to use `CARGO_ENV_*` if set, since these contain the most
        // correct information relative to the current `rustc`, and makes it
        // possible to support custom target JSON specs unknown to `rustc`.
        //
        // NOTE: If the user is using an older `rustc`, that data may be older
        // than our pre-generated data, but we still prefer Cargo's view of
        // the world, since at least `cc` won't differ from `rustc` in that
        // case.
        //
        // These may not be set in case the user depended on being able to
        // just set `TARGET` outside of build scripts; in those cases, fall
        // back back to data from the known set of target triples instead.
        //
        // See discussion in #1225 for further details.
        let fallback_target = TargetInfo::from_str(&target_triple).ok();
        let ft = fallback_target.as_ref();
        let arch = cargo_env("CARGO_CFG_TARGET_ARCH", ft.map(|t| t.arch))?;
        let vendor = cargo_env("CARGO_CFG_TARGET_VENDOR", ft.map(|t| t.vendor))?;
        let os = cargo_env("CARGO_CFG_TARGET_OS", ft.map(|t| t.os))?;
        let env = cargo_env("CARGO_CFG_TARGET_ENV", ft.map(|t| t.env))?;
        // `target_abi` was stabilized in Rust 1.78, which is higher than our
        // MSRV, so it may not always be available; In that case, fall back to
        // `""`, which is _probably_ correct for unknown target triples.
        let abi = cargo_env("CARGO_CFG_TARGET_ABI", ft.map(|t| t.abi))
            .unwrap_or_else(|_| String::default().into_boxed_str());

        // Prefer `rustc`'s LLVM target triple information.
        let llvm_target = match fallback_target {
            Some(ft) => ft.llvm_target.to_string(),
            None => llvm::guess_llvm_target_triple(full_arch, &vendor, &os, &env, &abi),
        };

        Ok(Self {
            full_arch: full_arch.to_string().into_boxed_str(),
            arch,
            vendor,
            os,
            env,
            abi,
            llvm_target: llvm_target.into_boxed_str(),
        })
    }
}

/// Parser for [`TargetInfo`], contains cached information.
#[derive(Default, Debug)]
pub(crate) struct TargetInfoParser(OnceLock<Result<TargetInfoParserInner, Error>>);

impl TargetInfoParser {
    pub fn parse_from_cargo_environment_variables(&self) -> Result<TargetInfo<'_>, Error> {
        match self
            .0
            .get_or_init(TargetInfoParserInner::from_cargo_environment_variables)
        {
            Ok(TargetInfoParserInner {
                full_arch,
                arch,
                vendor,
                os,
                env,
                abi,
                llvm_target,
            }) => Ok(TargetInfo {
                full_arch,
                arch,
                vendor,
                os,
                env,
                abi,
                llvm_target,
            }),
            Err(e) => Err(e.clone()),
        }
    }
}
