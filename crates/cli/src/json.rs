//! Shared JSON output primitives for Foundry CLIs.

use alloy_dyn_abi::DynSolValue;
use eyre::Result;
use foundry_common::{
    fmt::{format_tokens, serialize_value_as_json},
    sh_println, shell,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, to_string};

/// The current version of Foundry's top-level JSON output envelope.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// Stable top-level envelope for complete machine-readable command output.
///
/// This envelope represents a terminal command outcome. Long-running commands
/// that stream intermediate records should use a separate event type and reserve
/// this shape for final, complete results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonEnvelope<T> {
    /// Version of the envelope schema.
    pub schema_version: u32,
    /// Whether the command completed successfully.
    ///
    /// Only meaningful for a complete/terminal command outcome.
    pub success: bool,
    /// Command-specific payload.
    pub data: Option<T>,
    /// Structured errors emitted by the command.
    pub errors: Vec<JsonMessage>,
    /// Structured warnings emitted by the command.
    pub warnings: Vec<JsonMessage>,
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
    pub const fn success_with_warnings(data: T, warnings: Vec<JsonMessage>) -> Self {
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
    pub fn error(error: JsonMessage) -> Self {
        Self::failure(vec![error])
    }

    /// Creates a failed envelope with structured errors.
    pub const fn failure(errors: Vec<JsonMessage>) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            success: false,
            data: None,
            errors,
            warnings: Vec::new(),
        }
    }
}

/// Severity level for a structured JSON diagnostic.
///
/// These levels classify diagnostics attached to an envelope. Progress,
/// informational, and debug records should be modeled as command output data or
/// stream events instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonMessageLevel {
    /// Error message.
    Error,
    /// Warning message.
    Warning,
}

/// Structured diagnostic entry for JSON output.
///
/// Diagnostics describe errors and warnings associated with command output. They
/// are not intended for progress, informational, or debug events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonMessage {
    /// Diagnostic severity level.
    pub level: JsonMessageLevel,
    /// Stable machine-readable diagnostic code.
    pub code: String,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Optional structured context for the diagnostic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl JsonMessage {
    /// Creates a structured error without details.
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: JsonMessageLevel::Error,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Creates a structured warning without details.
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: JsonMessageLevel::Warning,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Adds structured details to the diagnostic.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Prints a serializable object: envelope-wrapped in `--json` mode, pretty-printed otherwise.
///
/// Use this for objects that have no human-readable `Display` format (block data, RPC responses,
/// etc.).
pub fn print_json_object<T: Serialize>(value: T) -> Result<()> {
    if foundry_common::shell::is_json() {
        print_json_success(value)
    } else {
        sh_println!("{}", serde_json::to_string_pretty(&value)?)?;
        Ok(())
    }
}

/// Prints a value as one compact JSON line on stdout and flushes.
///
/// Bypasses the shell verbosity layer so `--quiet` cannot suppress structured
/// output the caller explicitly asked for.
pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let s = to_string(value)?;
    let mut shell = foundry_common::shell::Shell::get();
    let out = shell.out();
    writeln!(out, "{s}")?;
    out.flush()?;
    Ok(())
}

/// Prints a successful JSON envelope to stdout.
pub fn print_json_success<T: Serialize>(data: T) -> Result<()> {
    print_json(&JsonEnvelope::success(data))
}

/// Prints a successful JSON envelope with warnings to stdout.
pub fn print_json_success_with_warnings<T: Serialize>(
    data: T,
    warnings: Vec<JsonMessage>,
) -> Result<()> {
    print_json(&JsonEnvelope::success_with_warnings(data, warnings))
}

/// Prints command output that may already be JSON: parsed and envelope-wrapped in `--json` mode,
/// plain text otherwise. If the output is not valid JSON, it is wrapped as a scalar string.
pub fn print_json_value_or_scalar(value: impl AsRef<str> + std::fmt::Display) -> Result<()> {
    if shell::is_json() {
        match serde_json::from_str::<Value>(value.as_ref()) {
            Ok(value) => print_json_success(value),
            Err(_) => print_json_success(value.as_ref()),
        }
    } else {
        sh_println!("{value}")?;
        Ok(())
    }
}

/// Prints a scalar value: JSON envelope in `--json` mode, plain text otherwise.
pub fn print_scalar(value: impl Serialize + std::fmt::Display) -> Result<()> {
    if shell::is_json() {
        print_json_success(value)
    } else {
        sh_println!("{value}")?;
        Ok(())
    }
}

/// Prints a list of serializable items: JSON envelope wrapping an array in `--json` mode,
/// one item per line otherwise.
pub fn print_list<T: Serialize + std::fmt::Display>(items: &[T]) -> Result<()> {
    if shell::is_json() {
        print_json_success(items)
    } else {
        for item in items {
            sh_println!("{item}")?;
        }
        Ok(())
    }
}

/// Prints ABI-decoded tokens: JSON envelope wrapping a value array in `--json` mode,
/// one formatted token per line otherwise.
pub fn print_tokens(tokens: &[DynSolValue]) -> Result<()> {
    if shell::is_json() {
        let values = tokens
            .iter()
            .cloned()
            .map(|t| serialize_value_as_json(t, None))
            .collect::<Result<Vec<Value>>>()?;
        print_json_success(values)
    } else {
        format_tokens(tokens).try_for_each(|t| sh_println!("{t}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[derive(Serialize)]
    struct BuildData {
        contracts: usize,
    }

    #[test]
    fn success_envelope_serializes_all_top_level_fields() {
        let envelope = JsonEnvelope::success(BuildData { contracts: 2 });

        let json = to_string(&envelope).unwrap();

        assert_eq!(
            json,
            r#"{"schema_version":1,"success":true,"data":{"contracts":2},"errors":[],"warnings":[]}"#
        );
    }

    #[test]
    fn warning_details_are_structured() {
        let warning = JsonMessage::warning("compiler.remappings", "auto-detected remappings")
            .with_details(json!({ "count": 3 }));
        let envelope =
            JsonEnvelope::success_with_warnings(BuildData { contracts: 1 }, vec![warning]);

        let value = to_value(&envelope).unwrap();

        assert_eq!(value["success"], true);
        assert_eq!(value["warnings"][0]["level"], "warning");
        assert_eq!(value["warnings"][0]["code"], "compiler.remappings");
        assert_eq!(value["warnings"][0]["details"]["count"], 3);
    }

    #[test]
    fn failure_envelope_serializes_null_data_and_structured_errors() {
        let error = JsonMessage::error("config.invalid", "invalid foundry.toml")
            .with_details(json!({ "path": "foundry.toml" }));
        let envelope = JsonEnvelope::error(error);

        let value = to_value(&envelope).unwrap();

        assert_eq!(value["schema_version"], JSON_SCHEMA_VERSION);
        assert_eq!(value["success"], false);
        assert!(value["data"].is_null());
        assert_eq!(value["errors"][0]["level"], "error");
        assert_eq!(value["errors"][0]["code"], "config.invalid");
        assert_eq!(value["errors"][0]["details"]["path"], "foundry.toml");
        assert_eq!(value["warnings"], json!([]));
    }
}
