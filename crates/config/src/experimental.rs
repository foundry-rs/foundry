//! Experimental Solidity compiler configuration.

use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentalConfig {
    /// Enable Solidity's experimental SSA CFG backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_ssa_cfg: Option<bool>,
}

impl ExperimentalConfig {
    pub const fn is_empty(&self) -> bool {
        self.via_ssa_cfg.is_none()
    }

    pub fn extend_solc_cli_args(&self, extra_args: &mut Vec<String>) {
        if self.via_ssa_cfg == Some(true) {
            Self::push_arg(extra_args, "--experimental");
            Self::push_arg(extra_args, "--via-ssa-cfg");
        }
    }

    fn push_arg(extra_args: &mut Vec<String>, arg: &str) {
        if !extra_args.iter().any(|existing| existing == arg) {
            extra_args.push(arg.to_string());
        }
    }
}
