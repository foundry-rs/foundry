use crate::{filter::GlobMatcher, serde_helpers};
use foundry_compilers::{
    artifacts::EvmVersion,
    multi::{MultiCompilerRestrictions, MultiCompilerSettings},
    settings::VyperRestrictions,
    solc::{EvmVersionRestriction, SolcRestrictions},
    RestrictionsWithVersion,
};
use semver::VersionReq;
use serde::{Deserialize, Serialize};

/// Keeps possible overrides for default settings which users may configure to construct additional
/// settings profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsOverrides {
    pub name: String,
    via_ir: Option<bool>,
    #[serde(default, with = "serde_helpers::display_from_str_opt")]
    evm_version: Option<EvmVersion>,
    optimizer: Option<bool>,
    optimizer_runs: Option<usize>,
}

impl SettingsOverrides {
    /// Applies the overrides to the given settings.
    pub fn apply(&self, settings: &mut MultiCompilerSettings) {
        if let Some(via_ir) = self.via_ir {
            settings.solc.via_ir = Some(via_ir);
        }

        if let Some(evm_version) = self.evm_version {
            settings.solc.evm_version = Some(evm_version);
            settings.vyper.evm_version = Some(evm_version);
        }

        if let Some(enabled) = self.optimizer {
            settings.solc.optimizer.enabled = Some(enabled);
        }

        if let Some(optimizer_runs) = self.optimizer_runs {
            settings.solc.optimizer.runs = Some(optimizer_runs);
        }
    }
}

/// Restrictions for compilation of given paths.
///
/// Only purpose of this type is to accept user input to later construct
/// `RestrictionsWithVersion<MultiCompilerRestrictions>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompilationRestrictions {
    pub paths: GlobMatcher,
    version: Option<VersionReq>,
    via_ir: Option<bool>,
    min_optimizer_runs: Option<usize>,
    max_optimizer_runs: Option<usize>,
    #[serde(flatten)]
    evm_version: EvmVersionRestriction,
}

impl From<CompilationRestrictions> for RestrictionsWithVersion<MultiCompilerRestrictions> {
    fn from(value: CompilationRestrictions) -> Self {
        Self {
            restrictions: MultiCompilerRestrictions {
                solc: SolcRestrictions {
                    evm_version: value.evm_version,
                    via_ir: value.via_ir,
                    min_optimizer_runs: value.min_optimizer_runs,
                    max_optimizer_runs: value.max_optimizer_runs,
                },
                vyper: VyperRestrictions { evm_version: value.evm_version },
            },
            version: value.version,
        }
    }
}
