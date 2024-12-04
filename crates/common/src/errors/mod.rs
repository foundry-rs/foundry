//! Commonly used errors

mod fs;
pub use fs::FsPathError;

mod artifacts;
pub use artifacts::*;

/// Displays a chain of errors in a single line.
pub fn display_chain(error: &eyre::Report) -> String {
    let mut causes = all_sources(error);
    // Deduplicate the common pattern `msg1: msg2; msg2` -> `msg1: msg2`.
    causes.dedup_by(|b, a| a.contains(b.as_str()));
    causes.join("; ")
}

fn all_sources(err: &eyre::Report) -> Vec<String> {
    err.chain().map(|cause| cause.to_string().trim().to_string()).collect()
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
}
