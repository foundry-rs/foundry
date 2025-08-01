use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tokio::process::Command;
use tower_lsp::{
    async_trait,
    lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, Url},
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lint_valid_file() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let file_path = format!("{manifest_dir}/testdata/A.sol");
        let path = std::path::Path::new(&file_path);
        assert!(path.exists(), "Test file {path:?} does not exist");

        let compiler = ForgeCompiler;
        let result = compiler.lint(&file_path).await;

        assert!(result.is_ok(), "Expected lint to succeed");
        let json_value = result.unwrap();

        assert!(json_value.is_array(), "Expected lint output to be an array");
    }


    #[tokio::test]
    async fn test_debug_lint_conversion() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let file_path = format!("{manifest_dir}/testdata/A.sol");

        let compiler = ForgeCompiler;
        let result = compiler.lint(&file_path).await;
        assert!(result.is_ok());

        let json_value = result.unwrap();
        let diagnostics = lint_output_to_diagnostics(&json_value, &file_path);

        assert!(!diagnostics.is_empty(), "Expected diagnostics");
    }

    #[tokio::test]
    async fn test_forge_lint_to_lsp_diagnostics() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let file_path = format!("{manifest_dir}/testdata/A.sol");
        let path = std::path::Path::new(&file_path);
        assert!(path.exists(), "Test file {path:?} does not exist");

        let compiler = ForgeCompiler;
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
}
