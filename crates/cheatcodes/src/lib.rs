//! # foundry-cheatcodes
//!
//! Foundry cheatcodes implementations.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(elided_lifetimes_in_paths)] // Cheats context uses 3 lifetimes

#[macro_use]
extern crate foundry_common;

#[macro_use]
pub extern crate foundry_cheatcodes_spec as spec;

#[macro_use]
extern crate tracing;

use alloy_primitives::{Address, B256, U256};
use evm2::interpreter::GasTracker;
use foundry_evm_core::{
    backend::{Backend, CowBackend, LocalForkId},
    evm::{
        BlockEnvFor, EvmEnvFor, EvmFactoryFor, FoundryEvmFactory, FoundryEvmFor, FoundryEvmNetwork,
        SpecFor, evm_env,
    },
    fork::CreateFork,
};

pub use Vm::ForgeContext;
pub use config::CheatsConfig;
pub use error::{Error, ErrorKind, Result};
pub use inspector::{
    BroadcastableTransaction, BroadcastableTransactions, Cheatcodes, CheatcodesExecutor,
    StorageHook, message_result, revert_frame,
};
pub use spec::{CheatcodeDef, Vm};

#[macro_use]
mod error;

mod base64;

mod config;

mod crypto;

mod version;

mod env;
pub use env::{current_execution_context, set_execution_context};

mod evm;

mod expected_emit;

mod external_storage;

mod fs;

mod inspector;
pub use inspector::CheatcodeAnalysis;

mod json;

#[cfg(feature = "monad")]
mod monad;

mod script;
pub use script::{Wallets, WalletsInner};

mod state;
pub use state::CheatcodesCore;

mod string;

mod tempo;

mod test;
pub use test::{
    expect::ExpectedCallTracker,
    revert_handlers::{DUMMY_CALL_OUTPUT, DUMMY_CREATE_ADDRESS},
};

mod toml;

mod utils;

/// Cheatcode implementation.
pub(crate) trait Cheatcode: CheatcodeDef {
    /// Applies this cheatcode to the given state.
    ///
    /// Implement this function if you need cheatcode state but no access to the EVM context.
    fn apply_context(&self, _state: &mut CheatcodesCore) -> Result {
        self.apply_context_free()
    }

    /// Applies this cheatcode to the given context.
    ///
    /// Implement this function if you need access to the EVM data.
    #[inline(always)]
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        self.apply_context(&mut ccx.state.core)
    }

    /// Applies this cheatcode to the given context and executor.
    ///
    /// Implement this function if you need access to the executor.
    #[inline(always)]
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let _ = executor;
        self.apply_stateful(ccx)
    }

    /// Applies this cheatcode without accessing cheatcode state or the EVM context.
    ///
    /// This may still read or modify external host state, such as environment variables.
    fn apply_context_free(&self) -> Result {
        unimplemented!("{}", Self::CHEATCODE.func.id)
    }
}

/// Backend access of the cheatcode inspector while an execution runs.
///
/// Implemented for [`Backend`] and [`CowBackend`], so fuzzed calls keep their lazy copy-on-write
/// clone: read-only cheatcodes never clone the backend.
pub trait CheatcodesBackend<FEN: FoundryEvmNetwork> {
    /// Returns the backend for reading.
    fn backend(&self) -> &Backend<FEN>;

    /// Returns the backend for modification.
    fn backend_mut(&mut self) -> &mut Backend<FEN>;
}

impl<FEN: FoundryEvmNetwork> CheatcodesBackend<FEN> for Backend<FEN> {
    fn backend(&self) -> &Self {
        self
    }

    fn backend_mut(&mut self) -> &mut Self {
        self
    }
}

impl<FEN: FoundryEvmNetwork> CheatcodesBackend<FEN> for CowBackend<'_, FEN> {
    fn backend(&self) -> &Backend<FEN> {
        self
    }

    fn backend_mut(&mut self) -> &mut Backend<FEN> {
        self
    }
}

/// The cheatcode context.
pub struct CheatsCtxt<'a, 'evm, FEN: FoundryEvmNetwork> {
    /// The cheatcodes inspector state.
    pub(crate) state: &'a mut Cheatcodes<FEN>,
    /// The running EVM.
    pub(crate) evm: &'a mut FoundryEvmFor<'evm, FEN>,
    /// The backend the EVM runs over.
    pub(crate) backend: &'a mut dyn CheatcodesBackend<FEN>,
    /// The original `msg.sender`.
    pub(crate) caller: Address,
    /// Gas of the current cheatcode call, returned to the caller.
    pub(crate) gas: &'a mut GasTracker,
    /// Whether the current cheatcode call is static.
    pub(crate) is_static: bool,
    /// Call depth of the frame calling the cheatcode.
    pub(crate) depth: usize,
}

impl<'evm, FEN: FoundryEvmNetwork> CheatsCtxt<'_, 'evm, FEN> {
    pub(crate) fn ensure_not_precompile(&self, address: &Address) -> Result<()> {
        if self.is_precompile(address) { Err(precompile_error(address)) } else { Ok(()) }
    }

    /// Returns whether `address` is one of the EVM's registered precompiles.
    ///
    /// Checks the provider's registered addresses rather than `PrecompileProvider::contains`:
    /// networks such as Tempo also dispatch storage-backed system contracts as precompiles, and
    /// cheatcodes like `vm.store` must keep working on their state.
    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.evm.precompiles().addresses().contains(address)
    }

    /// Returns the backend.
    #[inline]
    pub(crate) fn db(&self) -> &Backend<FEN> {
        self.backend.backend()
    }

    /// Returns the backend for modification.
    #[inline]
    pub(crate) fn db_mut(&mut self) -> &mut Backend<FEN> {
        self.backend.backend_mut()
    }

    /// Returns the running EVM and the backend for modification.
    #[inline]
    pub(crate) fn evm_db_mut(&mut self) -> (&mut FoundryEvmFor<'evm, FEN>, &mut Backend<FEN>) {
        (&mut *self.evm, self.backend.backend_mut())
    }

    /// Returns the current call depth.
    #[inline]
    pub(crate) const fn depth(&self) -> usize {
        self.depth
    }

    /// Returns the depth of the frame that called the cheatcode, whose scope its writes join.
    #[inline]
    pub(crate) const fn caller_depth(&self) -> usize {
        self.depth.saturating_sub(1)
    }

    /// Returns the active hardfork: the one the calls and creates started next run with.
    ///
    /// Cheatcode calls run on the running EVM, which keeps its spec while a change requested by
    /// `vm.setEvmVersion` or deferred by an in-flight operation applies to new frames.
    #[inline]
    pub(crate) fn spec(&self) -> SpecFor<FEN> {
        self.state
            .execution_evm_version
            .or_else(|| self.db().lifecycle().pending_env().map(|env| env.spec))
            .unwrap_or_else(|| self.evm.config_spec_id())
    }

    /// Returns the running environment with [`Self::spec`], for transactions a cheatcode executes
    /// with the requested spec.
    pub(crate) fn evm_env(&self) -> EvmEnvFor<FEN> {
        let env = evm_env(self.evm);
        let spec = self.spec();
        if spec == env.spec {
            return env;
        }
        EvmFactoryFor::<FEN>::default().env_with_spec(env, spec)
    }

    /// Returns the chain ID.
    #[inline]
    pub(crate) fn chain_id(&self) -> u64 {
        self.state
            .env_overrides
            .get(self.active_fork_id())
            .and_then(|overrides| overrides.chain_id)
            .unwrap_or(self.evm.version().chain_id)
    }

    /// Returns the block number.
    #[inline]
    pub(crate) const fn block_number(&self) -> U256 {
        self.evm.block().number
    }

    /// Returns the block timestamp.
    #[inline]
    pub(crate) const fn timestamp(&self) -> U256 {
        self.evm.block().timestamp
    }

    /// Returns the block base fee.
    #[inline]
    pub(crate) fn basefee(&self) -> u64 {
        self.evm.block().basefee.saturating_to()
    }

    /// Returns the block slot number.
    #[inline]
    pub(crate) fn slot_num(&self) -> u64 {
        self.evm.block().slot_num.saturating_to()
    }

    /// Updates the block environment of the running EVM.
    #[inline]
    pub(crate) fn update_block(&mut self, f: impl FnOnce(&mut BlockEnvFor<FEN>)) {
        let mut block = *self.evm.block();
        f(&mut block);
        self.evm.set_block(block);
    }

    /// Returns the transaction origin (`tx.origin`), including prank and broadcast overrides.
    #[inline]
    pub(crate) fn tx_caller(&self) -> Address {
        self.state.origin_override.unwrap_or(self.evm.tx_env().origin)
    }

    /// Returns the transaction blob versioned hashes.
    #[inline]
    pub(crate) fn tx_blob_hashes(&self) -> Vec<B256> {
        self.evm.tx_env().blob_hashes.iter().map(|hash| B256::from(*hash)).collect()
    }

    /// Returns the active fork ID, if any.
    #[inline]
    pub(crate) fn active_fork_id(&self) -> Option<LocalForkId> {
        self.db().active_fork_id()
    }

    /// Returns the active fork URL, if any.
    #[inline]
    pub(crate) fn active_fork_url(&self) -> Option<String> {
        self.db().active_fork_url()
    }

    /// Returns the active fork block number, if any.
    #[inline]
    pub(crate) fn active_fork_block_number(&self) -> Option<u64> {
        self.db().active_fork_block_number()
    }

    /// Returns the active fork options, if any.
    #[inline]
    pub(crate) fn active_fork_options(&self) -> Option<CreateFork> {
        self.db().active_fork_options()
    }

    /// Returns whether a fork is active.
    #[inline]
    pub(crate) fn is_forked_mode(&self) -> bool {
        self.db().is_forked_mode()
    }

    /// Returns whether `account` persists across forks.
    #[inline]
    pub(crate) fn is_persistent(&self, account: &Address) -> bool {
        self.db().is_persistent(account)
    }
}

#[cold]
fn precompile_error(address: &Address) -> Error {
    fmt_err!("cannot use precompile {address} as an argument")
}
