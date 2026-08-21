//! Monad execution-context shims for builds without Monad support.

#[derive(Clone)]
pub(super) struct ReplayContext;

pub(super) struct ExecutionContext<'a> {
    _context: &'a mut ReplayContext,
}

/// No-op Monad execution session for feature-independent backend paths.
#[derive(Clone, Default)]
pub(crate) struct ExecutionSession {
    _private: (),
}

impl ExecutionSession {
    pub(super) const fn new(_context: Option<ReplayContext>) -> Self {
        Self { _private: () }
    }

    pub(super) const fn next_transaction(&mut self) -> Option<ExecutionContext<'_>> {
        None
    }

    pub(super) const fn transaction_at(&self, _index: usize) -> Option<ExecutionContext<'static>> {
        None
    }

    pub(super) const fn advance_block(&mut self) {}
}
