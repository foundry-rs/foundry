use crate::ContractBytecodeSome;
use ethers_solc::{contracts::ArtifactContracts, ArtifactId, Graph, Project};
use std::{
    collections::{BTreeMap, HashMap},
    convert::From,
    fs,
    path::PathBuf,
};

/// Resolve the import tree of our target path, and get only the artifacts and
/// sources we need. If it's a standalone script, don't filter anything out.
#[allow(unused)]
pub fn filter_sources_and_artifacts(
    target: &str,
    sources: BTreeMap<ArtifactId, String>,
    highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    project: &Project,
) -> eyre::Result<(BTreeMap<ArtifactId, String>, HashMap<String, ContractBytecodeSome>)> {
    // Find all imports
    let graph = Graph::resolve(&project.paths)?;
    let target_path = project.root().join(target);
    let mut target_tree = BTreeMap::new();
    let mut is_standalone = false;

    if let Some(target_index) = graph.files().get(&target_path) {
        target_tree.extend(
            graph
                .all_imported_nodes(*target_index)
                .map(|index| graph.node(index).unpack())
                .collect::<BTreeMap<_, _>>(),
        );

        // Add our target into the tree as well.
        let (target_path, target_source) = graph.node(*target_index).unpack();
        target_tree.insert(target_path, target_source);
    } else {
        is_standalone = true;
    }

    let sources = sources
        .into_iter()
        .filter_map(|(id, path)| {
            let mut resolved = project
                .paths
                .resolve_library_import(project.root(), &PathBuf::from(&path))
                .unwrap_or_else(|| PathBuf::from(&path));

            if !resolved.is_absolute() {
                resolved = project.root().join(&resolved);
            }

            if !is_standalone {
                target_tree.get(&resolved).map(|source| (id, source.content.as_str().to_string()))
            } else {
                Some((
                    id,
                    fs::read_to_string(&resolved).unwrap_or_else(|_| {
                        panic!("Something went wrong reading the source file: {path:?}")
                    }),
                ))
            }
        })
        .collect();

    let artifacts = highlevel_known_contracts
        .into_iter()
        .filter_map(|(id, artifact)| {
            if !is_standalone {
                target_tree.get(&id.source).map(|_| (id.name, artifact))
            } else {
                Some((id.name, artifact))
            }
        })
        .collect();

    Ok((sources, artifacts))
}
