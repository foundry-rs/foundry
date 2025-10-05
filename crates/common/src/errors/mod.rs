//! Commonly used errors

mod fs;
pub use fs::FsPathError;

mod private {
    use eyre::Chain;
    use std::error::Error;

    pub trait ErrorChain {
        fn chain(&self) -> Chain<'_>;
    }

    impl ErrorChain for dyn Error + 'static {
        fn chain(&self) -> Chain<'_> {
            Chain::new(self)
        }
    }

    impl ErrorChain for eyre::Report {
        fn chain(&self) -> Chain<'_> {
            self.chain()
        }
    }
}

/// Displays a chain of errors in a single line.
pub fn display_chain<E: private::ErrorChain + ?Sized>(error: &E) -> String {
    dedup_chain(error).join("; ")
}

/// Deduplicates a chain of errors.
pub fn dedup_chain<E: private::ErrorChain + ?Sized>(error: &E) -> Vec<String> {
    let mut causes = all_sources(error);
    // Deduplicate the common pattern `msg1: msg2; msg2` -> `msg1: msg2`.
    causes.dedup_by(|b, a| a.contains(b.as_str()));
    causes
}

fn all_sources<E: private::ErrorChain + ?Sized>(err: &E) -> Vec<String> {
    err.chain().map(|cause| cause.to_string().trim().to_string()).collect()
}

/// Converts solar errors to an eyre error.
pub fn convert_solar_errors(dcx: &solar::interface::diagnostics::DiagCtxt) -> eyre::Result<()> {
    match dcx.emitted_errors() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) if !e.is_empty() => eyre::bail!("solar run failed:\n\n{e}"),
        _ if dcx.has_errors().is_err() => eyre::bail!("solar run failed"),
        _ => Ok(()),
    }
}

/// Sanitizes compiler error messages by removing sensitive information.
///
/// This function filters out potentially sensitive information from compiler diagnostics
/// such as absolute file paths, internal system details, and other sensitive data
/// that could be exposed in error messages.
///
/// # Arguments
///
/// * `diagnostics` - The raw compiler diagnostics string to sanitize
///
/// # Returns
///
/// A sanitized version of the diagnostics with sensitive information removed or masked.
pub fn sanitize_compiler_diagnostics(diagnostics: &str) -> String {
    use regex::Regex;
    
    let mut sanitized = diagnostics.to_string();
    
    // List of regex patterns to sanitize sensitive information
    let patterns = [
        // Remove absolute file paths - replace with relative paths or [REDACTED_PATH]
        (r"/[^\s\n]*\.sol", "[REDACTED_PATH].sol"),
        (r"/[^\s\n]*\.rs", "[REDACTED_PATH].rs"),
        (r"/[^\s\n]*\.json", "[REDACTED_PATH].json"),
        // Remove Windows absolute paths
        (r"[A-Za-z]:\\[^\s\n]*\.sol", "[REDACTED_PATH].sol"),
        (r"[A-Za-z]:\\[^\s\n]*\.rs", "[REDACTED_PATH].rs"),
        (r"[A-Za-z]:\\[^\s\n]*\.json", "[REDACTED_PATH].json"),
        // Remove home directory paths
        (r"~/[^\s\n]*", "[REDACTED_PATH]"),
        (r"/home/[^/\s\n]+/[^\s\n]*", "[REDACTED_PATH]"),
        (r"/Users/[^/\s\n]+/[^\s\n]*", "[REDACTED_PATH]"),
        // Remove specific compiler version details that might reveal system info
        (r"Solc( version)? \d+\.\d+\.\d+", "Solc [VERSION]"),
        // Remove line and column information that might reveal internal structure
        (r":\d+:\d+:", ":[LINE]:[COL]:"),
        (r"line \d+", "line [LINE]"),
        (r"column \d+", "column [COL]"),
        // Remove memory addresses or internal references
        (r"0x[0-9a-fA-F]{8,}", "[ADDRESS]"),
        // Remove potential environment variable paths
        (r"\$[A-Z_]+/[^\s\n]*", "[ENV_PATH]"),
        (r"%[A-Z_]+%[^\s\n]*", "[ENV_PATH]"),
    ];
    
    for (pattern, replacement) in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            sanitized = regex.replace_all(&sanitized, replacement).to_string();
        }
    }
    
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedups_contained() {
        #[derive(thiserror::Error, Debug)]
        #[error("my error: {0}")]
        struct A(#[from] B);

        #[derive(thiserror::Error, Debug)]
        #[error("{0}")]
        struct B(String);

        let ee = eyre::Report::from(A(B("hello".into())));
        assert_eq!(ee.chain().count(), 2, "{ee:?}");
        let full = all_sources(&ee).join("; ");
        assert_eq!(full, "my error: hello; hello");
        let chained = display_chain(&ee);
        assert_eq!(chained, "my error: hello");
    }

    #[test]
    fn test_sanitize_compiler_diagnostics() {
        let test_input = r#"
Error: /home/user/project/contracts/MyContract.sol:123:45: DeclarationError: Identifier already declared.
  --> /Users/developer/src/Token.sol:67:12
  |
67 |     function transfer(address to, uint256 amount) public returns (bool) {
  |            ^^^^^^^^^
  |
  = note: Solc version 0.8.19+commit.7dd6d404
  = help: Consider using a different name or removing the duplicate declaration.
  
Error: C:\Users\Admin\Documents\project\utils\Helper.sol:89:23: TypeError: Member "balance" not found.
  --> 0x1234567890abcdef1234567890abcdef12345678
  |
89 |     uint256 bal = address(this).balance;
  |                       ^^^^^^^
"#;

        let sanitized = sanitize_compiler_diagnostics(test_input);
        
        // Check that sensitive paths are redacted
        assert!(!sanitized.contains("/home/user/project/"));
        assert!(!sanitized.contains("/Users/developer/src/"));
        assert!(!sanitized.contains("C:\\Users\\Admin\\Documents\\"));
        assert!(sanitized.contains("[REDACTED_PATH]"));
        
        // Check that version info is redacted
        assert!(!sanitized.contains("0.8.19+commit.7dd6d404"));
        assert!(sanitized.contains("Solc [VERSION]"));
        
        // Check that line/column info is redacted
        assert!(!sanitized.contains(":123:45:"));
        assert!(!sanitized.contains(":67:12"));
        assert!(!sanitized.contains(":89:23"));
        assert!(sanitized.contains(":[LINE]:[COL]:"));
        
        // Check that addresses are redacted
        assert!(!sanitized.contains("0x1234567890abcdef1234567890abcdef12345678"));
        assert!(sanitized.contains("[ADDRESS]"));
        
        // Check that error messages are preserved
        assert!(sanitized.contains("DeclarationError"));
        assert!(sanitized.contains("TypeError"));
        assert!(sanitized.contains("Identifier already declared"));
        assert!(sanitized.contains("Member \"balance\" not found"));
    }
}
