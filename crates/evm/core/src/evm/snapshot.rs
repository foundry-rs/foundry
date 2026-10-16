//! User-visible state snapshots.

use super::{EvmEnv, ForkRegistry, SendableExt, StoredExt};
use evm2::{EvmTypesHost, evm::StateSnapshot};
use std::fmt;

/// Everything a snapshot revert restores.
///
/// evm2's [`StateSnapshot`] covers the running context's accepted and in-flight state but not its
/// backing database; the fork registry supplies that, along with every inactive fork's accepted
/// state. Cheatcode state rides along as `C`.
pub struct LifecycleSnapshot<T: EvmTypesHost, C> {
    /// The running context's state.
    pub(super) state: StateSnapshot,
    /// Forks, persistent accounts and the active selection.
    pub(super) forks: ForkRegistry<T>,
    /// Environment of the running context.
    pub(super) env: EvmEnv<T>,
    /// Network extension state of the running context.
    pub(super) ext: StoredExt<T::EvmExt>,
    /// Caller-owned state, such as cheatcode state.
    pub(super) extra: C,
}

impl<T: EvmTypesHost<EvmExt: SendableExt>, C: Clone> Clone for LifecycleSnapshot<T, C> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            forks: self.forks.clone(),
            env: self.env,
            ext: self.ext.clone(),
            extra: self.extra.clone(),
        }
    }
}

impl<T: EvmTypesHost, C> fmt::Debug for LifecycleSnapshot<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LifecycleSnapshot").field("forks", &self.forks).finish_non_exhaustive()
    }
}
