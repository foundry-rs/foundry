use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{ForgeRunner, Runner};

    fn setup(testdata: &str) -> (std::string::String, ForgeRunner) {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let file_path = format!("{manifest_dir}/{testdata}");
        let path = std::path::Path::new(&file_path);
        assert!(path.exists(), "Test file {path:?} does not exist");

        let compiler = ForgeRunner;
        (file_path, compiler)
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
}
