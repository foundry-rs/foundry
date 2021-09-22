use ethers::core::{types::Bytes, utils::CompiledContract};
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DapptoolsArtifact {
    contracts: HashMap<String, HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Contract {
    abi: ethers::abi::Abi,
    evm: Evm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Evm {
    bytecode: Bytecode,
    deployed_bytecode: Bytecode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bytecode {
    #[serde(deserialize_with = "deserialize_bytes")]
    object: Bytes,
}

use serde::Deserializer;

pub fn deserialize_bytes<'de, D>(d: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(d)?;

    Ok(hex::decode(&value)
        .map_err(|e| serde::de::Error::custom(e.to_string()))?
        .into())
}

impl DapptoolsArtifact {
    pub fn contracts(&self) -> Result<HashMap<String, CompiledContract>> {
        let mut map = HashMap::new();
        for (key, value) in &self.contracts {
            for (contract, data) in value.iter() {
                let data: Contract = serde_json::from_value(data.clone())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dapptools_artifact() {
        let path = std::fs::canonicalize("testdata/dapp-artifact.json").unwrap();
        let file = std::fs::File::open(path).unwrap();
        let data = serde_json::from_reader::<_, DapptoolsArtifact>(file).unwrap();
        let contracts = data.contracts().unwrap();
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
