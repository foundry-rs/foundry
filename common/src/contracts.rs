//! commonly used contract types and functions

use ethers_core::{
    abi::{Abi, Event, Function},
    types::{Address, H256},
    utils::hex,
};
use ethers_solc::{artifacts::ContractBytecodeSome, ArtifactId, ProjectPathsConfig};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

type ArtifactWithContractRef<'a> = (&'a ArtifactId, &'a (Abi, Vec<u8>));

/// Wrapper type that maps an artifact to a contract ABI and bytecode.
#[derive(Default, Clone)]
pub struct ContractsByArtifact(pub BTreeMap<ArtifactId, (Abi, Vec<u8>)>);

impl ContractsByArtifact {
    /// Finds a contract which has a similar bytecode as `code`.
    pub fn find_by_code(&self, code: &[u8]) -> Option<ArtifactWithContractRef> {
        self.iter().find(|(_, (_, known_code))| diff_score(known_code, code) < 0.1)
    }
    /// Finds a contract which has the same contract name or identifier as `id`. If more than one is
    /// found, return error.
    pub fn find_by_name_or_identifier(
        &self,
        id: &str,
    ) -> eyre::Result<Option<ArtifactWithContractRef>> {
        let contracts = self
            .iter()
            .filter(|(artifact, _)| artifact.name == id || artifact.identifier() == id)
            .collect::<Vec<_>>();

        if contracts.len() > 1 {
            eyre::bail!("{id} has more than one implementation.");
        }

        Ok(contracts.first().cloned())
    }

    /// Flattens a group of contracts into maps of all events and functions
    pub fn flatten(&self) -> (BTreeMap<[u8; 4], Function>, BTreeMap<H256, Event>, Abi) {
        let flattened_funcs: BTreeMap<[u8; 4], Function> = self
            .iter()
            .flat_map(|(_name, (abi, _code))| {
                abi.functions()
                    .map(|func| (func.short_signature(), func.clone()))
                    .collect::<BTreeMap<[u8; 4], Function>>()
            })
            .collect();

        let flattened_events: BTreeMap<H256, Event> = self
            .iter()
            .flat_map(|(_name, (abi, _code))| {
                abi.events()
                    .map(|event| (event.signature(), event.clone()))
                    .collect::<BTreeMap<H256, Event>>()
            })
            .collect();

        // We need this for better revert decoding, and want it in abi form
        let mut errors_abi = Abi::default();
        self.iter().for_each(|(_name, (abi, _code))| {
            abi.errors().for_each(|error| {
                let entry =
                    errors_abi.errors.entry(error.name.clone()).or_insert_with(Default::default);
                entry.push(error.clone());
            });
        });
        (flattened_funcs, flattened_events, errors_abi)
    }
}

impl Deref for ContractsByArtifact {
    type Target = BTreeMap<ArtifactId, (Abi, Vec<u8>)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ContractsByArtifact {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Wrapper type that maps an address to a contract identifier and contract ABI.
pub type ContractsByAddress = BTreeMap<Address, (String, Abi)>;

/// Very simple fuzzy matching of contract bytecode.
///
/// Will fail for small contracts that are essentially all immutable variables.
pub fn diff_score(a: &[u8], b: &[u8]) -> f64 {
    let cutoff_len = usize::min(a.len(), b.len());
    if cutoff_len == 0 {
        return 1.0
    }

    let a = &a[..cutoff_len];
    let b = &b[..cutoff_len];
    let mut diff_chars = 0;
    for i in 0..cutoff_len {
        if a[i] != b[i] {
            diff_chars += 1;
        }
    }
    diff_chars as f64 / cutoff_len as f64
}

/// Flattens the contracts into  (`id` -> (`Abi`, `Vec<u8>`)) pairs
pub fn flatten_contracts(
    contracts: &BTreeMap<ArtifactId, ContractBytecodeSome>,
    deployed_code: bool,
) -> ContractsByArtifact {
    ContractsByArtifact(
        contracts
            .iter()
            .filter_map(|(id, c)| {
                let bytecode = if deployed_code {
                    c.deployed_bytecode.clone().into_bytes()
                } else {
                    c.bytecode.clone().object.into_bytes()
                };

                if let Some(bytecode) = bytecode {
                    return Some((id.clone(), (c.abi.clone(), bytecode.to_vec())))
                }
                None
            })
            .collect(),
    )
}

/// Artifact/Contract identifier can take the following form:
/// `<artifact file name>:<contract name>`, the `artifact file name` is the name of the json file of
/// the contract's artifact and the contract name is the name of the solidity contract, like
/// `SafeTransferLibTest.json:SafeTransferLibTest`
///
/// This returns the `contract name` part
///
/// # Example
///
/// ```
/// use foundry_common::*;
/// assert_eq!(
///     "SafeTransferLibTest",
///     get_contract_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_contract_name(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// This returns the `file name` part, See [`get_contract_name`]
///
/// # Example
///
/// ```
/// use foundry_common::*;
/// assert_eq!(
///     "SafeTransferLibTest.json",
///     get_file_name("SafeTransferLibTest.json:SafeTransferLibTest")
/// );
/// ```
pub fn get_file_name(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

/// Returns the path to the json artifact depending on the input
pub fn get_artifact_path(paths: &ProjectPathsConfig, path: &str) -> PathBuf {
    if path.ends_with(".json") {
        PathBuf::from(path)
    } else {
        let parts: Vec<&str> = path.split(':').collect();
        let file = parts[0];
        let contract_name =
            if parts.len() == 1 { parts[0].replace(".sol", "") } else { parts[1].to_string() };
        paths.artifacts.join(format!("{file}/{contract_name}.json"))
    }
}

/// Given the transaction data tries to identify the constructor arguments
/// The constructor data is encoded as: Constructor Code + Contract Code +  Constructor arguments
/// decoding the arguments here with only the transaction data is not trivial here, we try to find
/// the beginning of the constructor arguments by finding the length of the code, which is PUSH op
/// code which holds the code size and the code starts after the invalid op code (0xfe)
///
/// finding the `0xfe` (invalid opcode) in the data which should mark the beginning of constructor
/// arguments
pub fn find_constructor_args(data: &[u8]) -> Option<&[u8]> {
    // ref <https://ethereum.stackexchange.com/questions/126785/how-do-you-identify-the-start-of-constructor-arguments-for-contract-creation-cod>
    static CONSTRUCTOR_CODE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?m)(?:5b)?(?:60([a-z0-9]{2})|61([a-z0-9_]{4})|62([a-z0-9_]{6}))80(?:60([a-z0-9]{2})|61([a-z0-9_]{4})|62([a-z0-9_]{6}))(6000396000f3fe)").unwrap()
    });
    let s = hex::encode(data);

    // we're only interested in the last occurrence which skips additional CREATE inside the
    // constructor itself
    let caps = CONSTRUCTOR_CODE_RE.captures_iter(&s).last()?;

    let contract_len = u64::from_str_radix(
        caps.get(1).or_else(|| caps.get(2)).or_else(|| caps.get(3))?.as_str(),
        16,
    )
    .unwrap();

    // the end position of the constructor code, we use this instead of the contract offset , since
    // there could be multiple CREATE inside the data we need to divide by 2 for hex conversion
    let constructor_end = (caps.get(7)?.end() / 2) as u64;
    let start = (contract_len + constructor_end) as usize;
    let args = &data[start..];

    Some(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers_core::{abi, abi::ParamType};

    // <https://github.com/foundry-rs/foundry/issues/3053>
    #[test]
    fn test_find_constructor_args() {
        let code = "6080604052348015600f57600080fd5b50604051610121380380610121833981016040819052602c91606e565b600080546001600160a01b0319166001600160a01b0396909616959095179094556001929092556002556003556004805460ff191691151591909117905560d4565b600080600080600060a08688031215608557600080fd5b85516001600160a01b0381168114609b57600080fd5b809550506020860151935060408601519250606086015191506080860151801515811460c657600080fd5b809150509295509295909350565b603f806100e26000396000f3fe6080604052600080fdfea264697066735822122089f2c61beace50d105ec1b6a56a1204301b5595e850e7576f6f3aa8e76f12d0b64736f6c6343000810003300000000000000000000000000a329c0648769a73afac7f9381e08fb43dbea720000000000000000000000000000000000000000000000000000000100000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf60000000000000000000000000000000000000000000000000000000000000001";

        let code = hex::decode(code).unwrap();

        let args = find_constructor_args(&code).unwrap();

        let params = vec![
            ParamType::Address,
            ParamType::Uint(256),
            ParamType::Int(256),
            ParamType::FixedBytes(32),
            ParamType::Bool,
        ];

        let _decoded = abi::decode(&params, args).unwrap();
    }

    #[test]
    fn test_find_constructor_args_nested_deploy() {
        let code = "608060405234801561001057600080fd5b5060405161066d38038061066d83398101604081905261002f9161014a565b868686868686866040516100429061007c565b610052979695949392919061022f565b604051809103906000f08015801561006e573d6000803e3d6000fd5b50505050505050505061028a565b610396806102d783390190565b634e487b7160e01b600052604160045260246000fd5b60005b838110156100ba5781810151838201526020016100a2565b50506000910152565b600082601f8301126100d457600080fd5b81516001600160401b03808211156100ee576100ee610089565b604051601f8301601f19908116603f0116810190828211818310171561011657610116610089565b8160405283815286602085880101111561012f57600080fd5b61014084602083016020890161009f565b9695505050505050565b600080600080600080600060e0888a03121561016557600080fd5b87516001600160a01b038116811461017c57600080fd5b80975050602088015195506040880151945060608801519350608088015180151581146101a857600080fd5b60a08901519093506001600160401b03808211156101c557600080fd5b6101d18b838c016100c3565b935060c08a01519150808211156101e757600080fd5b506101f48a828b016100c3565b91505092959891949750929550565b6000815180845261021b81602086016020860161009f565b601f01601f19169290920160200192915050565b60018060a01b0388168152866020820152856040820152846060820152831515608082015260e060a0820152600061026a60e0830185610203565b82810360c084015261027c8185610203565b9a9950505050505050505050565b603f806102986000396000f3fe6080604052600080fdfea264697066735822122072aeef1567521008007b956bd7c6e9101a9b49fbce1f45210fa929c79d28bd9364736f6c63430008110033608060405234801561001057600080fd5b5060405161039638038061039683398101604081905261002f91610148565b600080546001600160a01b0319166001600160a01b0389161790556001869055600285905560038490556004805460ff19168415151790556005610073838261028a565b506006610080828261028a565b5050505050505050610349565b634e487b7160e01b600052604160045260246000fd5b600082601f8301126100b457600080fd5b81516001600160401b03808211156100ce576100ce61008d565b604051601f8301601f19908116603f011681019082821181831017156100f6576100f661008d565b8160405283815260209250868385880101111561011257600080fd5b600091505b838210156101345785820183015181830184015290820190610117565b600093810190920192909252949350505050565b600080600080600080600060e0888a03121561016357600080fd5b87516001600160a01b038116811461017a57600080fd5b80975050602088015195506040880151945060608801519350608088015180151581146101a657600080fd5b60a08901519093506001600160401b03808211156101c357600080fd5b6101cf8b838c016100a3565b935060c08a01519150808211156101e557600080fd5b506101f28a828b016100a3565b91505092959891949750929550565b600181811c9082168061021557607f821691505b60208210810361023557634e487b7160e01b600052602260045260246000fd5b50919050565b601f82111561028557600081815260208120601f850160051c810160208610156102625750805b601f850160051c820191505b818110156102815782815560010161026e565b5050505b505050565b81516001600160401b038111156102a3576102a361008d565b6102b7816102b18454610201565b8461023b565b602080601f8311600181146102ec57600084156102d45750858301515b600019600386901b1c1916600185901b178555610281565b600085815260208120601f198616915b8281101561031b578886015182559484019460019091019084016102fc565b50858210156103395787850151600019600388901b60f8161c191681555b5050505050600190811b01905550565b603f806103576000396000f3fe6080604052600080fdfea2646970667358221220a468ac913d3ecf191b6559ae7dca58e05ba048434318f393b86640b25cbbf1ed64736f6c6343000811003300000000000000000000000000a329c0648769a73afac7f9381e08fb43dbea720000000000000000000000000000000000000000000000000000000100000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffff00000000b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000066162636465660000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000568656c6c6f000000000000000000000000000000000000000000000000000000";

        let code = hex::decode(code).unwrap();

        let args = find_constructor_args(&code).unwrap();

        let params = vec![
            ParamType::Address,
            ParamType::Uint(256),
            ParamType::Int(256),
            ParamType::FixedBytes(32),
            ParamType::Bool,
            ParamType::Bytes,
            ParamType::String,
        ];

        let _decoded = abi::decode(&params, args).unwrap();
    }
}
