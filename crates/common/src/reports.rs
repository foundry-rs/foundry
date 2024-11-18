use serde::{Deserialize, Serialize};

use crate::shell;

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReportKind {
    #[default]
    Markdown,
    JSON,
}

/// Determine the kind of report to generate based on the current shell.
pub fn report_kind() -> ReportKind {
    if shell::is_json() {
        ReportKind::JSON
    } else {
        ReportKind::Markdown
    }
}
