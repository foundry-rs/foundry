use ethers::core::utils::{solc::Contract, CompiledContract};
use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DapptoolsArtifact {
    contracts: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl DapptoolsArtifact {
    /// Convenience function to read from a file
    pub fn read(file: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(file.as_ref()).wrap_err_with(|| {
            format!("Failed to open artifacts file `{}`", file.as_ref().display())
        })?;
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
