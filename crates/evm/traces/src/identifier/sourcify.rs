use super::{IdentifiedAddress, TraceIdentifier};
use alloy_json_abi::JsonAbi;
use revm_inspectors::tracing::types::CallTraceNode;
use serde::Deserialize;
use std::borrow::Cow;

#[derive(Deserialize)]
struct SourcifyFile {
    name: String,
    content: String,
}

/// A trace identifier that uses Sourcify to identify contract ABIs.
pub struct SourcifyIdentifier;

impl SourcifyIdentifier {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SourcifyIdentifier {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceIdentifier for SourcifyIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        let mut identities = Vec::new();
        let client = reqwest::Client::new();

        for &node in nodes {
            let address = node.trace.address;

            // Try to get ABI from Sourcify
            let abi = foundry_common::block_on(async {
                let url = format!("https://repo.sourcify.dev/contracts/full_match/1/{address:?}/");

                let files: Vec<SourcifyFile> =
                    client.get(&url).send().await.ok()?.json().await.ok()?;

                let metadata_file = files.into_iter().find(|file| file.name == "metadata.json")?;
                let metadata: serde_json::Value =
                    serde_json::from_str(&metadata_file.content).ok()?;
                let abi_value = metadata.get("output")?.get("abi")?;
                serde_json::from_value::<JsonAbi>(abi_value.clone()).ok()
            });

            if let Some(abi) = abi {
                identities.push(IdentifiedAddress {
                    address,
                    label: Some("Sourcify".to_string()),
                    contract: Some("Sourcify".to_string()),
                    abi: Some(Cow::Owned(abi)),
                    artifact_id: None,
                });
            }
        }

        identities
    }
}
