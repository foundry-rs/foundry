use alloy_rpc_types_beacon::payload::execution_payload_from_beacon_str;
use alloy_rpc_types_engine::ExecutionPayload;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_common::{fs, sh_print};

/// CLI arguments for `cast b2e-payload`, convert Beacon block's execution payload to Execution
/// JSON-RPC format.
#[derive(Parser)]
pub struct B2EPayloadArgs {
    /// Input data provided through JSON file path.
    #[arg(help = "Path to the JSON file containing the beacon block")]
    pub json_file: String,
}

impl B2EPayloadArgs {
    pub async fn run(self) -> Result<()> {
        // Get input beacon block data
        let beacon_block = fs::read_to_string(&self.json_file)
            .map_err(|e| eyre!("Failed to read JSON file '{}': {}", self.json_file, e))?;

        // Extract and convert execution payload
        let execution_payload = Self::extract_and_convert_execution_payload(&beacon_block)?;

        let json_rpc_output = format_as_json_rpc(execution_payload)?;
        sh_print!("{}", json_rpc_output)?;

        Ok(())
    }

    // Extracts the execution payload from a beacon block JSON string and converts it to
    // `ExecutionPayload` It matches `beaconcha.in` json format
    fn extract_and_convert_execution_payload(beacon_block: &str) -> Result<ExecutionPayload> {
        let beacon_json: serde_json::Value = serde_json::from_str(beacon_block)
            .map_err(|e| eyre!("Failed to parse beacon block JSON: {}", e))?;

        // early detection if the format is not correct
        if beacon_json
            .get("message")
            .and_then(|m| m.get("body"))
            .and_then(|b| b.get("execution_payload"))
            .is_none()
        {
            return Err(eyre!("Invalid beacon block format: missing 'message' field"));
        }
        // Extract the "message.body.execution_payload" field from the beacon block JSON
        // TODO: check if we extract from beacon api it works but not sure it will work with all API
        // interfaces
        let execution_payload_beacon_block = beacon_json
            .get("message")
            .and_then(|m| m.get("body"))
            .and_then(|b| b.get("execution_payload"))
            .ok_or_else(|| eyre!("Could not find execution_payload in beacon block"))?;

        let execution_payload_str = serde_json::to_string(execution_payload_beacon_block)
            .map_err(|e| eyre!("Failed to serialize execution payload: {}", e))?;

        // Convert beacon block's execution payload to json rpc execution payload
        let execution_payload = execution_payload_from_beacon_str(&execution_payload_str)?;

        Ok(execution_payload)
    }
}

// Helper to format the execution payload as JSON-RPC response
fn format_as_json_rpc(execution_payload: ExecutionPayload) -> Result<String> {
    // TODO: check if we used this format and this method engine version
    let json_rpc_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "engine_newPayloadV3",
        "params": [execution_payload],
        "id": 1
    });

    serde_json::to_string_pretty(&json_rpc_request)
        .map_err(|e| eyre!("Failed to serialize JSON-RPC response: {}", e))
}
