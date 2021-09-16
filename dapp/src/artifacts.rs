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
