use crate::{
    Ast, CompactBytecode, CompactContract, CompactContractBytecode, CompactContractBytecodeCow,
    CompactDeployedBytecode, DevDoc, Ewasm, FunctionDebugData, GasEstimates, GeneratedSource,
    Metadata, Offsets, SourceFile, StorageLayout, UserDoc,
};
use alloy_json_abi::JsonAbi;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap};

/// Represents the `Artifact` that `ConfigurableArtifacts` emits.
///
/// This is essentially a superset of [`CompactContractBytecode`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurableContractArtifact {
    /// The Ethereum Contract ABI. If empty, it is represented as an empty
    /// array. See <https://docs.soliditylang.org/en/develop/abi-spec.html>
    pub abi: Option<JsonAbi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytecode: Option<CompactBytecode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_bytecode: Option<CompactDeployedBytecode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_assembly: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opcodes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_identifiers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_sources: Vec<GeneratedSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_debug_data: Option<BTreeMap<String, FunctionDebugData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_estimates: Option<GasEstimates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_metadata: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_layout: Option<StorageLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transient_storage_layout: Option<StorageLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub userdoc: Option<UserDoc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devdoc: Option<DevDoc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_optimized: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_optimized_ast: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ewasm: Option<Ewasm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ast: Option<Ast>,
    /// The identifier of the source file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
}

impl ConfigurableContractArtifact {
    /// Returns the inner element that contains the core bytecode related information
    pub fn into_contract_bytecode(self) -> CompactContractBytecode {
        self.into()
    }

    /// Looks for all link references in deployment and runtime bytecodes
    pub fn all_link_references(&self) -> BTreeMap<String, BTreeMap<String, Vec<Offsets>>> {
        let mut links = BTreeMap::new();
        if let Some(bcode) = &self.bytecode {
            links.extend(bcode.link_references.clone());
        }

        if let Some(d_bcode) = &self.deployed_bytecode {
            if let Some(bcode) = &d_bcode.bytecode {
                links.extend(bcode.link_references.clone());
            }
        }
        links
    }

    /// Returns the source file of this artifact's contract
    pub fn source_file(&self) -> Option<SourceFile> {
        self.id.map(|id| SourceFile { id, ast: self.ast.clone() })
    }
}

impl From<ConfigurableContractArtifact> for CompactContractBytecode {
    fn from(artifact: ConfigurableContractArtifact) -> Self {
        Self {
            abi: artifact.abi,
            bytecode: artifact.bytecode,
            deployed_bytecode: artifact.deployed_bytecode,
        }
    }
}

impl From<ConfigurableContractArtifact> for CompactContract {
    fn from(artifact: ConfigurableContractArtifact) -> Self {
        CompactContractBytecode::from(artifact).into()
    }
}

impl<'a> From<&'a ConfigurableContractArtifact> for CompactContractBytecodeCow<'a> {
    fn from(artifact: &'a ConfigurableContractArtifact) -> Self {
        CompactContractBytecodeCow {
            abi: artifact.abi.as_ref().map(Cow::Borrowed),
            bytecode: artifact.bytecode.as_ref().map(Cow::Borrowed),
            deployed_bytecode: artifact.deployed_bytecode.as_ref().map(Cow::Borrowed),
        }
    }
}
