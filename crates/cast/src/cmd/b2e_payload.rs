//! Command Line handler to convert Beacon block's execution payload to Execution format.

use std::path::PathBuf;

use alloy_rpc_types_beacon::payload::BeaconBlockData;
use clap::{Parser, builder::ValueParser};
use eyre::{Result, eyre};
use foundry_common::{fs, sh_print};

/// CLI arguments for `cast b2e-payload`, convert Beacon block's execution payload to Execution
/// format.
#[derive(Parser)]
pub struct B2EPayloadArgs {
    /// Input data, it can be either a file path to JSON file or raw JSON string containing the
    /// beacon block
    #[arg(value_name = "INPUT", value_parser=ValueParser::new(parse_input_source), help = "File path to JSON file or raw JSON string containing the beacon block")]
    pub input: InputSource,
}

impl B2EPayloadArgs {
    pub async fn run(self) -> Result<()> {
        let beacon_block_json = match self.input {
            InputSource::Json(json) => json,
            InputSource::File(path) => fs::read_to_string(&path)
                .map_err(|e| eyre!("Failed to read JSON file '{}': {}", path.display(), e))?,
        };

        let beacon_block_data: BeaconBlockData = serde_json::from_str(&beacon_block_json)
            .map_err(|e| eyre!("Failed to parse beacon block JSON: {}", e))?;

        let execution_payload = beacon_block_data.execution_payload();

        // Output raw execution payload
        let output = serde_json::to_string(&execution_payload)
            .map_err(|e| eyre!("Failed to serialize execution payload: {}", e))?;
        sh_print!("{}", output)?;

        Ok(())
    }
}

/// Represents the different input sources for beacon block data
#[derive(Debug, Clone)]
pub enum InputSource {
    /// Path to a JSON file containing beacon block data
    File(PathBuf),
    /// Raw JSON string containing beacon block data
    Json(String),
}

fn parse_input_source(s: &str) -> Result<InputSource, String> {
    // Try parsing as JSON first
    if serde_json::from_str::<serde_json::Value>(s).is_ok() {
        return Ok(InputSource::Json(s.to_string()));
    }

    // Otherwise treat as file path
    Ok(InputSource::File(PathBuf::from(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_input_source_json_object() {
        let json_input = r#"{"execution_payload": {"block_hash": "0x123"}}"#;
        let result = parse_input_source(json_input).unwrap();

        match result {
            InputSource::Json(json) => assert_eq!(json, json_input),
            InputSource::File(_) => panic!("Expected JSON input, got File"),
        }
    }

    #[test]
    fn test_parse_input_source_json_array() {
        let json_input = r#"[{"block": "data"}]"#;
        let result = parse_input_source(json_input).unwrap();

        match result {
            InputSource::Json(json) => assert_eq!(json, json_input),
            InputSource::File(_) => panic!("Expected JSON input, got File"),
        }
    }

    #[test]
    fn test_parse_input_source_file_path() {
        let file_path =
            "block-12225729-6ceadbf2a6adbbd64cbec33fdebbc582f25171cd30ac43f641cbe76ac7313ddf.json";
        let result = parse_input_source(file_path).unwrap();

        match result {
            InputSource::File(path) => assert_eq!(path, PathBuf::from(file_path)),
            InputSource::Json(_) => panic!("Expected File input, got JSON"),
        }
    }

    #[test]
    fn test_parse_input_source_malformed_but_not_json() {
        let malformed = "not-json-{";
        let result = parse_input_source(malformed).unwrap();

        // Should be treated as file path since it's not valid JSON
        match result {
            InputSource::File(path) => assert_eq!(path, PathBuf::from(malformed)),
            InputSource::Json(_) => panic!("Expected File input, got File"),
        }
    }
}
