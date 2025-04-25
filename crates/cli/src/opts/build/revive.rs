use clap::Parser;
use foundry_config::{revive::ResolcConfig, SolcReq};
use serde::Serialize;
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[clap(next_help_heading = "Resolc configuration")]
/// Compiler options for resolc
pub struct ResolcOpts {
    #[clap(
        value_name = "RESOLC_COMPILE",
        help = "Enable compiling with resolc",
        long = "resolc-compile",
        visible_alias = "resolc",
        action = clap::ArgAction::SetTrue,
    )]
    pub resolc_compile: Option<bool>,

    /// Specify the resolc version, or a path to a local resolc, to build with.
    ///
    /// Valid values follow the SemVer format `x.y.z-dev.n`, `resolc:x.y.z-dev.n` or
    /// `path/to/resolc`.
    #[arg(
        long = "use-resolc",
        help = "Use resolc version",
        alias = "resolc-compiler-version",
        help = "Use compiler version",
        value_name = "RESOLC_VERSION"
    )]
    #[serde(skip)]
    pub use_resolc: Option<String>,
}

impl ResolcOpts {
    pub(crate) fn apply_overrides(&self, mut resolc: ResolcConfig) -> ResolcConfig {
        macro_rules! set_if_some {
            ($src:expr, $dst:expr) => {
                if let Some(src) = $src {
                    $dst = src.into();
                }
            };
        }

        set_if_some!(
            self.use_resolc.as_ref().map(|v| SolcReq::from(v.trim_start_matches("resolc:"))),
            resolc.resolc
        );

        set_if_some!(self.resolc_compile, resolc.resolc_compile);
        resolc
    }
}
