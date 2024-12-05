use serde::{Deserialize, Serialize};

use crate::shell;

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReportKind {
    #[default]
    Text,
    JSON,
}

/// Determine the kind of report to generate based on the current shell.
pub fn report_kind() -> ReportKind {
    if shell::is_json() {
        ReportKind::JSON
    } else {
        ReportKind::Text
    }
}
