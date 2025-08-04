use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tokio::process::Command;
use tower_lsp::{
    async_trait,
    lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url},
};

#[derive(Error, Debug)]
pub enum CompilerError {
    #[error("Invalid file URL")]
    InvalidUrl,
    #[error("Failed to run command: {0}")]
    CommandError(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Empty output from compiler")]
    EmptyOutput,
}

#[async_trait]
trait Compiler: Send + Sync {
    async fn lint(&self, file: &str) -> Result<serde_json::Value, CompilerError>;
    async fn build(&self, file: &str) -> Result<serde_json::Value, CompilerError>;
}

struct ForgeCompiler;

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeDiagnostic {
    #[serde(rename = "$message_type")]
    pub message_type: String,
    pub message: String,
    pub code: Option<ForgeLintCode>,
    pub level: String,
    pub spans: Vec<ForgeLintSpan>,
    pub children: Vec<ForgeLintChild>,
    pub rendered: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeLintCode {
    pub code: String,
    pub explanation: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeLintSpan {
    pub file_name: String,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line_start: u32,
    pub line_end: u32,
    pub column_start: u32,
    pub column_end: u32,
    pub is_primary: bool,
    pub text: Vec<ForgeLintText>,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeLintText {
    pub text: String,
    pub highlight_start: u32,
    pub highlight_end: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeLintChild {
    pub message: String,
    pub code: Option<String>,
    pub level: String,
    pub spans: Vec<ForgeLintSpan>,
    pub children: Vec<ForgeLintChild>,
    pub rendered: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeCompileError {
    #[serde(rename = "sourceLocation")]
    source_location: ForgeSourceLocation,
    #[serde(rename = "type")]
    error_type: String,
    component: String,
    severity: String,
    #[serde(rename = "errorCode")]
    error_code: String,
    message: String,
    #[serde(rename = "formattedMessage")]
    formatted_message: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeSourceLocation {
    file: String,
    start: i32, // Changed to i32 to handle -1 values
    end: i32,   // Changed to i32 to handle -1 values
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgeCompileOutput {
    errors: Option<Vec<ForgeCompileError>>,
    sources: serde_json::Value,
    contracts: serde_json::Value,
    build_infos: Vec<serde_json::Value>,
}

pub async fn get_lint_diagnostics(file: &Url) -> Result<Vec<Diagnostic>, CompilerError> {
    let path: PathBuf = file.to_file_path().map_err(|_| CompilerError::InvalidUrl)?;
    let path_str = path.to_str().ok_or(CompilerError::InvalidUrl)?;
    let compiler = ForgeCompiler;
    let lint_output = compiler.lint(path_str).await?;
    let diagnostics = lint_output_to_diagnostics(&lint_output, path_str);
    Ok(diagnostics)
}

pub async fn get_build_diagnostics(file: &Url) -> Result<Vec<Diagnostic>, CompilerError> {
    let path: PathBuf = file.to_file_path().map_err(|_| CompilerError::InvalidUrl)?;
    let path_str = path.to_str().ok_or(CompilerError::InvalidUrl)?;
    let compiler = ForgeCompiler;
    let build_output = compiler.build(path_str).await?;
    let diagnostics = build_output_to_diagnostics(&build_output);
    Ok(diagnostics)
}

pub fn build_output_to_diagnostics(forge_output: &serde_json::Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(errors) = forge_output.get("errors").and_then(|e| e.as_array()) {
        for err in errors {
            let message =
                err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error").to_string();

            let severity = match err.get("severity").and_then(|s| s.as_str()) {
                Some("error") => Some(DiagnosticSeverity::ERROR),
                Some("warning") => Some(DiagnosticSeverity::WARNING),
                Some("note") => Some(DiagnosticSeverity::INFORMATION),
                Some("help") => Some(DiagnosticSeverity::HINT),
                _ => Some(DiagnosticSeverity::INFORMATION),
            };

            let code = err
                .get("errorCode")
                .and_then(|c| c.as_str())
                .map(|s| NumberOrString::String(s.to_string()));

            // Attempt to extract line:column from formattedMessage
            let (line, column) = err
                .get("formattedMessage")
                .and_then(|fm| fm.as_str())
                .and_then(parse_line_col_from_formatted_message)
                .unwrap_or((0, 0)); // fallback to start of file

            let range = Range {
                start: Position {
                    line: line.saturating_sub(1),        // LSP is 0-based
                    character: column.saturating_sub(1), // LSP is 0-based
                },
                end: Position {
                    line: line.saturating_sub(1),
                    character: column.saturating_sub(1) + 1, // Just one char span
                },
            };

            let diagnostic = Diagnostic {
                range,
                severity,
                code,
                code_description: None,
                source: Some("forge-build".to_string()),
                message: format!("[forge build] {message}"),
                related_information: None,
                tags: None,
                data: None,
            };

            diagnostics.push(diagnostic);
        }
    }

    diagnostics
}

/// Parses `--> file.sol:17:5:` from formattedMessage and returns (line, column)
fn parse_line_col_from_formatted_message(msg: &str) -> Option<(u32, u32)> {
    // Find the line starting with `--> `
    for line in msg.lines() {
        if let Some(rest) = line.strip_prefix("  --> ") {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() >= 3 {
                let line = parts[1].parse::<u32>().ok()?;
                let column = parts[2].parse::<u32>().ok()?;
                return Some((line, column));
            }
        }
    }
    None
}

pub fn lint_output_to_diagnostics(
    forge_output: &serde_json::Value,
    target_file: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let serde_json::Value::Array(items) = forge_output {
        for item in items {
            if let Ok(forge_diag) = serde_json::from_value::<ForgeDiagnostic>(item.clone()) {
                // Only include diagnostics for the target file
                for span in &forge_diag.spans {
                    if span.file_name.ends_with(target_file) && span.is_primary {
                        let diagnostic = Diagnostic {
                            range: Range {
                                start: Position {
                                    line: (span.line_start - 1),        // LSP is 0-based
                                    character: (span.column_start - 1), // LSP is 0-based
                                },
                                end: Position {
                                    line: (span.line_end - 1),
                                    character: (span.column_end - 1),
                                },
                            },
                            severity: Some(match forge_diag.level.as_str() {
                                "error" => DiagnosticSeverity::ERROR,
                                "warning" => DiagnosticSeverity::WARNING,
                                "note" => DiagnosticSeverity::INFORMATION,
                                "help" => DiagnosticSeverity::HINT,
                                _ => DiagnosticSeverity::INFORMATION,
                            }),
                            code: forge_diag.code.as_ref().map(|c| {
                                tower_lsp::lsp_types::NumberOrString::String(c.code.clone())
                            }),
                            code_description: None,
                            source: Some("forge-lint".to_string()),
                            message: format!("[forge lint] {}", forge_diag.message),
                            related_information: None,
                            tags: None,
                            data: None,
                        };
                        diagnostics.push(diagnostic);
                        break; // Only take the first primary span per diagnostic
                    }
                }
            }
        }
    }

    diagnostics
}

#[async_trait]
impl Compiler for ForgeCompiler {
    async fn lint(&self, file_path: &str) -> Result<serde_json::Value, CompilerError> {
        let output =
            Command::new("forge").arg("lint").arg(file_path).arg("--json").output().await?;

        let stderr_str = String::from_utf8_lossy(&output.stderr);

        // Parse JSON output line by line
        let mut diagnostics = Vec::new();
        for line in stderr_str.lines() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value) => diagnostics.push(value),
                Err(_e) => {
                    continue;
                }
            }
        }

        Ok(serde_json::Value::Array(diagnostics))
    }

    async fn build(&self, file_path: &str) -> Result<serde_json::Value, CompilerError> {
        let output = Command::new("forge")
            .arg("build")
            .arg(file_path)
            .arg("--json")
            .arg("--no-cache")
            .arg("--ast")
            .output()
            .await?;

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout_str)?;

        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(testdata: &str) -> (std::string::String, ForgeCompiler) {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let file_path = format!("{manifest_dir}/{testdata}");
        let path = std::path::Path::new(&file_path);
        assert!(path.exists(), "Test file {path:?} does not exist");

        let compiler = ForgeCompiler;
        (file_path, compiler)
    }

    #[tokio::test]
    async fn test_build_success() {
        let (file_path, compiler) = setup("testdata/A.sol");

        let result = compiler.build(&file_path).await;
        assert!(result.is_ok(), "Expected build to succeed");

        let json = result.unwrap();
        assert!(json.get("sources").is_some(), "Expected 'sources' in output");
    }

    #[tokio::test]
    async fn test_build_has_errors_array() {
        let (file_path, compiler) = setup("testdata/A.sol");

        let json = compiler.build(&file_path).await.unwrap();
        assert!(json.get("errors").is_some(), "Expected 'errors' array in build output");
    }

    #[tokio::test]
    async fn test_build_error_formatting() {
        let (file_path, compiler) = setup("testdata/A.sol");

        let json = compiler.build(&file_path).await.unwrap();
        if let Some(errors) = json.get("errors") {
            if let Some(first) = errors.get(0) {
                assert!(first.get("message").is_some(), "Expected error object to have a message");
            }
        }
    }

    #[tokio::test]
    async fn test_lint_valid_file() {
        let compiler;
        let file_path;
        (file_path, compiler) = setup("testdata/A.sol");

        let result = compiler.lint(&file_path).await;
        assert!(result.is_ok(), "Expected lint to succeed");

        let json_value = result.unwrap();
        assert!(json_value.is_array(), "Expected lint output to be an array");
    }

    #[tokio::test]
    async fn test_lint_diagnosis_output() {
        let compiler;
        let file_path;
        (file_path, compiler) = setup("testdata/A.sol");

        let result = compiler.lint(&file_path).await;
        assert!(result.is_ok());

        let json_value = result.unwrap();
        let diagnostics = lint_output_to_diagnostics(&json_value, &file_path);
        assert!(!diagnostics.is_empty(), "Expected diagnostics");
    }

    #[tokio::test]
    async fn test_lint_to_lsp_diagnostics() {
        let compiler;
        let file_path;
        (file_path, compiler) = setup("testdata/A.sol");

        let result = compiler.lint(&file_path).await;
        assert!(result.is_ok(), "Expected lint to succeed");

        let json_value = result.unwrap();
        let diagnostics = lint_output_to_diagnostics(&json_value, &file_path);
        assert!(!diagnostics.is_empty(), "Expected at least one diagnostic");

        let first_diag = &diagnostics[0];
        assert_eq!(first_diag.source, Some("forge-lint".to_string()));
        assert_eq!(first_diag.message, "[forge lint] function names should use mixedCase");
        assert_eq!(
            first_diag.severity,
            Some(tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION)
        );
        assert_eq!(first_diag.range.start.line, 8);
        assert_eq!(first_diag.range.start.character, 13);
    }

    #[test]
    fn test_parse_line_col_from_valid_formatted_message() {
        let msg = r#"
Warning: Unused local variable.
  --> C.sol:19:5:
   |
19 |     bool fad;
   |     ^^^^^^^^
"#;
        let (line, col) = parse_line_col_from_formatted_message(msg).unwrap();
        assert_eq!(line, 19);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_parse_line_col_from_invalid_message() {
        let msg = "Something that doesn't match";
        assert!(parse_line_col_from_formatted_message(msg).is_none());
    }

    #[test]
    fn test_build_output_to_diagnostics_extracts_range() {
        let mock = serde_json::json!({
            "errors": [
                {
                    "sourceLocation": {
                        "file": "Test.sol",
                        "start": 123,
                        "end": 130
                    },
                    "severity": "warning",
                    "errorCode": "2072",
                    "message": "Unused local variable.",
                    "formattedMessage": "Warning: Unused local variable.\n  --> Test.sol:10:3:\n   |\n10 |     bool x;\n   |     ^^^^^^^\n"
                }
            ]
        });

        let diagnostics = build_output_to_diagnostics(&mock);
        assert_eq!(diagnostics.len(), 1);

        let diag = &diagnostics[0];
        assert!(diag.message.contains("Unused"));

        // Should be 0-based in the Diagnostic object
        let expected_range = Range {
            start: Position { line: 9, character: 2 },
            end: Position { line: 9, character: 3 },
        };
        assert_eq!(diag.range, expected_range);
    }

    #[test]
    fn test_build_output_to_diagnostics_empty() {
        let mock = serde_json::json!({ "errors": [] });
        let diagnostics = build_output_to_diagnostics(&mock);
        assert!(diagnostics.is_empty());
    }
}
