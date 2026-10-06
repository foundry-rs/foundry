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
use foundry_evm_core::{
    FoundryTransaction,
    backend::{DatabaseExt, LocalForkId},
    env::FoundryContextExt,
    evm::{FoundryContextFor, FoundryEvmNetwork, SpecFor},
    fork::CreateFork,
};
use revm::context::{Block, Cfg, ContextTr, JournalTr, Transaction};

pub use Vm::ForgeContext;
pub use config::CheatsConfig;
pub use error::{Error, ErrorKind, Result};
pub use foundry_evm_core::evm::NestedEvmClosureFor;
pub use inspector::{
    BroadcastableTransaction, BroadcastableTransactions, Cheatcodes, CheatcodesExecutor,
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

mod string;

mod tempo;

mod test;
pub use test::expect::ExpectedCallTracker;

mod toml;

mod utils;

/// Cheatcode implementation.
pub(crate) trait Cheatcode: CheatcodeDef {
    /// Applies this cheatcode to the given state.
    ///
    /// Implement this function if you don't need access to the EVM data.
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let _ = state;
        unimplemented!("{}", Self::CHEATCODE.func.id)
    }

    /// Applies this cheatcode to the given context.
    ///
    /// Implement this function if you need access to the EVM data.
    #[inline(always)]
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        self.apply(ccx.state)
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
}

/// The cheatcode context.
pub struct CheatsCtxt<'a, 'db, FEN: FoundryEvmNetwork + 'db> {
    /// The cheatcodes inspector state.
    pub(crate) state: &'a mut Cheatcodes<FEN>,
    /// The EVM context.
    pub(crate) ecx: &'a mut FoundryContextFor<'db, FEN>,
    /// The original `msg.sender`.
    pub(crate) caller: Address,
    /// Gas limit of the current cheatcode call.
    pub(crate) gas_limit: u64,
    /// Whether the current cheatcode call is static.
    pub(crate) is_static: bool,
}

impl<FEN: FoundryEvmNetwork> CheatsCtxt<'_, '_, FEN> {
    pub(crate) fn ensure_not_precompile(&self, address: &Address) -> Result<()> {
        if self.is_precompile(address) { Err(precompile_error(address)) } else { Ok(()) }
    }

    pub(crate) fn is_precompile(&self, address: &Address) -> bool {
        self.ecx.journal().precompile_addresses().contains(address)
    }

    /// Returns the current call depth.
    #[inline]
    pub(crate) fn depth(&self) -> usize {
        self.ecx.journal().depth()
    }

    /// Returns the active hardfork.
    #[inline]
    pub(crate) fn spec(&self) -> SpecFor<FEN> {
        self.ecx.cfg().spec()
    }

    /// Returns the chain ID.
    #[inline]
    pub(crate) fn chain_id(&self) -> u64 {
        self.ecx.cfg().chain_id()
    }

    /// Returns the maximum initcode size.
    #[inline]
    pub(crate) fn max_initcode_size(&self) -> usize {
        self.ecx.cfg().max_initcode_size()
    }

    /// Returns the configured contract code size limit, if any.
    #[inline]
    pub(crate) fn limit_contract_code_size(&self) -> Option<usize> {
        self.ecx.cfg_env().limit_contract_code_size
    }

    /// Returns the block number.
    #[inline]
    pub(crate) fn block_number(&self) -> U256 {
        self.ecx.block().number()
    }

    /// Returns the block timestamp.
    #[inline]
    pub(crate) fn timestamp(&self) -> U256 {
        self.ecx.block().timestamp()
    }

    /// Returns the block base fee.
    #[inline]
    pub(crate) fn basefee(&self) -> u64 {
        self.ecx.block().basefee()
    }

    /// Returns the block slot number.
    #[inline]
    pub(crate) fn slot_num(&self) -> u64 {
        self.ecx.block().slot_num()
    }

    /// Returns the block excess blob gas, if any.
    #[inline]
    pub(crate) fn blob_excess_gas(&self) -> Option<u64> {
        self.ecx.block().blob_excess_gas()
    }

    /// Returns the transaction caller (`tx.origin`).
    #[inline]
    pub(crate) fn tx_caller(&self) -> Address {
        self.ecx.tx().caller()
    }

    /// Returns the transaction gas price.
    #[inline]
    pub(crate) fn tx_gas_price(&self) -> u128 {
        self.ecx.tx().gas_price()
    }

    /// Returns the transaction type.
    #[inline]
    pub(crate) fn tx_type(&self) -> u8 {
        self.ecx.tx().tx_type()
    }

    /// Returns the transaction blob versioned hashes.
    #[inline]
    pub(crate) fn tx_blob_hashes(&self) -> &[B256] {
        self.ecx.tx().blob_versioned_hashes()
    }

    /// Returns the transaction fee token, if any.
    #[inline]
    pub(crate) fn tx_fee_token(&self) -> Option<Address> {
        self.ecx.tx().fee_token()
    }

    /// Returns the active fork ID, if any.
    #[inline]
    pub(crate) fn active_fork_id(&self) -> Option<LocalForkId> {
        self.ecx.db().active_fork_id()
    }

    /// Returns the active fork URL, if any.
    #[inline]
    pub(crate) fn active_fork_url(&self) -> Option<String> {
        self.ecx.db().active_fork_url()
    }

    /// Returns the active fork block number, if any.
    #[inline]
    pub(crate) fn active_fork_block_number(&self) -> Option<u64> {
        self.ecx.db().active_fork_block_number()
    }

    /// Returns the active fork options, if any.
    #[inline]
    pub(crate) fn active_fork_options(&self) -> Option<CreateFork> {
        self.ecx.db().active_fork_options()
    }

    /// Returns whether a fork is active.
    #[inline]
    pub(crate) fn is_forked_mode(&self) -> bool {
        self.ecx.db().is_forked_mode()
    }

    /// Returns whether `account` persists across forks.
    #[inline]
    pub(crate) fn is_persistent(&self, account: &Address) -> bool {
        self.ecx.db().is_persistent(account)
    }
}

#[cold]
fn precompile_error(address: &Address) -> Error {
    fmt_err!("cannot use precompile {address} as an argument")
}
