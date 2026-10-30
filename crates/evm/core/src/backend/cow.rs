//! A clone-on-write wrapper around [`Backend`] used for fuzzing.

use crate::{
    backend::Backend,
    evm::{FoundryEvmFor, FoundryEvmNetwork},
};
use alloy_primitives::{Address, TxKind};
use std::{
    borrow::Cow,
    fmt::Debug,
    ops::{Deref, DerefMut},
};

/// A clone-on-write [`Backend`].
///
/// Reads and executions that discard their state ([`Backend::create_evm`]) borrow the wrapped
/// backend. The first mutable access, such as a
/// cheatcode that switches forks, clones it and applies the pending test initialization, so the
/// original backend is never modified.
///
/// Entire purpose of this type is for fuzzing. A test function fuzzer will repeatedly execute the
/// function via immutable raw (no state changes) calls.
///
/// **N.B.**: we're assuming cheatcodes that alter the state (like multi fork swapping) are niche.
/// If they executed, it will require a clone of the initial input database.
/// This way we can support these cheatcodes cheaply without adding overhead for tests that
/// don't make use of them. Alternatively each test case would require its own `Backend` clone,
/// which would add significant overhead for large fuzz sets even if the Database is not big after
/// setup.
pub struct CowBackend<'a, FEN: FoundryEvmNetwork> {
    /// The underlying `Backend`.
    ///
    /// No calls on the `CowBackend` will ever persistently modify the `backend`'s state.
    pub backend: Cow<'a, Backend<FEN>>,
    /// Test initialization applied on first mutable access.
    pending_init: Option<(Address, TxKind)>,
}

impl<FEN: FoundryEvmNetwork> Clone for CowBackend<'_, FEN> {
    fn clone(&self) -> Self {
        Self { backend: self.backend.clone(), pending_init: self.pending_init }
    }
}

impl<FEN: FoundryEvmNetwork> Debug for CowBackend<'_, FEN> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CowBackend")
            .field("backend", &self.backend)
            .field("pending_init", &self.pending_init)
            .finish()
    }
}

impl<'a, FEN: FoundryEvmNetwork> CowBackend<'a, FEN> {
    /// Creates a new `CowBackend` with the given `Backend`.
    pub const fn new_borrowed(backend: &'a Backend<FEN>) -> Self {
        Self { backend: Cow::Borrowed(backend), pending_init: None }
    }

    /// Records the caller and test contract of the next execution, applied with
    /// [`Backend::initialize`] on first mutable access.
    pub const fn initialize(&mut self, caller: Address, kind: TxKind) {
        self.pending_init = Some((caller, kind));
    }

    /// Returns whether the backend was cloned.
    pub const fn is_owned(&self) -> bool {
        matches!(self.backend, Cow::Owned(_))
    }

    /// Parks accepted state on owned backends; leaves pending writes uncommitted.
    pub fn park_detached(&mut self, evm: FoundryEvmFor<'_, FEN>) {
        if let Cow::Owned(backend) = &mut self.backend {
            backend.lifecycle.park_detached(evm);
            backend.bal = None;
        }
    }
}

impl<FEN: FoundryEvmNetwork> Deref for CowBackend<'_, FEN> {
    type Target = Backend<FEN>;

    fn deref(&self) -> &Self::Target {
        &self.backend
    }
}

impl<FEN: FoundryEvmNetwork> DerefMut for CowBackend<'_, FEN> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let backend = self.backend.to_mut();
        if let Some((caller, kind)) = self.pending_init.take() {
            backend.initialize(caller, kind);
        }
        backend
    }
}
