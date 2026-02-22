use super::{Preprocessor, PreprocessorId};
use crate::{Document, PreprocessorOutput};
use alloy_primitives::Address;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// [Deployments] preprocessor id.
pub const DEPLOYMENTS_ID: PreprocessorId = PreprocessorId("deployments");

/// The deployments preprocessor.
///
/// This preprocessor writes to [Document]'s context.
#[derive(Debug)]
pub struct Deployments {
    /// The project root.
    pub root: PathBuf,
    /// The deployments directory.
    pub deployments: Option<PathBuf>,
}

/// A contract deployment.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Deployment {
    /// The contract address
    pub address: Address,
    /// The network name
    pub network: Option<String>,
}

impl Preprocessor for Deployments {
    fn id(&self) -> PreprocessorId {
        DEPLOYMENTS_ID
    }

    fn preprocess(&self, documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error> {
        let deployments_dir =
            self.root.join(self.deployments.as_deref().unwrap_or(Path::new("deployments")));

        // Gather all networks from the deployments directory.
        let networks = fs::read_dir(&deployments_dir)?
            .map(|entry| {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|e| eyre::eyre!("failed to extract directory name: {e:?}"))
                } else {
                    eyre::bail!("not a directory: {}", path.display())
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        if networks.is_empty() {
            return Ok(documents);
        }

        // Iterate over all documents to find any deployments.
        for document in &documents {
            let mut deployments = Vec::default();

            let mut item_path_json = document.item_path.clone();
            item_path_json.set_extension("json");
            let deployment_file = PathBuf::from(
                item_path_json
                    .file_name()
                    .ok_or_else(|| eyre::eyre!("Failed to extract file name from item path"))?,
            );

            // Iterate over all networks and check if there is a deployment for the given contract.
            for network in &networks {
                // Determine the path of the deployment artifact relative to the root directory.
                let deployment_path = deployments_dir.join(network).join(&deployment_file);

                // If the deployment file for the given contract is found, add the deployment
                // address to the document context.
                let mut deployment: Deployment =
                    serde_json::from_str(&fs::read_to_string(deployment_path)?)?;
                deployment.network = Some(network.clone());
                deployments.push(deployment);
            }

            // If there are any deployments for the given contract, add them to the document
            // context.
            if !deployments.is_empty() {
                document.add_context(self.id(), PreprocessorOutput::Deployments(deployments));
            }
        }

        Ok(documents)
    }
}
