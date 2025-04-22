use clap::Parser;
use foundry_config::{revive::ReviveConfig, SolcReq};
use serde::Serialize;
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[clap(next_help_heading = "Revive configuration")]
/// Compiler options for revive
pub struct ReviveOpts {
    #[clap(
        value_name = "REVIVE_COMPILE",
        help = "Enable compiling with revive",
        long = "revive-compile",
        visible_alias = "revive",
        action = clap::ArgAction::SetTrue,
    )]
    pub revive_compile: Option<bool>,

    /// Specify the revive version, or a path to a local resolc, to build with.
    ///
    /// Valid values follow the SemVer format `x.y.z-dev.n`, `revive:x.y.z-dev.n` or
    /// `path/to/resolc`.
    #[arg(
        long = "use-revive",
        help = "Use revive version",
        alias = "revive-compiler-version",
        help = "Use compiler version",
        value_name = "REVIVE_VERSION"
    )]
    #[serde(skip)]
    pub use_revive: Option<String>,
}

impl ReviveOpts {
    pub(crate) fn apply_overrides(&self, mut revive: ReviveConfig) -> ReviveConfig {
        macro_rules! set_if_some {
            ($src:expr, $dst:expr) => {
                if let Some(src) = $src {
                    $dst = src.into();
                }
            };
        }

        set_if_some!(
            self.use_revive.as_ref().map(|v| SolcReq::from(v.trim_start_matches("revive:"))),
            revive.revive
        );

        set_if_some!(self.revive_compile, revive.revive_compile);
        revive
    }
}
