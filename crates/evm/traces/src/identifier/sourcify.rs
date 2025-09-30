use super::{IdentifiedAddress, TraceIdentifier};
use alloy_json_abi::JsonAbi;
use foundry_config::Chain;
use revm_inspectors::tracing::types::CallTraceNode;
use std::borrow::Cow;

/// A trace identifier that uses Sourcify to identify contract ABIs.
pub struct SourcifyIdentifier {
    chain_id: u64,
}

impl SourcifyIdentifier {
    /// Creates a new Sourcify identifier for the given chain.
    pub fn new(chain: Option<Chain>) -> Self {
        let chain_id = chain.map(|c| c.id()).unwrap_or(1);
        Self { chain_id }
    }
}

impl Default for SourcifyIdentifier {
    fn default() -> Self {
        Self::new(None)
    }
}

impl TraceIdentifier for SourcifyIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        let mut identities = Vec::new();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client");

        for &node in nodes {
            let address = node.trace.address;

            // Try to get ABI from Sourcify using APIv2
            let abi = foundry_common::block_on(async {
                let url = format!(
                    "https://sourcify.dev/server/v2/contract/{}/{}?fields=abi",
                    self.chain_id, address
                );

                let response = client.get(&url).send().await.ok()?;
                let json: serde_json::Value = response.json().await.ok()?;
                let abi_value = json.get("abi")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::NamedChain;

    #[test]
    fn test_sourcify_identifier_creation() {
        let identifier = SourcifyIdentifier::new(None);
        assert_eq!(identifier.chain_id, 1); // Default to mainnet
    }

    #[test]
    fn test_sourcify_identifier_with_chain() {
        let identifier = SourcifyIdentifier::new(Some(NamedChain::Polygon.into()));
        assert_eq!(identifier.chain_id, 137); // Polygon chain ID
    }

    #[test]
    fn test_sourcify_identifier_default() {
        let identifier = SourcifyIdentifier::default();
        assert_eq!(identifier.chain_id, 1); // Default to mainnet
    }

    #[test]
    fn test_empty_nodes() {
        let mut identifier = SourcifyIdentifier::default();
        let nodes: Vec<&CallTraceNode> = vec![];
        let result = identifier.identify_addresses(&nodes);
        assert!(result.is_empty());
    }

    #[test]
    fn test_sourcify_apiv2_response_parsing() {
        // Test that we can parse the new APIv2 response format correctly
        let response_json = r#"{
            "abi": [
                {"name": "transfer", "type": "function", "inputs": [], "outputs": []}
            ],
            "matchId": "1532018",
            "creationMatch": "match",
            "runtimeMatch": "match",
            "verifiedAt": "2024-08-08T13:20:07Z",
            "match": "match",
            "chainId": "1",
            "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        }"#;

        let json: serde_json::Value = serde_json::from_str(response_json).unwrap();
        let abi_value = json.get("abi").unwrap();
        let abi: Result<JsonAbi, _> = serde_json::from_value(abi_value.clone());

        assert!(abi.is_ok());
        let abi = abi.unwrap();
        assert_eq!(abi.len(), 1);
    }
}
