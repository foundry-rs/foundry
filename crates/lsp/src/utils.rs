use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    #[error("ReadError")]
    ReadError,
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
    let path = file.to_file_path().map_err(|_| CompilerError::InvalidUrl)?;
    let path_str = path.to_str().ok_or(CompilerError::InvalidUrl)?;
    let filename =
        path.file_name().and_then(|os_str| os_str.to_str()).ok_or(CompilerError::InvalidUrl)?;
    let content = tokio::fs::read_to_string(&path).await.map_err(|_| CompilerError::ReadError)?;
    let compiler = ForgeCompiler;
    let build_output = compiler.build(path_str).await?;
    let diagnostics = build_output_to_diagnostics(&build_output, filename, &content);
    Ok(diagnostics)
}

pub fn build_output_to_diagnostics(
    forge_output: &serde_json::Value,
    filename: &str,
    content: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(errors) = forge_output.get("errors").and_then(|e| e.as_array()) {
        for err in errors {
            // Extract file name from error's sourceLocation.file path
            let source_file = err
                .get("sourceLocation")
                .and_then(|loc| loc.get("file"))
                .and_then(|f| f.as_str())
                .and_then(|full_path| Path::new(full_path).file_name())
                .and_then(|os_str| os_str.to_str());

            // Compare just the file names, not full paths
            if source_file != Some(filename) {
                continue;
            }

            // Rest of your code remains the same...
            let start_offset = err
                .get("sourceLocation")
                .and_then(|loc| loc.get("start"))
                .and_then(|s| s.as_u64())
                .unwrap_or(0) as usize;

            let end_offset = err
                .get("sourceLocation")
                .and_then(|loc| loc.get("end"))
                .and_then(|s| s.as_u64())
                .map(|v| v as usize)
                .unwrap_or(start_offset);

            let (start_line, start_col) = byte_offset_to_position(content, start_offset);
            let (mut end_line, mut end_col) = byte_offset_to_position(content, end_offset);

            if end_col > 0 {
                end_col -= 1;
            } else if end_line > 0 {
                end_line -= 1;
                end_col = content
                    .lines()
                    .nth(end_line.try_into().unwrap())
                    .map(|l| l.len() as u32)
                    .unwrap_or(0);
            }

            let range = Range {
                start: Position { line: start_line, character: start_col },
                end: Position { line: end_line, character: end_col + 1 },
            };

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

            diagnostics.push(Diagnostic {
                range,
                severity,
                code,
                code_description: None,
                source: Some("forge-build".to_string()),
                message: format!("[forge build] {message}"),
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }

    diagnostics
}

fn byte_offset_to_position(source: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 0;
    let mut bytes_counted = 0;

    for line_str in source.lines() {
        // Detect newline length after this line
        // Find the position after this line in source to check newline length
        let line_start = bytes_counted;
        let line_end = line_start + line_str.len();

        // Peek next char(s) to count newline length
        let newline_len = if source.get(line_end..line_end + 2) == Some("\r\n") {
            2
        } else if source.get(line_end..line_end + 1) == Some("\n") {
            1
        } else {
            0
        };

        let line_len = line_str.len() + newline_len;

        if bytes_counted + line_len > byte_offset {
            let col = (byte_offset - bytes_counted) as u32;
            return (line, col);
        }

        bytes_counted += line_len;
        line += 1;
    }

    (line, 0)
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
        if let Some(errors) = json.get("errors")
            && let Some(first) = errors.get(0)
        {
            assert!(first.get("message").is_some(), "Expected error object to have a message");
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

    #[tokio::test]
    async fn test_diagnostic_offsets_match_source() {
        let (file_path, compiler) = setup("testdata/A.sol");
        let source_code = tokio::fs::read_to_string(&file_path).await.expect("read source");
        let build_output = compiler.build(&file_path).await.expect("build failed");
        let expected_start_byte = 81;
        let expected_end_byte = 82;
        let expected_start_pos = byte_offset_to_position(&source_code, expected_start_byte);
        let expected_end_pos = byte_offset_to_position(&source_code, expected_end_byte);
        let filename = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|f| f.to_str())
            .expect("filename");
        let diagnostics = build_output_to_diagnostics(&build_output, filename, &source_code);
        assert!(!diagnostics.is_empty(), "no diagnostics found");

        let diag = &diagnostics[0];
        assert_eq!(diag.range.start.line, expected_start_pos.0);
        assert_eq!(diag.range.start.character, expected_start_pos.1);
        assert_eq!(diag.range.end.line, expected_end_pos.0);
        assert_eq!(diag.range.end.character, expected_end_pos.1);
    }

    #[tokio::test]
    async fn test_build_output_to_diagnostics_from_file() {
        let (file_path, compiler) = setup("testdata/A.sol");
        let source_code =
            tokio::fs::read_to_string(&file_path).await.expect("Failed to read source file");
        let build_output = compiler.build(&file_path).await.expect("Compiler build failed");
        let filename = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|f| f.to_str())
            .expect("Failed to get filename");

        let diagnostics = build_output_to_diagnostics(&build_output, filename, &source_code);
        assert!(!diagnostics.is_empty(), "Expected at least one diagnostic");

        let diag = &diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert!(diag.message.contains("Identifier is not a library name"));
        assert_eq!(diag.code, Some(NumberOrString::String("9589".to_string())));
        assert!(diag.range.start.line > 0);
        assert!(diag.range.start.character > 0);
    }
}
