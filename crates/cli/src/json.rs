//! Shared JSON output primitives for Foundry CLIs.

use eyre::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The current version of Foundry's top-level JSON output envelope.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// Stable top-level envelope for machine-readable command output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonEnvelope<T> {
    /// Version of the envelope schema.
    pub schema_version: u32,
    /// Whether the command completed successfully.
    pub success: bool,
    /// Command-specific payload.
    pub data: Option<T>,
    /// Structured errors emitted by the command.
    pub errors: Vec<JsonError>,
    /// Structured warnings emitted by the command.
    pub warnings: Vec<JsonWarning>,
}

impl<T> JsonEnvelope<T> {
    /// Creates a successful envelope with command-specific data.
    pub const fn success(data: T) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            success: true,
            data: Some(data),
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Creates a successful envelope with command-specific data and warnings.
    pub const fn success_with_warnings(data: T, warnings: Vec<JsonWarning>) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            success: true,
            data: Some(data),
            errors: Vec::new(),
            warnings,
        }
    }
}

impl JsonEnvelope<()> {
    /// Creates a failed envelope with one structured error.
    pub fn error(error: JsonError) -> Self {
        Self::failure(vec![error])
    }

    /// Creates a failed envelope with structured errors.
    pub const fn failure(errors: Vec<JsonError>) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            success: false,
            data: None,
            errors,
            warnings: Vec::new(),
        }
    }
}

/// Structured error entry for JSON output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured context for the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl JsonError {
    /// Creates a structured error without details.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), details: None }
    }

    /// Adds structured details to the error.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Structured warning entry for JSON output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonWarning {
    /// Stable machine-readable warning code.
    pub code: String,
    /// Human-readable warning message.
    pub message: String,
    /// Optional structured context for the warning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl JsonWarning {
    /// Creates a structured warning without details.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), details: None }
    }

    /// Adds structured details to the warning.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Serializes a value as pretty JSON.
pub fn to_json_string<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

/// Prints a value as pretty JSON to stdout.
pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    sh_println!("{}", to_json_string(value)?)?;
    Ok(())
}

/// Prints a successful JSON envelope to stdout.
pub fn print_json_success<T: Serialize>(data: T) -> Result<()> {
    print_json(&JsonEnvelope::success(data))
}

/// Prints a successful JSON envelope with warnings to stdout.
pub fn print_json_success_with_warnings<T: Serialize>(
    data: T,
    warnings: Vec<JsonWarning>,
) -> Result<()> {
    print_json(&JsonEnvelope::success_with_warnings(data, warnings))
}

/// Prints a failed JSON envelope with one structured error to stdout.
pub fn print_json_error(error: JsonError) -> Result<()> {
    print_json(&JsonEnvelope::error(error))
}

/// Prints a failed JSON envelope with structured errors to stdout.
pub fn print_json_failure(errors: Vec<JsonError>) -> Result<()> {
    print_json(&JsonEnvelope::failure(errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Serialize)]
    struct BuildData {
        contracts: usize,
    }

    #[test]
    fn success_envelope_serializes_all_top_level_fields() {
        let envelope = JsonEnvelope::success(BuildData { contracts: 2 });

        let json = to_json_string(&envelope).unwrap();

        assert_eq!(
            json,
            r#"{
  "schema_version": 1,
  "success": true,
  "data": {
    "contracts": 2
  },
  "errors": [],
  "warnings": []
}"#
        );
    }

    #[test]
    fn warning_details_are_structured() {
        let warning = JsonWarning::new("compiler.remappings", "auto-detected remappings")
            .with_details(json!({ "count": 3 }));
        let envelope =
            JsonEnvelope::success_with_warnings(BuildData { contracts: 1 }, vec![warning]);

        let value: Value = serde_json::from_str(&to_json_string(&envelope).unwrap()).unwrap();

        assert_eq!(value["success"], true);
        assert_eq!(value["warnings"][0]["code"], "compiler.remappings");
        assert_eq!(value["warnings"][0]["details"]["count"], 3);
    }

    #[test]
    fn failure_envelope_serializes_null_data_and_structured_errors() {
        let error = JsonError::new("config.invalid", "invalid foundry.toml")
            .with_details(json!({ "path": "foundry.toml" }));
        let envelope = JsonEnvelope::error(error);

        let value: Value = serde_json::from_str(&to_json_string(&envelope).unwrap()).unwrap();

        assert_eq!(value["schema_version"], JSON_SCHEMA_VERSION);
        assert_eq!(value["success"], false);
        assert!(value["data"].is_null());
        assert_eq!(value["errors"][0]["code"], "config.invalid");
        assert_eq!(value["errors"][0]["details"]["path"], "foundry.toml");
        assert_eq!(value["warnings"], json!([]));
    }
}
