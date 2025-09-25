use alloy_rpc_types_beacon::payload::BeaconBlockData;
use alloy_rpc_types_engine::ExecutionPayload;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_common::{fs, sh_print};
use std::path::PathBuf;

/// CLI arguments for `cast b2e-payload`, convert Beacon block's execution payload to Execution
/// JSON-RPC format.
#[derive(Parser)]
pub struct B2EPayloadArgs {
    /// Input data provided through JSON file path.
    #[arg(
        long = "json-file",
        value_name = "FILE",
        help = "Path to the JSON file containing the beacon block"
    )]
    pub json_file: PathBuf,
}

impl B2EPayloadArgs {
    pub async fn run(self) -> Result<()> {
        // Get input beacon block data
        let beacon_block = fs::read_to_string(&self.json_file)
            .map_err(|e| eyre!("Failed to read JSON file '{}': {}", self.json_file.display(), e))?;

        let beacon_block_data: BeaconBlockData = serde_json::from_str(&beacon_block)
            .map_err(|e| eyre!("Failed to parse beacon block JSON: {}", e))?;

        let execution_payload = beacon_block_data.execution_payload();

        let json_rpc_output = format_as_json_rpc(execution_payload.clone())?;
        sh_print!("{}", json_rpc_output)?;

        Ok(())
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
