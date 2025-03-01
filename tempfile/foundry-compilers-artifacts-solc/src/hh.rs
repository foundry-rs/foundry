//! Hardhat support

use crate::{
    Bytecode, BytecodeObject, CompactContract, CompactContractBytecode, CompactContractBytecodeCow,
    ContractBytecode, DeployedBytecode, Offsets,
};
use alloy_json_abi::JsonAbi;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::btree_map::BTreeMap};

pub const HH_ARTIFACT_VERSION: &str = "hh-sol-artifact-1";

/// A hardhat artifact
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardhatArtifact {
    #[serde(rename = "_format")]
    pub format: String,
    /// A string with the contract's name.
    pub contract_name: String,
    /// The source name of this contract in the workspace like `contracts/Greeter.sol`
    pub source_name: String,
    /// The contract's ABI
    pub abi: JsonAbi,
    /// A "0x"-prefixed hex string of the unlinked deployment bytecode. If the contract is not
    /// deployable, this has the string "0x"
    pub bytecode: Option<BytecodeObject>,
    /// A "0x"-prefixed hex string of the unlinked runtime/deployed bytecode. If the contract is
    /// not deployable, this has the string "0x"
    pub deployed_bytecode: Option<BytecodeObject>,
    /// The bytecode's link references object as returned by solc. If the contract doesn't need to
    /// be linked, this value contains an empty object.
    #[serde(default)]
    pub link_references: BTreeMap<String, BTreeMap<String, Vec<Offsets>>>,
    /// The deployed bytecode's link references object as returned by solc. If the contract doesn't
    /// need to be linked, this value contains an empty object.
    #[serde(default)]
    pub deployed_link_references: BTreeMap<String, BTreeMap<String, Vec<Offsets>>>,
}

impl<'a> From<&'a HardhatArtifact> for CompactContractBytecodeCow<'a> {
    fn from(artifact: &'a HardhatArtifact) -> Self {
        let c: ContractBytecode = artifact.clone().into();
        CompactContractBytecodeCow {
            abi: Some(Cow::Borrowed(&artifact.abi)),
            bytecode: c.bytecode.map(|b| Cow::Owned(b.into())),
            deployed_bytecode: c.deployed_bytecode.map(|b| Cow::Owned(b.into())),
        }
    }
}

impl From<HardhatArtifact> for CompactContract {
    fn from(artifact: HardhatArtifact) -> Self {
        Self {
            abi: Some(artifact.abi),
            bin: artifact.bytecode,
            bin_runtime: artifact.deployed_bytecode,
        }
    }
}

impl From<HardhatArtifact> for ContractBytecode {
    fn from(artifact: HardhatArtifact) -> Self {
        let bytecode: Option<Bytecode> = artifact.bytecode.as_ref().map(|t| {
            let mut bcode: Bytecode = t.clone().into();
            bcode.link_references = artifact.link_references.clone();
            bcode
        });

        let deployed_bytecode: Option<DeployedBytecode> = artifact.bytecode.as_ref().map(|t| {
            let mut bcode: Bytecode = t.clone().into();
            bcode.link_references = artifact.deployed_link_references.clone();
            bcode.into()
        });

        Self { abi: Some(artifact.abi), bytecode, deployed_bytecode }
    }
}

impl From<HardhatArtifact> for CompactContractBytecode {
    fn from(artifact: HardhatArtifact) -> Self {
        let c: ContractBytecode = artifact.into();

        c.into()
    }
}
