use crate::{output::sources::VersionedSourceFile, ArtifactOutput};
use foundry_compilers_artifacts::{
    hh::{HardhatArtifact, HH_ARTIFACT_VERSION},
    Contract, SourceFile,
};
use std::path::Path;

/// Hardhat style artifacts handler
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardhatArtifacts {
    _priv: (),
}

impl ArtifactOutput for HardhatArtifacts {
    type Artifact = HardhatArtifact;
    type CompilerContract = Contract;

    fn contract_to_artifact(
        &self,
        file: &Path,
        name: &str,
        contract: Contract,
        _source_file: Option<&SourceFile>,
    ) -> Self::Artifact {
        let (bytecode, link_references, deployed_bytecode, deployed_link_references) =
            if let Some(evm) = contract.evm {
                let (deployed_bytecode, deployed_link_references) =
                    if let Some(code) = evm.deployed_bytecode.and_then(|code| code.bytecode) {
                        (Some(code.object), code.link_references)
                    } else {
                        (None, Default::default())
                    };

                let (bytecode, link_ref) = if let Some(bc) = evm.bytecode {
                    (Some(bc.object), bc.link_references)
                } else {
                    (None, Default::default())
                };

                (bytecode, link_ref, deployed_bytecode, deployed_link_references)
            } else {
                (Default::default(), Default::default(), None, Default::default())
            };

        HardhatArtifact {
            format: HH_ARTIFACT_VERSION.to_string(),
            contract_name: name.to_string(),
            source_name: file.to_string_lossy().to_string(),
            abi: contract.abi.unwrap_or_default(),
            bytecode,
            deployed_bytecode,
            link_references,
            deployed_link_references,
        }
    }

    fn standalone_source_file_to_artifact(
        &self,
        _path: &Path,
        _file: &VersionedSourceFile,
    ) -> Option<Self::Artifact> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Artifact;

    #[test]
    fn can_parse_hh_artifact() {
        let s = include_str!("../../../../test-data/hh-greeter-artifact.json");
        let artifact = serde_json::from_str::<HardhatArtifact>(s).unwrap();
        let compact = artifact.into_compact_contract();
        assert!(compact.abi.is_some());
        assert!(compact.bin.is_some());
        assert!(compact.bin_runtime.is_some());
    }
}
