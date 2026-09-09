//! Command Line handler to convert Beacon block's execution payload to Execution format.

use alloy_rpc_types_beacon::payload::BeaconBlockData;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_common::{fs, sh_print};

/// CLI arguments for `cast b2e-payload`, convert Beacon block's execution payload to Execution
/// format.
#[derive(Parser)]
pub struct B2EPayloadArgs {
    /// Input data, it can be either a file path to JSON file or raw JSON string containing the
    /// beacon block
    #[arg(
        value_name = "INPUT",
        help = "File path to JSON file or raw JSON string containing the beacon block"
    )]
    pub input: String,
}

impl B2EPayloadArgs {
    pub async fn run(self) -> Result<()> {
        let json = read_input(self.input)?;
        let beacon_block_data: BeaconBlockData = serde_json::from_str(&json)
            .map_err(|e| eyre!("Failed to parse beacon block JSON: {}", e))?;
        let output = serde_json::to_string(&beacon_block_data.execution_payload())
            .map_err(|e| eyre!("Failed to serialize execution payload: {}", e))?;
        sh_print!("{}", output)?;
        Ok(())
    }
}

/// Returns `input` if it is raw JSON, otherwise reads it as a file path.
fn read_input(input: String) -> Result<String> {
    if serde_json::from_str::<serde_json::Value>(&input).is_ok() {
        return Ok(input);
    }
    fs::read_to_string(&input).map_err(|e| eyre!("Failed to read JSON file '{input}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_input_prefers_raw_json_over_paths() {
        let json = r#"{"execution_payload": {"block_hash": "0x123"}}"#;
        assert_eq!(read_input(json.to_string()).unwrap(), json);
        let json = r#"[{"block": "data"}]"#;
        assert_eq!(read_input(json.to_string()).unwrap(), json);

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), json).unwrap();
        assert_eq!(read_input(file.path().to_string_lossy().into_owned()).unwrap(), json);

        let err = read_input("not-json-{".to_string()).unwrap_err().to_string();
        assert!(err.starts_with("Failed to read JSON file 'not-json-{'"), "{err}");
    }
}
