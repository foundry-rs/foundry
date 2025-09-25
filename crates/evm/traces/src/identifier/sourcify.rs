use super::{IdentifiedAddress, TraceIdentifier};
use alloy_json_abi::JsonAbi;
use revm_inspectors::tracing::types::CallTraceNode;
use serde::Deserialize;
use std::borrow::Cow;

#[derive(Deserialize)]
struct SourceifyFile {
    name: String,
    content: String,
}

/// A trace identifier that uses Sourcify to identify contract ABIs.
pub struct SourceifyIdentifier;

impl SourceifyIdentifier {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SourceifyIdentifier {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceIdentifier for SourceifyIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        let mut identities = Vec::new();

        for &node in nodes {
            let address = node.trace.address;

            // Try to get ABI from Sourcify
            let abi = foundry_common::block_on(async {
                let client = reqwest::Client::new();
                let url = format!("https://repo.sourcify.dev/contracts/full_match/1/{address:?}/");

                let files: Vec<SourceifyFile> =
                    client.get(&url).send().await.ok()?.json().await.ok()?;

                for file in files {
                    if file.name == "metadata.json" {
                        let metadata: serde_json::Value =
                            serde_json::from_str(&file.content).ok()?;
                        let abi_value = metadata.get("output")?.get("abi")?;
                        return serde_json::from_value::<JsonAbi>(abi_value.clone()).ok();
                    }
                }
                None
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
