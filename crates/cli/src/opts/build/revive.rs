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
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
        env = "REVIVE_COMPILE" 
    )]
    pub revive_compile: bool,

    #[clap(
        long = "revive-path",
        visible_alias = "revive",
        help = "Specify a revive path to be used",
        value_name = "REVIVE_PATH",
        env = "REVIVE_PATH"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revive_path: Option<PathBuf>,
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

        set_if_some!(self.revive_path.clone(), revive.revive_path);
        revive.revive_compile = self.revive_compile;

        revive
    }
}

impl From<ReviveOpts> for ReviveConfig {
    fn from(args: ReviveOpts) -> Self {
        Self { revive_compile: args.revive_compile, revive_path: args.revive_path }
    }
}
