use foundry_compilers_artifacts_solc::{
    output_selection::OutputSelection, serde_helpers, EvmVersion,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub const VYPER_SEARCH_PATHS: Version = VYPER_0_4;
pub const VYPER_BERLIN: Version = Version::new(0, 3, 0);
pub const VYPER_PARIS: Version = Version::new(0, 3, 7);
pub const VYPER_SHANGHAI: Version = Version::new(0, 3, 8);
pub const VYPER_CANCUN: Version = Version::new(0, 3, 8);

const VYPER_0_4: Version = Version::new(0, 4, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VyperOptimizationMode {
    Gas,
    Codesize,
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VyperSettings {
    #[serde(
        default,
        with = "serde_helpers::display_from_str_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub evm_version: Option<EvmVersion>,
    /// Optimization mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize: Option<VyperOptimizationMode>,
    /// Whether or not the bytecode should include Vyper's signature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytecode_metadata: Option<bool>,
    pub output_selection: OutputSelection,
    #[serde(rename = "search_paths", skip_serializing_if = "Option::is_none")]
    pub search_paths: Option<BTreeSet<PathBuf>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_codegen: Option<bool>,
}

impl VyperSettings {
    pub fn strip_prefix(&mut self, base: &Path) {
        self.output_selection = OutputSelection(
            std::mem::take(&mut self.output_selection.0)
                .into_iter()
                .map(|(file, selection)| {
                    (
                        Path::new(&file)
                            .strip_prefix(base)
                            .map(|p| p.display().to_string())
                            .unwrap_or(file),
                        selection,
                    )
                })
                .collect(),
        );
        self.search_paths = self.search_paths.as_ref().map(|paths| {
            paths.iter().map(|p| p.strip_prefix(base).unwrap_or(p.as_path()).into()).collect()
        });
    }

    /// Sanitize the output selection.
    #[allow(clippy::collapsible_if)]
    pub fn sanitize_output_selection(&mut self, version: &Version) {
        self.output_selection.0.values_mut().for_each(|selection| {
            selection.values_mut().for_each(|selection| {
                // During caching we prune output selection for some of the sources, however, Vyper
                // will reject `[]` as an output selection, so we are adding "abi" as a default
                // output selection which is cheap to be produced.
                if selection.is_empty() {
                    selection.push("abi".to_string())
                }

                // Unsupported selections.
                #[rustfmt::skip]
                selection.retain(|selection| {
                    if *version < VYPER_0_4 {
                        if matches!(
                            selection.as_str(),
                            | "evm.bytecode.sourceMap" | "evm.deployedBytecode.sourceMap"
                        ) {
                            return false;
                        }
                    }

                    if matches!(
                        selection.as_str(),
                        | "evm.bytecode.sourceMap" | "evm.deployedBytecode.sourceMap"
                        // https://github.com/vyperlang/vyper/issues/4389
                        | "evm.bytecode.linkReferences" | "evm.deployedBytecode.linkReferences"
                        | "evm.deployedBytecode.immutableReferences"
                    ) {
                        return false;
                    }

                    true
                });
            })
        });
    }

    /// Sanitize the settings based on the compiler version.
    pub fn sanitize(&mut self, version: &Version) {
        if version < &VYPER_SEARCH_PATHS {
            self.search_paths = None;
        }

        self.sanitize_output_selection(version);
        self.normalize_evm_version(version);
    }

    /// Sanitize the settings based on the compiler version.
    pub fn sanitized(mut self, version: &Version) -> Self {
        self.sanitize(version);
        self
    }

    /// Adjusts the EVM version based on the compiler version.
    pub fn normalize_evm_version(&mut self, version: &Version) {
        if let Some(evm_version) = &mut self.evm_version {
            *evm_version = if *evm_version >= EvmVersion::Cancun && *version >= VYPER_CANCUN {
                EvmVersion::Cancun
            } else if *evm_version >= EvmVersion::Shanghai && *version >= VYPER_SHANGHAI {
                EvmVersion::Shanghai
            } else if *evm_version >= EvmVersion::Paris && *version >= VYPER_PARIS {
                EvmVersion::Paris
            } else if *evm_version >= EvmVersion::Berlin && *version >= VYPER_BERLIN {
                EvmVersion::Berlin
            } else {
                *evm_version
            };
        }
    }
}
