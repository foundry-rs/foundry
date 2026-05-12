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
        Some(Err(e)) if !e.is_empty() => eyre::bail!("solar reported errors:\n\n{e}"),
        _ if dcx.has_errors().is_err() => {
            // Non-buffer emitter: diagnostics already went to stderr; include the count.
            let n = dcx.err_count();
            let plural = if n == 1 { "" } else { "s" };
            eyre::bail!(
                "solar reported {n} error{plural}; see the diagnostic{plural} printed above"
            )
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar::interface::diagnostics::{DiagCtxt, SilentEmitter};

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

    /// Regression test for the "non-buffer emitter" branch of [`convert_solar_errors`].
    ///
    /// Simulates an unhandled solar edge case: the linter installs a non-buffer (stderr-style)
    /// emitter, errors are emitted to it, and only the count is recoverable afterwards. The
    /// returned eyre error must reference the count and direct the user to the diagnostics that
    /// were already printed above.
    #[test]
    fn solar_non_buffer_emitter_singular() {
        let dcx = DiagCtxt::new(Box::new(SilentEmitter::new_silent()));
        dcx.err("boom").emit();

        let err = convert_solar_errors(&dcx).unwrap_err();
        assert_eq!(err.to_string(), "solar reported 1 error; see the diagnostic printed above");
    }

    #[test]
    fn solar_non_buffer_emitter_plural() {
        let dcx = DiagCtxt::new(Box::new(SilentEmitter::new_silent()));
        dcx.err("boom 1").emit();
        dcx.err("boom 2").emit();

        let err = convert_solar_errors(&dcx).unwrap_err();
        assert_eq!(err.to_string(), "solar reported 2 errors; see the diagnostics printed above");
    }

    #[test]
    fn solar_no_errors_is_ok() {
        let dcx = DiagCtxt::new(Box::new(SilentEmitter::new_silent()));
        assert!(convert_solar_errors(&dcx).is_ok());
    }
}
