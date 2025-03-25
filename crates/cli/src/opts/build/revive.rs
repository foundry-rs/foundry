use clap::Parser;
use foundry_config::revive::ReviveConfig;
use serde::Serialize;
use std::path::PathBuf;

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

    #[clap(
        long = "revive-path",
        help = "Specify a revive path to be used",
        value_name = "REVIVE_PATH"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revive_path: Option<PathBuf>,

    #[clap(
        help = "Solc compiler path to use when compiling with revive",
        long = "revive-solc-path",
        value_name = "REVIVE_SOLC_PATH"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solc_path: Option<PathBuf>,
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

        set_if_some!(self.revive_compile, revive.revive_compile);
        set_if_some!(self.revive_path.clone(), revive.revive_path);
        set_if_some!(self.solc_path.clone(), revive.solc_path);

        revive
    }
}
