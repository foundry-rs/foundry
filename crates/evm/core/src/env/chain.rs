//! Chain context construction and lifecycle operations.

use std::fmt::Debug;

#[cfg(feature = "monad")]
use super::FoundryJournal;

/// Foundry extension for chain context type
///
/// Every family that doesn't need chain metadata uses `()`.
pub trait FoundryChain<Tx>: Clone + Debug + Default + Send + Sync {
    /// Builds chain context for a standalone synthetic transaction.
    fn for_transaction(_tx: &Tx) -> Self {
        Self::default()
    }

    /// Builds chain context for a transaction at an exact block position.
    fn for_block(
        _grandparent: &[Tx],
        _parent: &[Tx],
        _current: &[Tx],
        _current_tx_index: usize,
    ) -> Self {
        Self::default()
    }

    /// Refreshes journal state derived from the active chain position.
    #[cfg(feature = "monad")]
    fn refresh_journal<J: FoundryJournal>(&self, _journal: &mut J) {}

    /// Clears cached protocol fees after a synthetic transaction restores chain context.
    fn clear_transaction_fee_cache(&mut self) {}
}

impl<Tx> FoundryChain<Tx> for () {}
