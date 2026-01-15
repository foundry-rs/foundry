use crate::{filter::GlobMatcher, serde_helpers};
use foundry_compilers::{
    RestrictionsWithVersion,
    artifacts::{BytecodeHash, EvmVersion},
    multi::{MultiCompilerRestrictions, MultiCompilerSettings},
    settings::VyperRestrictions,
    solc::{Restriction, SolcRestrictions},
};
use semver::VersionReq;
use serde::{Deserialize, Deserializer, Serialize};

/// Keeps possible overrides for default settings which users may configure to construct additional
/// settings profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsOverrides {
    pub name: String,
    pub via_ir: Option<bool>,
    #[serde(default, with = "serde_helpers::display_from_str_opt")]
    pub evm_version: Option<EvmVersion>,
    pub optimizer: Option<bool>,
    pub optimizer_runs: Option<usize>,
    pub bytecode_hash: Option<BytecodeHash>,
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
            // Enable optimizer in optimizer runs set to a higher value than 0.
            if optimizer_runs > 0 && self.optimizer.is_none() {
                settings.solc.optimizer.enabled = Some(true);
            }
        }

        if let Some(bytecode_hash) = self.bytecode_hash {
            if let Some(metadata) = settings.solc.metadata.as_mut() {
                metadata.bytecode_hash = Some(bytecode_hash);
            } else {
                settings.solc.metadata = Some(bytecode_hash.into());
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RestrictionsError {
    #[error("specified both exact and relative restrictions for {0}")]
    BothExactAndRelative(&'static str),
}

/// Restrictions for compilation of given paths.
///
/// Only purpose of this type is to accept user input to later construct
/// `RestrictionsWithVersion<MultiCompilerRestrictions>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompilationRestrictions {
    pub paths: GlobMatcher,
    #[serde(default, deserialize_with = "deserialize_version_req")]
    pub version: Option<VersionReq>,
    pub via_ir: Option<bool>,
    pub bytecode_hash: Option<BytecodeHash>,

    pub min_optimizer_runs: Option<usize>,
    pub optimizer_runs: Option<usize>,
    pub max_optimizer_runs: Option<usize>,

    #[serde(default, with = "serde_helpers::display_from_str_opt")]
    pub min_evm_version: Option<EvmVersion>,
    #[serde(default, with = "serde_helpers::display_from_str_opt")]
    pub evm_version: Option<EvmVersion>,
    #[serde(default, with = "serde_helpers::display_from_str_opt")]
    pub max_evm_version: Option<EvmVersion>,
}

/// Custom deserializer for version field that rejects ambiguous bare version numbers.
fn deserialize_version_req<'de, D>(deserializer: D) -> Result<Option<VersionReq>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt_string: Option<String> = Option::deserialize(deserializer)?;
    let Some(opt_string) = opt_string else {
        return Ok(None);
    };

    let version = opt_string.trim();
    // Reject bare versions like "0.8.11" that lack an operator prefix
    if version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Err(serde::de::Error::custom(format!(
            "Invalid version format '{opt_string}' in compilation_restrictions. \
             Bare version numbers are ambiguous and default to caret requirements (e.g. '^{version}'). \
             Use an explicit constraint such as '={version}' for an exact version or '>={version}' for a minimum version."
        )));
    }

    let req = VersionReq::parse(&opt_string).map_err(|e| {
        serde::de::Error::custom(format!(
            "Invalid version requirement '{opt_string}': {e}. \
             Examples: '=0.8.11' (exact), '>=0.8.11' (minimum), '>=0.8.11 <0.9.0' (range)."
        ))
    })?;

    Ok(Some(req))
}

impl TryFrom<CompilationRestrictions> for RestrictionsWithVersion<MultiCompilerRestrictions> {
    type Error = RestrictionsError;

    fn try_from(value: CompilationRestrictions) -> Result<Self, Self::Error> {
        let (min_evm, max_evm) =
            match (value.min_evm_version, value.max_evm_version, value.evm_version) {
                (None, None, Some(exact)) => (Some(exact), Some(exact)),
                (min, max, None) => (min, max),
                _ => return Err(RestrictionsError::BothExactAndRelative("evm_version")),
            };
        let (min_opt, max_opt) =
            match (value.min_optimizer_runs, value.max_optimizer_runs, value.optimizer_runs) {
                (None, None, Some(exact)) => (Some(exact), Some(exact)),
                (min, max, None) => (min, max),
                _ => return Err(RestrictionsError::BothExactAndRelative("optimizer_runs")),
            };
        Ok(Self {
            restrictions: MultiCompilerRestrictions {
                solc: SolcRestrictions {
                    evm_version: Restriction { min: min_evm, max: max_evm },
                    via_ir: value.via_ir,
                    optimizer_runs: Restriction { min: min_opt, max: max_opt },
                    bytecode_hash: value.bytecode_hash,
                },
                vyper: VyperRestrictions {
                    evm_version: Restriction { min: min_evm, max: max_evm },
                },
            },
            version: value.version,
        })
    }
}
