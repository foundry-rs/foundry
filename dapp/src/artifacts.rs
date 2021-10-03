use ethers::core::{types::Bytes, utils::CompiledContract};
use eyre::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize, Serializer};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DapptoolsArtifact {
    contracts: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub abi: ethers::abi::Abi,
    pub evm: Evm,
    #[serde(
        deserialize_with = "de_from_json_opt",
        serialize_with = "ser_to_inner_json",
        skip_serializing_if = "Option::is_none"
    )]
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evm {
    pub bytecode: Bytecode,
    pub deployed_bytecode: Bytecode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bytecode {
    #[serde(deserialize_with = "deserialize_bytes")]
    pub object: Bytes,
}

use serde::Deserializer;

pub fn deserialize_bytes<'de, D>(d: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(d)?;

    Ok(hex::decode(&value).map_err(|e| serde::de::Error::custom(e.to_string()))?.into())
}

impl DapptoolsArtifact {
    /// Convenience function to read from a file
    pub fn read(file: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(file)?;
        Ok(serde_json::from_reader::<_, _>(file)?)
    }

    /// Returns all the contract from the artifacts
    pub fn into_contracts(self) -> Result<HashMap<String, CompiledContract>> {
        let mut map = HashMap::with_capacity(self.contracts.len());
        for (key, value) in self.contracts {
            for (contract, data) in value {
                let data: Contract = serde_json::from_value(data)?;
                let data = CompiledContract {
                    abi: data.abi,
                    bytecode: data.evm.bytecode.object,
                    runtime_bytecode: data.evm.deployed_bytecode.object,
                };
                map.insert(format!("{}:{}", key, contract), data);
            }
        }

        Ok(map)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub compiler: Compiler,
    pub language: String,
    pub output: Output,
    pub settings: Settings,
    pub sources: Sources,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compiler {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub abi: Vec<Abi>,
    pub devdoc: Option<Doc>,
    pub userdoc: Option<Doc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Abi {
    pub inputs: Vec<Item>,
    #[serde(rename = "stateMutability")]
    pub state_mutability: Option<String>,
    #[serde(rename = "type")]
    pub abi_type: String,
    pub name: Option<String>,
    pub outputs: Option<Vec<Item>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    #[serde(rename = "internalType")]
    pub internal_type: String,
    pub name: String,
    #[serde(rename = "type")]
    pub put_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Doc {
    pub kind: String,
    pub methods: Libraries,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Libraries {
    #[serde(flatten)]
    pub libs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(rename = "compilationTarget")]
    pub compilation_target: CompilationTarget,
    #[serde(rename = "evmVersion")]
    pub evm_version: String,
    pub libraries: Libraries,
    pub metadata: MetadataClass,
    pub optimizer: Optimizer,
    pub remappings: Vec<Option<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationTarget {
    #[serde(flatten)]
    pub inner: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataClass {
    #[serde(rename = "bytecodeHash")]
    pub bytecode_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Optimizer {
    pub enabled: bool,
    pub runs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sources {
    #[serde(flatten)]
    pub inner: HashMap<String, serde_json::Value>,
}

fn de_from_json_opt<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    if let Some(val) = <Option<String>>::deserialize(deserializer)? {
        serde_json::from_str(&val).map_err(serde::de::Error::custom)
    } else {
        Ok(None)
    }
}

fn ser_to_inner_json<S, T>(val: &T, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let val = serde_json::to_string(val).map_err(serde::ser::Error::custom)?;
    s.serialize_str(&val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dapptools_artifact() {
        let path = std::fs::canonicalize("testdata/dapp-artifact.json").unwrap();
        let data = DapptoolsArtifact::read(path).unwrap();
        let contracts = data.into_contracts().unwrap();
        let mut expected = [
            "src/test/Greeter.t.sol:Greet",
            "lib/ds-test/src/test.sol:DSTest",
            "src/test/utils/Hevm.sol:Hevm",
            "src/test/Greeter.t.sol:Gm",
            "src/test/utils/GreeterTest.sol:User",
            "src/test/utils/GreeterTest.sol:GreeterTest",
            "lib/openzeppelin-contracts/contracts/access/Ownable.sol:Ownable",
            "lib/openzeppelin-contracts/contracts/utils/Context.sol:Context",
            "src/Greeter.sol:Greeter",
            "src/Greeter.sol:Errors",
        ]
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>();
        expected.sort_by_key(|name| name.to_lowercase());

        let mut got = contracts.keys().cloned().collect::<Vec<_>>();
        got.sort_by_key(|name| name.to_lowercase());
        assert_eq!(expected, got);
    }
}
