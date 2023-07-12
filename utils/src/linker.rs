use ethers_core::types::{Address, Bytes, U256};
use ethers_solc::{
    artifacts::{
        BytecodeObject, CompactBytecode, CompactContractBytecode, CompactDeployedBytecode,
        Libraries,
    },
    contracts::ArtifactContracts,
    ArtifactId,
};
use std::{collections::HashMap, fmt::Formatter, str::FromStr};
use thiserror::Error;

#[derive(Debug)]
pub struct LinkedArtifact {
    pub id: ArtifactId,
    pub bytecode: CompactBytecode,
    pub deployed_bytecode: CompactDeployedBytecode,
    // does not include addresses specified by user
    // this is flattened
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug)]
pub struct ResolvedDependency {
    /// The address the linker resolved
    pub address: Address,
    /// The nonce used to resolve the dependency
    pub nonce: U256,
    pub id: ArtifactId,
    pub bytecode: Bytes,
    linker_symbol: String,
}

impl ResolvedDependency {
    // returns {file}:{lib}:{addr}
    pub fn library_line(&self) -> String {
        format!("{}:{:?}", self.linker_symbol, self.address)
    }
}

impl std::fmt::Display for ResolvedDependency {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @ {} (resolved with nonce {})", self.id.slug(), self.address, self.nonce)
    }
}

struct DependencyWithSymbol<'a> {
    id: &'a ArtifactId,
    contract: &'a CompactContractBytecode,
    // the source file path used in the linker symbol
    link_source_path: String,
}

#[derive(Default)]
struct DependencyTree<'a>(HashMap<ArtifactId, Vec<DependencyWithSymbol<'a>>>);

impl<'a> DependencyTree<'a> {
    fn new(contracts: &'a ArtifactContracts) -> Result<Self, LinkerError> {
        let mut map = HashMap::new();

        // todo: is there a more readable way to construct this?
        for (artifact_id, contract) in contracts.iter() {
            let dependencies: Vec<DependencyWithSymbol> = contract
                .all_link_references()
                .iter()
                .flat_map(|(link_source_path, links)| {
                    links.keys().map(|name| {
                        let dependency = contracts
                            .iter()
                            .find(|(dep_id, _)| dep_id.slug() == format!("{name}.json:{name}"));

                        if let Some((dep_id, contract)) = dependency {
                            Ok(DependencyWithSymbol {
                                id: dep_id,
                                contract,
                                link_source_path: link_source_path.clone(),
                            })
                        } else {
                            Err(LinkerError::UnresolvedDependency {
                                dependent: artifact_id.clone(),
                                slug: format!("{name}.json:{name}"),
                            })
                        }
                    })
                })
                .collect::<Result<_, LinkerError>>()?;

            map.insert(artifact_id.clone(), dependencies);
        }

        Ok(Self(map))
    }
}

#[derive(Debug, Error)]
pub enum LinkerError {
    #[error("artifact is unknown to linker")]
    UnknownArtifact,
    #[error("the linker could not resolve dependency {slug} for {}", dependent.name)]
    UnresolvedDependency {
        dependent: ArtifactId,
        // note: dep slug
        slug: String,
    },
    #[error("the artifact has no bytecode")]
    AbstractArtifact,
}

pub fn link_all(
    contracts: &ArtifactContracts,
    library_addresses: &Libraries,
    sender: Address,
    nonce: U256,
) -> Result<HashMap<ArtifactId, LinkedArtifact>, LinkerError> {
    let mut result = HashMap::new();
    for artifact_id in contracts.keys().cloned() {
        let linked_artifact =
            match link_single(contracts, library_addresses, sender, nonce, artifact_id.clone()) {
                // This is ok for us, we just skip these
                Err(LinkerError::AbstractArtifact) => continue,
                Err(err) => return Err(err),
                Ok(artifact) => artifact,
            };
        result.insert(artifact_id, linked_artifact);
    }
    Ok(result)
}

/// Links the given artifact.
///
/// The linker placeholders in the given artifact's bytecode are resolved by first checking if the
/// address is in `library_addresses`, which is a list addresses for known   pre-deployed libraries
/// supplied by the user.
///
/// If the library address is not known beforehand, the address is resolved using the `sender`
/// address and the current `nonce`.
///
/// Artifacts are recursively linked, i.e. if one of the artifact's dependencies is unlinked, it is
/// linked first. This may increase the nonce.
///
/// The return type contains the linked bytecode of the artifact, as well as the linked
/// representation of every dependency of the contract. These **must** be deployed in the order they
/// appear in the return type.
pub fn link_single(
    contracts: &ArtifactContracts,
    library_addresses: &Libraries,
    sender: Address,
    mut nonce: U256,
    target: ArtifactId,
) -> Result<LinkedArtifact, LinkerError> {
    let contract = contracts.get(&target).ok_or(LinkerError::UnknownArtifact)?;
    let mut dependency_tree = DependencyTree::new(contracts)?;

    match &contract.bytecode {
        Some(bytecode) => match &bytecode.object {
            BytecodeObject::Unlinked(_) => {
                let (bytecode, deployed_bytecode, dependencies) = recurse_link(
                    &mut dependency_tree,
                    library_addresses,
                    &mut HashMap::new(),
                    &target,
                    contract,
                    sender,
                    &mut nonce,
                );
                Ok(LinkedArtifact { id: target, bytecode, deployed_bytecode, dependencies })
            }
            BytecodeObject::Bytecode(bytes) if bytes.is_empty() => {
                Err(LinkerError::AbstractArtifact)
            }
            BytecodeObject::Bytecode(_) => Ok(LinkedArtifact {
                id: target,
                bytecode: contract.bytecode.clone().unwrap(),
                deployed_bytecode: contract.deployed_bytecode.clone().unwrap(),
                dependencies: Vec::new(),
            }),
        },
        None => Err(LinkerError::AbstractArtifact),
    }
}

fn recurse_link(
    dependency_tree: &mut DependencyTree,
    library_addresses: &Libraries,
    resolution_cache: &mut HashMap<ArtifactId, Address>,
    target_id: &ArtifactId,
    target: &CompactContractBytecode,
    sender: Address,
    nonce: &mut U256,
) -> (CompactBytecode, CompactDeployedBytecode, Vec<ResolvedDependency>) {
    let Some(dependencies) = dependency_tree.0.remove(target_id) else {
        // Safety: We've already checked that the bytecode is `Some` before calling `recurse_link`
        return (
            target.bytecode.clone().unwrap(),
            target.deployed_bytecode.clone().unwrap(),
            Vec::new(),
        )
    };

    // Safety: We've already checked that the bytecode is `Some` before calling `recurse_link`
    let mut bytecode = target.bytecode.clone().unwrap();
    let mut deployed_bytecode = target.deployed_bytecode.clone().unwrap();

    let mut resolved_dependencies = Vec::with_capacity(dependencies.len());
    for dep in dependencies {
        let dep_id = dep.id;

        // The address for the library was specified in advance, so we don't even care if it has
        // dependencies we need to link
        if let Some(address) =
            library_addresses.libs.get(&dep_id.path).and_then(|f| f.get(&dep_id.name))
        {
            let address = Address::from_str(address).unwrap();
            bytecode.link(&dep.link_source_path, &dep_id.name, address);
            if let Some(inner) = deployed_bytecode.bytecode.as_mut() {
                inner.link(&dep.link_source_path, &dep_id.name, address);
            }
            continue
        }

        // todo: dedup .link calls?
        // todo: docs
        if let Some(address) = resolution_cache.get(dep_id) {
            bytecode.link(&dep.link_source_path, &dep_id.name, *address);
            if let Some(inner) = deployed_bytecode.bytecode.as_mut() {
                inner.link(&dep.link_source_path, &dep_id.name, *address);
            }
            continue
        }

        // Check if the dependency has dependencies of its own
        let dep_bytecode = if dependency_tree.0.contains_key(dep_id) {
            let (dep_bytecode, _, linked_deps_of_dep) = recurse_link(
                dependency_tree,
                library_addresses,
                resolution_cache,
                dep_id,
                dep.contract,
                sender,
                nonce,
            );
            resolved_dependencies.extend(linked_deps_of_dep);
            dep_bytecode
        } else {
            dep.contract.bytecode.clone().unwrap()
        };

        // Compute address the dependency will live at
        let nonce_used = *nonce;
        let address = ethers_core::utils::get_contract_address(sender, *nonce);
        *nonce += 1.into();

        // Link the dependency
        bytecode.link(&dep.link_source_path, &dep_id.name, address);
        if let Some(inner) = deployed_bytecode.bytecode.as_mut() {
            inner.link(&dep.link_source_path, &dep_id.name, address);
        }

        resolved_dependencies.push(ResolvedDependency {
            address,
            nonce: nonce_used,
            id: dep_id.clone(),
            linker_symbol: format!("{}:{}", dep.link_source_path, dep_id.name),
            // todo: replace unwrap
            bytecode: dep_bytecode.object.into_bytes().unwrap(),
        });
        resolution_cache.insert(dep_id.clone(), address);
    }

    (bytecode, deployed_bytecode, resolved_dependencies)
}

// test: link w/ no deps (contract)
// test: simple link (contract -> a)
// test: unknown artifact
// test: unknown dependency
// test: abstract artifact
// test: src/Lib.sol:CoolLibrary (file not same name as lib)
// test: contract -> a -> b
// test: contract -> a & contract -> b
// test: contract -> a -> b & contract -> b
// test: specified library addresses
// test: evalir linker test
