//! Commonly used errors

mod fs;
pub use fs::FsPathError;

mod artifacts;
pub use artifacts::*;

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
