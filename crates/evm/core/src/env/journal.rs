//! Access to execution journals and their native state.

use crate::backend::JournaledState;
use revm::{Database, Journal, inspector::JournalExt};

/// Foundry extension for Journal type
pub trait FoundryJournal: JournalExt {
    /// Mutable access to the database and journal inner.
    fn db_journal_inner_mut(&mut self) -> (&mut Self::Database, &mut JournaledState);

    /// Reference to the journal inner.
    fn journal_inner(&self) -> &JournaledState;

    /// Captures Monad's reserve-balance tracker for the active transaction.
    #[cfg(feature = "monad")]
    fn capture_reserve_balance(
        &self,
    ) -> monad_revm::reserve_balance::tracker::ReserveBalanceTracker {
        monad_revm::reserve_balance::tracker::ReserveBalanceTracker::default()
    }

    /// Restores Monad's reserve-balance tracker for the active transaction.
    #[cfg(feature = "monad")]
    fn restore_reserve_balance(
        &mut self,
        _tracker: monad_revm::reserve_balance::tracker::ReserveBalanceTracker,
    ) {
    }

    /// Whether transaction boundaries currently preserve the reserve-balance tracker, e.g. for
    /// an isolated call that models an inner call of the enclosing transaction rather than a
    /// new one.
    #[cfg(feature = "monad")]
    fn preserves_reserve_balance(&self) -> bool {
        false
    }

    /// Sets whether transaction boundaries preserve the reserve-balance tracker.
    #[cfg(feature = "monad")]
    fn set_preserve_reserve_balance(&mut self, _preserve: bool) {}
}

impl<DB: Database> FoundryJournal for Journal<DB> {
    fn db_journal_inner_mut(&mut self) -> (&mut DB, &mut JournaledState) {
        (&mut self.database, &mut self.inner)
    }

    fn journal_inner(&self) -> &JournaledState {
        &self.inner
    }
}

#[cfg(feature = "monad")]
impl<DB: Database> FoundryJournal for monad_revm::MonadJournal<DB> {
    fn db_journal_inner_mut(&mut self) -> (&mut DB, &mut JournaledState) {
        Journal::db_journal_inner_mut(self)
    }

    fn journal_inner(&self) -> &JournaledState {
        Journal::journal_inner(self)
    }

    fn capture_reserve_balance(
        &self,
    ) -> monad_revm::reserve_balance::tracker::ReserveBalanceTracker {
        monad_revm::MonadJournalTr::reserve_balance(self).clone()
    }

    fn restore_reserve_balance(
        &mut self,
        tracker: monad_revm::reserve_balance::tracker::ReserveBalanceTracker,
    ) {
        *monad_revm::MonadJournalTr::reserve_balance_mut(self) = tracker;
    }

    fn preserves_reserve_balance(&self) -> bool {
        monad_revm::MonadJournalTr::preserves_reserve_balance_tracker(self)
    }

    fn set_preserve_reserve_balance(&mut self, preserve: bool) {
        monad_revm::MonadJournalTr::set_preserve_reserve_balance_tracker(self, preserve);
    }
}
