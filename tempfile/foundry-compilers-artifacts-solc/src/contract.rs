//! Contract related types.

use crate::{
    bytecode::{
        Bytecode, BytecodeObject, CompactBytecode, CompactDeployedBytecode, DeployedBytecode,
    },
    serde_helpers, DevDoc, Evm, Ewasm, LosslessMetadata, Offsets, StorageLayout, UserDoc,
};
use alloy_json_abi::JsonAbi;
use alloy_primitives::Bytes;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap};

/// Represents a compiled solidity contract
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contract {
    /// The Ethereum Contract Metadata.
    /// See <https://docs.soliditylang.org/en/develop/metadata.html>
    pub abi: Option<JsonAbi>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_helpers::json_string_opt"
    )]
    pub metadata: Option<LosslessMetadata>,
    #[serde(default)]
    pub userdoc: UserDoc,
    #[serde(default)]
    pub devdoc: DevDoc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<String>,
    #[serde(default, skip_serializing_if = "StorageLayout::is_empty")]
    pub storage_layout: StorageLayout,
    #[serde(default, skip_serializing_if = "StorageLayout::is_empty")]
    pub transient_storage_layout: StorageLayout,
    /// EVM-related outputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evm: Option<Evm>,
    /// Ewasm related outputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ewasm: Option<Ewasm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_optimized: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_optimized_ast: Option<serde_json::Value>,
}

impl<'a> From<&'a Contract> for CompactContractBytecodeCow<'a> {
    fn from(artifact: &'a Contract) -> Self {
        let (bytecode, deployed_bytecode) = if let Some(evm) = &artifact.evm {
            (
                evm.bytecode.clone().map(Into::into).map(Cow::Owned),
                evm.deployed_bytecode.clone().map(Into::into).map(Cow::Owned),
            )
        } else {
            (None, None)
        };
        CompactContractBytecodeCow {
            abi: artifact.abi.as_ref().map(Cow::Borrowed),
            bytecode,
            deployed_bytecode,
        }
    }
}

/// Minimal representation of a contract with a present abi and bytecode.
///
/// Unlike `CompactContractSome` which contains the `BytecodeObject`, this holds the whole
/// `Bytecode` object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContractBytecode {
    /// The Ethereum Contract ABI. If empty, it is represented as an empty
    /// array. See <https://docs.soliditylang.org/en/develop/abi-spec.html>
    pub abi: Option<JsonAbi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytecode: Option<Bytecode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_bytecode: Option<DeployedBytecode>,
}

impl ContractBytecode {
    /// Unwraps `self` into `ContractBytecodeSome`.
    ///
    /// # Panics
    ///
    /// Panics if any field is `None`.
    #[track_caller]
    pub fn unwrap(self) -> ContractBytecodeSome {
        ContractBytecodeSome {
            abi: self.abi.unwrap(),
            bytecode: self.bytecode.unwrap(),
            deployed_bytecode: self.deployed_bytecode.unwrap(),
        }
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
}

impl From<Contract> for ContractBytecode {
    fn from(c: Contract) -> Self {
        let (bytecode, deployed_bytecode) = if let Some(evm) = c.evm {
            (evm.bytecode, evm.deployed_bytecode)
        } else {
            (None, None)
        };

        Self { abi: c.abi, bytecode, deployed_bytecode }
    }
}

/// Minimal representation of a contract with a present abi and bytecode.
///
/// Unlike `CompactContractSome` which contains the `BytecodeObject`, this holds the whole
/// `Bytecode` object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactContractBytecode {
    /// The Ethereum Contract ABI. If empty, it is represented as an empty
    /// array. See <https://docs.soliditylang.org/en/develop/abi-spec.html>
    pub abi: Option<JsonAbi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytecode: Option<CompactBytecode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_bytecode: Option<CompactDeployedBytecode>,
}

impl CompactContractBytecode {
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
}

impl<'a> From<&'a CompactContractBytecode> for CompactContractBytecodeCow<'a> {
    fn from(artifact: &'a CompactContractBytecode) -> Self {
        CompactContractBytecodeCow {
            abi: artifact.abi.as_ref().map(Cow::Borrowed),
            bytecode: artifact.bytecode.as_ref().map(Cow::Borrowed),
            deployed_bytecode: artifact.deployed_bytecode.as_ref().map(Cow::Borrowed),
        }
    }
}

impl From<Contract> for CompactContractBytecode {
    fn from(c: Contract) -> Self {
        let (bytecode, deployed_bytecode) = if let Some(evm) = c.evm {
            let evm = evm.into_compact();
            (evm.bytecode, evm.deployed_bytecode)
        } else {
            (None, None)
        };

        Self { abi: c.abi, bytecode, deployed_bytecode }
    }
}

impl From<ContractBytecode> for CompactContractBytecode {
    fn from(c: ContractBytecode) -> Self {
        let (maybe_bcode, maybe_runtime) = match (c.bytecode, c.deployed_bytecode) {
            (Some(bcode), Some(dbcode)) => (Some(bcode.into()), Some(dbcode.into())),
            (None, Some(dbcode)) => (None, Some(dbcode.into())),
            (Some(bcode), None) => (Some(bcode.into()), None),
            (None, None) => (None, None),
        };
        Self { abi: c.abi, bytecode: maybe_bcode, deployed_bytecode: maybe_runtime }
    }
}

impl From<CompactContractBytecode> for ContractBytecode {
    fn from(c: CompactContractBytecode) -> Self {
        let (maybe_bcode, maybe_runtime) = match (c.bytecode, c.deployed_bytecode) {
            (Some(bcode), Some(dbcode)) => (Some(bcode.into()), Some(dbcode.into())),
            (None, Some(dbcode)) => (None, Some(dbcode.into())),
            (Some(bcode), None) => (Some(bcode.into()), None),
            (None, None) => (None, None),
        };
        Self { abi: c.abi, bytecode: maybe_bcode, deployed_bytecode: maybe_runtime }
    }
}

/// A [CompactContractBytecode] that is either owns or borrows its content
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactContractBytecodeCow<'a> {
    pub abi: Option<Cow<'a, JsonAbi>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytecode: Option<Cow<'a, CompactBytecode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_bytecode: Option<Cow<'a, CompactDeployedBytecode>>,
}

impl From<CompactContractBytecodeCow<'_>> for CompactContract {
    fn from(value: CompactContractBytecodeCow<'_>) -> Self {
        Self {
            abi: value.abi.map(Cow::into_owned),
            bin: value.bytecode.map(|bytecode| match bytecode {
                Cow::Owned(bytecode) => bytecode.object,
                Cow::Borrowed(bytecode) => bytecode.object.clone(),
            }),
            bin_runtime: value
                .deployed_bytecode
                .and_then(|bytecode| match bytecode {
                    Cow::Owned(bytecode) => bytecode.bytecode,
                    Cow::Borrowed(bytecode) => bytecode.bytecode.clone(),
                })
                .map(|bytecode| bytecode.object),
        }
    }
}

impl From<CompactContractBytecodeCow<'_>> for CompactContractBytecode {
    fn from(value: CompactContractBytecodeCow<'_>) -> Self {
        Self {
            abi: value.abi.map(Cow::into_owned),
            bytecode: value.bytecode.map(Cow::into_owned),
            deployed_bytecode: value.deployed_bytecode.map(Cow::into_owned),
        }
    }
}

impl<'a> From<&'a CompactContractBytecodeCow<'_>> for CompactContractBytecodeCow<'a> {
    fn from(value: &'a CompactContractBytecodeCow<'_>) -> Self {
        Self {
            abi: value.abi.as_ref().map(|x| Cow::Borrowed(&**x)),
            bytecode: value.bytecode.as_ref().map(|x| Cow::Borrowed(&**x)),
            deployed_bytecode: value.deployed_bytecode.as_ref().map(|x| Cow::Borrowed(&**x)),
        }
    }
}

/// Minimal representation of a contract with a present abi and bytecode.
///
/// Unlike `CompactContractSome` which contains the `BytecodeObject`, this holds the whole
/// `Bytecode` object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContractBytecodeSome {
    pub abi: JsonAbi,
    pub bytecode: Bytecode,
    pub deployed_bytecode: DeployedBytecode,
}

impl TryFrom<ContractBytecode> for ContractBytecodeSome {
    type Error = ContractBytecode;

    fn try_from(value: ContractBytecode) -> Result<Self, Self::Error> {
        if value.abi.is_none() || value.bytecode.is_none() || value.deployed_bytecode.is_none() {
            return Err(value);
        }
        Ok(value.unwrap())
    }
}

/// Minimal representation of a contract's artifact with a present abi and bytecode.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactContractSome {
    /// The Ethereum Contract ABI. If empty, it is represented as an empty
    /// array. See <https://docs.soliditylang.org/en/develop/abi-spec.html>
    pub abi: JsonAbi,
    pub bin: BytecodeObject,
    #[serde(rename = "bin-runtime")]
    pub bin_runtime: BytecodeObject,
}

impl TryFrom<CompactContract> for CompactContractSome {
    type Error = CompactContract;

    fn try_from(value: CompactContract) -> Result<Self, Self::Error> {
        if value.abi.is_none() || value.bin.is_none() || value.bin_runtime.is_none() {
            return Err(value);
        }
        Ok(value.unwrap())
    }
}

/// The general purpose minimal representation of a contract's abi with bytecode
/// Unlike `CompactContractSome` all fields are optional so that every possible compiler output can
/// be represented by it
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactContract {
    /// The Ethereum Contract ABI. If empty, it is represented as an empty
    /// array. See <https://docs.soliditylang.org/en/develop/abi-spec.html>
    pub abi: Option<JsonAbi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<BytecodeObject>,
    #[serde(default, rename = "bin-runtime", skip_serializing_if = "Option::is_none")]
    pub bin_runtime: Option<BytecodeObject>,
}

impl CompactContract {
    /// Returns the contents of this type as a single tuple of abi, bytecode and deployed bytecode
    pub fn into_parts(self) -> (Option<JsonAbi>, Option<Bytes>, Option<Bytes>) {
        (
            self.abi,
            self.bin.and_then(|bin| bin.into_bytes()),
            self.bin_runtime.and_then(|bin| bin.into_bytes()),
        )
    }

    /// Returns the individual parts of this contract.
    ///
    /// If the values are `None`, then `Default` is returned.
    pub fn into_parts_or_default(self) -> (JsonAbi, Bytes, Bytes) {
        (
            self.abi.unwrap_or_default(),
            self.bin.and_then(|bin| bin.into_bytes()).unwrap_or_default(),
            self.bin_runtime.and_then(|bin| bin.into_bytes()).unwrap_or_default(),
        )
    }

    /// Unwraps `self` into `CompactContractSome`.
    ///
    /// # Panics
    ///
    /// Panics if any field is `None`.
    #[track_caller]
    pub fn unwrap(self) -> CompactContractSome {
        CompactContractSome {
            abi: self.abi.unwrap(),
            bin: self.bin.unwrap(),
            bin_runtime: self.bin_runtime.unwrap(),
        }
    }

    /// Returns the `CompactContractSome` if any if the field equals `None` the `Default` value is
    /// returned
    ///
    /// Unlike `unwrap`, this function does _not_ panic
    pub fn unwrap_or_default(self) -> CompactContractSome {
        CompactContractSome {
            abi: self.abi.unwrap_or_default(),
            bin: self.bin.unwrap_or_default(),
            bin_runtime: self.bin_runtime.unwrap_or_default(),
        }
    }
}

impl From<serde_json::Value> for CompactContract {
    fn from(mut val: serde_json::Value) -> Self {
        if let Some(map) = val.as_object_mut() {
            let abi = map.remove("abi").and_then(|val| serde_json::from_value(val).ok());
            let bin = map.remove("bin").and_then(|val| serde_json::from_value(val).ok());
            let bin_runtime =
                map.remove("bin-runtime").and_then(|val| serde_json::from_value(val).ok());
            Self { abi, bin, bin_runtime }
        } else {
            Self::default()
        }
    }
}

impl<'a> From<&'a serde_json::Value> for CompactContractBytecodeCow<'a> {
    fn from(artifact: &'a serde_json::Value) -> Self {
        let c = CompactContractBytecode::from(artifact.clone());
        CompactContractBytecodeCow {
            abi: c.abi.map(Cow::Owned),
            bytecode: c.bytecode.map(Cow::Owned),
            deployed_bytecode: c.deployed_bytecode.map(Cow::Owned),
        }
    }
}

impl From<serde_json::Value> for CompactContractBytecode {
    fn from(val: serde_json::Value) -> Self {
        serde_json::from_value(val).unwrap_or_default()
    }
}

impl From<ContractBytecode> for CompactContract {
    fn from(c: ContractBytecode) -> Self {
        let ContractBytecode { abi, bytecode, deployed_bytecode } = c;
        Self {
            abi,
            bin: bytecode.map(|c| c.object),
            bin_runtime: deployed_bytecode
                .and_then(|deployed| deployed.bytecode.map(|code| code.object)),
        }
    }
}

impl From<CompactContractBytecode> for CompactContract {
    fn from(c: CompactContractBytecode) -> Self {
        let c: ContractBytecode = c.into();
        c.into()
    }
}

impl From<ContractBytecodeSome> for CompactContract {
    fn from(c: ContractBytecodeSome) -> Self {
        Self {
            abi: Some(c.abi),
            bin: Some(c.bytecode.object),
            bin_runtime: c.deployed_bytecode.bytecode.map(|code| code.object),
        }
    }
}

impl From<Contract> for CompactContract {
    fn from(c: Contract) -> Self {
        ContractBytecode::from(c).into()
    }
}

impl From<CompactContractSome> for CompactContract {
    fn from(c: CompactContractSome) -> Self {
        Self { abi: Some(c.abi), bin: Some(c.bin), bin_runtime: Some(c.bin_runtime) }
    }
}

impl<'a> From<CompactContractRef<'a>> for CompactContract {
    fn from(c: CompactContractRef<'a>) -> Self {
        Self { abi: c.abi.cloned(), bin: c.bin.cloned(), bin_runtime: c.bin_runtime.cloned() }
    }
}

impl<'a> From<CompactContractRefSome<'a>> for CompactContract {
    fn from(c: CompactContractRefSome<'a>) -> Self {
        Self {
            abi: Some(c.abi.clone()),
            bin: Some(c.bin.clone()),
            bin_runtime: Some(c.bin_runtime.clone()),
        }
    }
}

/// Minimal representation of a contract with a present abi and bytecode that borrows.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CompactContractRefSome<'a> {
    pub abi: &'a JsonAbi,
    pub bin: &'a BytecodeObject,
    #[serde(rename = "bin-runtime")]
    pub bin_runtime: &'a BytecodeObject,
}

impl CompactContractRefSome<'_> {
    /// Returns the individual parts of this contract.
    ///
    /// If the values are `None`, then `Default` is returned.
    pub fn into_parts(self) -> (JsonAbi, Bytes, Bytes) {
        CompactContract::from(self).into_parts_or_default()
    }
}

impl<'a> TryFrom<CompactContractRef<'a>> for CompactContractRefSome<'a> {
    type Error = CompactContractRef<'a>;

    fn try_from(value: CompactContractRef<'a>) -> Result<Self, Self::Error> {
        if value.abi.is_none() || value.bin.is_none() || value.bin_runtime.is_none() {
            return Err(value);
        }
        Ok(value.unwrap())
    }
}

/// Helper type to serialize while borrowing from `Contract`
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CompactContractRef<'a> {
    pub abi: Option<&'a JsonAbi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<&'a BytecodeObject>,
    #[serde(default, rename = "bin-runtime", skip_serializing_if = "Option::is_none")]
    pub bin_runtime: Option<&'a BytecodeObject>,
}

impl<'a> CompactContractRef<'a> {
    /// Clones the referenced values and returns as tuples
    pub fn into_parts(self) -> (Option<JsonAbi>, Option<Bytes>, Option<Bytes>) {
        CompactContract::from(self).into_parts()
    }

    /// Returns the individual parts of this contract.
    ///
    /// If the values are `None`, then `Default` is returned.
    pub fn into_parts_or_default(self) -> (JsonAbi, Bytes, Bytes) {
        CompactContract::from(self).into_parts_or_default()
    }

    pub fn bytecode(&self) -> Option<&Bytes> {
        self.bin.as_ref().and_then(|bin| bin.as_bytes())
    }

    pub fn runtime_bytecode(&self) -> Option<&Bytes> {
        self.bin_runtime.as_ref().and_then(|bin| bin.as_bytes())
    }

    /// Unwraps `self` into `CompactContractRefSome`.
    ///
    /// # Panics
    ///
    /// Panics if any field is `None`.
    #[track_caller]
    pub fn unwrap(self) -> CompactContractRefSome<'a> {
        CompactContractRefSome {
            abi: self.abi.unwrap(),
            bin: self.bin.unwrap(),
            bin_runtime: self.bin_runtime.unwrap(),
        }
    }
}

impl<'a> From<&'a Contract> for CompactContractRef<'a> {
    fn from(c: &'a Contract) -> Self {
        let (bin, bin_runtime) = if let Some(evm) = &c.evm {
            (
                evm.bytecode.as_ref().map(|c| &c.object),
                evm.deployed_bytecode
                    .as_ref()
                    .and_then(|deployed| deployed.bytecode.as_ref().map(|evm| &evm.object)),
            )
        } else {
            (None, None)
        };

        Self { abi: c.abi.as_ref(), bin, bin_runtime }
    }
}
