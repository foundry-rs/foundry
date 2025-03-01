use crate::store_impl::{
    file,
    file::{log, loose, loose::Reference},
};

pub(crate) fn must_be_io_err(err: loose::reflog::Error) -> std::io::Error {
    match err {
        loose::reflog::Error::Io(err) => err,
        loose::reflog::Error::RefnameValidation(_) => unreachable!("we are called from a valid ref"),
    }
}

impl Reference {
    /// Returns true if a reflog exists in the given `store`.
    ///
    /// Please note that this method shouldn't be used to check if a log exists before trying to read it, but instead
    /// is meant to be the fastest possible way to determine if a log exists or not.
    /// If the caller needs to know if it's readable, try to read the log instead with a reverse or forward iterator.
    pub fn log_exists(&self, store: &file::Store) -> bool {
        store
            .reflog_exists(self.name.as_ref())
            .expect("name conversion infallible")
    }
    /// Return a reflog reverse iterator for this ref, reading chunks from the back into the fixed buffer `buf`, in the given `store`.
    ///
    /// The iterator will traverse log entries from most recent to oldest, reading the underlying file in chunks from the back.
    /// Return `Ok(None)` if no reflog exists.
    pub fn log_iter_rev<'b>(
        &self,
        store: &file::Store,
        buf: &'b mut [u8],
    ) -> std::io::Result<Option<log::iter::Reverse<'b, std::fs::File>>> {
        store.reflog_iter_rev(self.name.as_ref(), buf).map_err(must_be_io_err)
    }

    /// Return a reflog forward iterator for this ref and write its file contents into `buf`, in the given `store`.
    ///
    /// The iterator will traverse log entries from oldest to newest.
    /// Return `Ok(None)` if no reflog exists.
    pub fn log_iter<'a, 'b: 'a>(
        &'a self,
        store: &file::Store,
        buf: &'b mut Vec<u8>,
    ) -> std::io::Result<Option<impl Iterator<Item = Result<log::LineRef<'b>, log::iter::decode::Error>> + 'a>> {
        store.reflog_iter(self.name.as_ref(), buf).map_err(must_be_io_err)
    }
}
