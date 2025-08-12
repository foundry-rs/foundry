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
    use crate::analyzer::test_utils::setup_analyzer;

    const LINT_CONTRACT: &str = include_str!("../../testdata/src/Deps.sol");

    #[test]
    fn test_lint_diagnosis_output() {
        // Setup analyzer. The analysis, including linting, runs inside.
        let (uri, analyzer, _temp_dir) = setup_analyzer(&[("LintContract.sol", LINT_CONTRACT)]);

        // Get the generated lint diagnostics
        let diagnostics = analyzer.get_lint_diagnostics(&uri).expect("Should get lint diagnostics");

        // Assert that we have diagnostics
        assert!(!diagnostics.is_empty(), "Expected diagnostics from linting");
    }

    #[test]
    fn test_lint_to_lsp_diagnostics() {
        // Setup analyzer
        let (uri, analyzer, _temp_dir) = setup_analyzer(&[("LintContract.sol", LINT_CONTRACT)]);

        // Get diagnostics
        let diagnostics = analyzer.get_lint_diagnostics(&uri).expect("Should get lint diagnostics");
        assert!(!diagnostics.is_empty(), "Expected at least one diagnostic");

        // Find the specific diagnostic for `add_num`
        let add_num_diagnostic = diagnostics
            .iter()
            .find(|d| d.message.contains("add_num"))
            .or_else(|| diagnostics.iter().find(|d| d.message.contains("mixedCase")));

        assert!(
            add_num_diagnostic.is_some(),
            "Expected to find a diagnostic for 'add_num' or 'mixedCase'"
        );

        let diag = add_num_diagnostic.unwrap();

        assert_eq!(diag.source, Some("forge-lint".to_string()));
        assert!(
            diag.message.contains("function names should use mixedCase"),
            "Diagnostic message is incorrect"
        );
        assert_eq!(
            diag.severity,
            Some(DiagnosticSeverity::INFORMATION),
            "Severity should be informational"
        );

        // `function add_num` is on line 7, column 14 in the source file.
        // LSP positions are 0-based, so we expect line 6, character 13.
        assert_eq!(
            diag.range.start,
            Position { line: 6, character: 13 },
            "Diagnostic start position is incorrect"
        );
    }
}
