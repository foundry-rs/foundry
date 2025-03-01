use crate::{artifacts::vyper::VyperCompilationError, compilers::CompilationError};
use foundry_compilers_artifacts::{error::SourceLocation, Severity};

impl CompilationError for VyperCompilationError {
    fn is_warning(&self) -> bool {
        self.severity.is_warning()
    }

    fn is_error(&self) -> bool {
        self.severity.is_error()
    }

    fn source_location(&self) -> Option<SourceLocation> {
        None
    }

    fn severity(&self) -> Severity {
        self.severity
    }

    fn error_code(&self) -> Option<u64> {
        None
    }
}
