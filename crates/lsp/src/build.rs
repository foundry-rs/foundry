use crate::utils::byte_offset_to_position;
use std::path::Path;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

fn ignored_code_for_tests(value: &serde_json::Value) -> bool {
    let error_code = value.get("errorCode").and_then(|v| v.as_str()).unwrap_or_default();
    let file_path = value
        .get("sourceLocation")
        .and_then(|loc| loc.get("file"))
        .and_then(|f| f.as_str())
        .unwrap_or_default();

    // Ignore error code 5574 for test files (code size limit)
    error_code == "5574" && (file_path.contains(".t.sol") || file_path.contains(".s.sol"))
}

pub fn build_output_to_diagnostics(
    forge_output: &serde_json::Value,
    filename: &str,
    content: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(errors) = forge_output.get("errors").and_then(|e| e.as_array()) {
        for err in errors {
            if ignored_code_for_tests(err) {
                continue;
            }

            let source_file = err
                .get("sourceLocation")
                .and_then(|loc| loc.get("file"))
                .and_then(|f| f.as_str())
                .and_then(|full_path| Path::new(full_path).file_name())
                .and_then(|os_str| os_str.to_str());

            if source_file != Some(filename) {
                continue;
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{ForgeRunner, Runner};
    use std::io::Write;

    static CONTRACT: &str = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

contract A {
    using B for string;

    function() internal c;

    function add_num(uint256 a) public pure returns (uint256) {
        bool fad;
        return a + 4;
    }
}"#;

    fn setup(contents: &str) -> (tempfile::TempPath, ForgeRunner) {
        let mut tmp =
            tempfile::Builder::new().suffix(".sol").tempfile().expect("failed to create temp file");

        tmp.write_all(contents.as_bytes()).expect("failed to write temp file");
        tmp.flush().expect("flush failed");
        tmp.as_file().sync_all().expect("sync failed");

        let path = tmp.into_temp_path();

        let compiler = ForgeRunner;
        (path, compiler)
    }

    #[tokio::test]
    async fn test_build_success() {
        let (tmp_file, compiler) = setup(CONTRACT);
        let file_path = tmp_file.to_string_lossy().to_string();

        let result = compiler.build(&file_path).await;
        assert!(result.is_ok(), "Expected build to succeed");
    }

    #[tokio::test]
    async fn test_build_has_errors_array() {
        let (file_, compiler) = setup(CONTRACT);
        let file_path = file_.to_string_lossy().to_string();

        let json = compiler.build(&file_path).await.unwrap();
        assert!(json.get("errors").is_some(), "Expected 'errors' array in build output");
    }

    #[tokio::test]
    async fn test_build_error_formatting() {
        let (file_, compiler) = setup(CONTRACT);
        let file_path = file_.to_string_lossy().to_string();

        let json = compiler.build(&file_path).await.unwrap();
        if let Some(errors) = json.get("errors")
            && let Some(first) = errors.get(0)
        {
            assert!(first.get("message").is_some(), "Expected error object to have a message");
        }
    }

    #[tokio::test]
    async fn test_diagnostic_offsets_match_source() {
        let (file_, compiler) = setup(CONTRACT);
        let file_path = file_.to_string_lossy().to_string();
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
        let (file_, compiler) = setup(CONTRACT);
        let file_path = file_.to_string_lossy().to_string();
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

    #[tokio::test]
    async fn test_ignored_code_for_tests() {
        let error_json = serde_json::json!({
            "errorCode": "5574",
            "sourceLocation": {
                "file": "test/ERC6909Claims.t.sol"
            }
        });
        assert!(ignored_code_for_tests(&error_json));

        let error_json_non_test = serde_json::json!({
            "errorCode": "5574",
            "sourceLocation": {
                "file": "contracts/ERC6909Claims.sol"
            }
        });
        assert!(!ignored_code_for_tests(&error_json_non_test));

        let error_json_other_code = serde_json::json!({
            "errorCode": "1234",
            "sourceLocation": {
                "file": "test/ERC6909Claims.t.sol"
            }
        });
        assert!(!ignored_code_for_tests(&error_json_other_code));
    }
}
