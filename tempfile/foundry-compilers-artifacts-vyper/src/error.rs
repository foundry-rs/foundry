use core::fmt;
use foundry_compilers_artifacts_solc::Severity;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VyperSourceLocation {
    file: PathBuf,
    #[serde(rename = "lineno")]
    line: Option<u64>,
    #[serde(rename = "col_offset")]
    offset: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VyperCompilationError {
    pub message: String,
    pub severity: Severity,
    pub source_location: Option<VyperSourceLocation>,
    pub formatted_message: Option<String>,
}

impl fmt::Display for VyperCompilationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = &self.source_location {
            write!(f, "Location: {}", location.file.display())?;
            if let Some(line) = location.line {
                write!(f, ":{line}")?;
            }
            if let Some(offset) = location.offset {
                write!(f, ":{offset}")?;
            }
            writeln!(f)?;
        }
        if let Some(message) = &self.formatted_message {
            write!(f, "{message}")
        } else {
            write!(f, "{}", self.message)
        }
    }
}
