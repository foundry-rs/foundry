//! evm2 successor of the [`crate::evm`] factory traits.
//!
//! [`FoundryEvmNetwork`] and [`FoundryEvmFactory`] keep the names, roles, and type aliases of their
//! REVM counterparts so callers can move by path. They are implemented for the same network
//! markers, such as [`EthEvmNetwork`](crate::evm::EthEvmNetwork). The factory no longer prescribes
//! a mutable REVM context: every network executes on [`evm2::Evm`] with its own type family, behind
//! [`EvmTypesHost`].
//!
//! | REVM-era item                                   | evm2 successor                                   |
//! |-------------------------------------------------|--------------------------------------------------|
//! | `EvmFactory::{Spec, BlockEnv, Tx, HaltReason}`  | [`FoundryEvmFactory::Types`] (`EvmTypesHost`)    |
//! | `PrecompilesMap`                                | [`FoundryEvmFactory::precompiles`]               |
//! | `FoundryContextExt` and `CfgEnv`                | [`evm2::Evm`], [`EvmEnv`] and [`evm2::Version`]  |
//! | `FoundryTransaction` on network `TxEnv`s        | [`FoundryTxEnv`] + [`FoundryEvmFactory::build_tx`] |
//! | `FoundryChain`                                  | `EvmTypesHost::EvmExt` + [`FoundryEvmExt`]       |
//! | `FoundryJournal` / `JournaledState`             | evm2 `State`, reached through [`evm2::Evm`]      |
//! | `NestedEvm` trait objects                       | [`evm2::Evm`], identical for every network       |
//! | `IntoInstructionResult` for halt reasons        | `TxResult::stop` + [`FoundryTxResultExt`]        |
//! | `DatabaseExt` as seen by the factory            | [`FoundryDatabase`]                              |

use alloy_consensus::{
    SignableTransaction, Signed,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_eips::Typed2718;
use alloy_network::Network;
use alloy_primitives::Signature;
use alloy_rlp::Decodable;
use evm2::{
    Evm, EvmTypes, EvmTypesHost, ExecutionConfig, TxResult, Version,
    env::BlockEnv,
    evm::{DynDatabase, EmptyDB, InMemoryDB, precompile::PrecompileProvider},
    registry::{HandlerError, TxRegistry},
};
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_evm_hardforks::ExecutionSpec;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

mod env;
pub use env::*;

mod inspector;
pub use inspector::FoundryInspectorExt;

mod tx;
pub use tx::*;

#[cfg(feature = "base")]
pub mod base;
pub mod eth;
#[cfg(feature = "monad")]
pub mod monad;
#[cfg(feature = "optimism")]
pub mod op;
pub mod tempo;

pub use eth::*;
pub use tempo::*;

#[cfg(test)]
mod tests;

/// Successor of [`crate::evm::FoundryEvmNetwork`]: binds an Alloy network to an evm2 factory.
pub trait FoundryEvmNetwork: Copy + Debug + Default + 'static {
    /// Alloy network used for RPC, signing and broadcasting.
    type Network: Network<
            TxEnvelope: Decodable
                            + SignerRecoverable
                            + From<Signed<<Self::Network as Network>::UnsignedTx>>
                            + for<'d> Deserialize<'d>
                            + Serialize
                            + UIfmt,
            UnsignedTx: SignableTransaction<Signature>,
            TransactionRequest: FoundryTransactionBuilder<Self::Network>
                                    + for<'d> Deserialize<'d>
                                    + Serialize,
            ReceiptResponse: FoundryReceiptResponse,
        >;

    /// evm2 factory executing this network.
    type EvmFactory: FoundryEvmFactory;
}

/// Successor of [`crate::evm::FoundryEvmFactory`], built around an evm2 [`EvmTypesHost`] family.
///
/// Implementors provide configuration (version, registry, precompiles, extension state) and
/// transaction construction. Construction and spec changes are provided on top, so every network
/// builds its EVM the same way.
pub trait FoundryEvmFactory: Clone + Debug + Default + Send + Sync + 'static {
    /// evm2 type family: spec, transaction, block, EVM and result extensions.
    type Types: EvmTypes
        + EvmTypesHost<
            SpecId: ExecutionSpec + Default + Debug + Send + Sync + Unpin,
            Tx: Typed2718 + Clone + Debug + Send + Sync,
            BlockEnvExt: FoundryBlockExt,
            EvmExt: FoundryEvmExt<Self::TxExt>,
            TxResultExt: FoundryTxResultExt,
        >;

    /// Network-specific fields of [`FoundryTxEnv`].
    type TxExt: FoundryTxExt;

    /// Returns the default runtime version for `spec` on `chain_id`.
    fn version(&self, spec: SpecOf<Self>, chain_id: u64) -> Version;

    /// Returns the execution config (dispatch tables) for `spec` with runtime `version`.
    fn execution_config(
        &self,
        spec: SpecOf<Self>,
        version: Version,
    ) -> ExecutionConfig<Self::Types> {
        ExecutionConfig::for_spec_and_version(spec, version)
    }

    /// Returns the transaction handler registry for `spec`.
    fn tx_registry(&self, spec: SpecOf<Self>) -> TxRegistry<Self::Types, TxResult<Self::Types>>;

    /// Returns the precompiles for `env`, sharing any accounting state held by `ext`.
    fn precompiles(
        &self,
        env: &EvmEnv<Self::Types>,
        ext: &<Self::Types as EvmTypesHost>::EvmExt,
    ) -> Box<dyn PrecompileProvider<Self::Types>>;

    /// Returns fresh extension state for an EVM built with `env`.
    fn evm_ext(&self, _env: &EvmEnv<Self::Types>) -> <Self::Types as EvmTypesHost>::EvmExt {
        Default::default()
    }

    /// Builds the network transaction described by `tx`, recovered to its caller.
    fn build_tx(
        &self,
        tx: &FoundryTxEnv<Self::TxExt>,
    ) -> eyre::Result<Recovered<<Self::Types as EvmTypesHost>::Tx>>;

    /// Returns this factory configured for an EVM over `db`, such as Base's activation admin for
    /// the fork's source chain.
    fn configure_for<D: FoundryDatabase + ?Sized>(
        &self,
        _db: &D,
        _env: &EvmEnv<Self::Types>,
    ) -> Self {
        self.clone()
    }

    /// Seeds network state into a newly created EVM, such as Tempo genesis or Base sentinels.
    fn initialize_evm(&self, _evm: &mut Evm<'_, Self::Types>, _is_forked: bool) {}

    /// Creates a Foundry-configured EVM without an inspector.
    fn create_evm<'a>(
        &self,
        db: impl FoundryDatabase + 'a,
        env: EvmEnv<Self::Types>,
    ) -> Evm<'a, Self::Types> {
        let factory = self.configure_for(&db, &env);
        let is_forked = db.is_forked_mode();
        let ext = factory.evm_ext(&env);
        let precompiles = factory.precompiles(&env, &ext);
        let EvmEnv { spec, version, block } = env;
        let mut evm = Evm::new_with_execution_config_and_ext(
            factory.execution_config(spec, version),
            spec,
            block,
            factory.tx_registry(spec),
            db,
            precompiles,
            ext,
        );
        factory.initialize_evm(&mut evm, is_forked);
        evm
    }

    /// Creates a Foundry-configured EVM with the given inspector.
    fn create_foundry_evm_with_inspector<'a, I: FoundryInspectorExt<Self::Types> + 'a>(
        &self,
        db: impl FoundryDatabase + 'a,
        env: EvmEnv<Self::Types>,
        inspector: I,
    ) -> Evm<'a, Self::Types> {
        let mut evm = self.create_evm(db, env);
        evm.set_inspector(inspector);
        evm
    }

    /// Creates a nested EVM without an inspector.
    ///
    /// Nested EVMs are ordinary [`Evm`]s; the boxed `NestedEvm` adapter is no longer needed.
    fn create_nested_evm<'a>(
        &self,
        db: impl FoundryDatabase + 'a,
        env: EvmEnv<Self::Types>,
    ) -> Evm<'a, Self::Types> {
        self.create_evm(db, env)
    }

    /// Creates a nested EVM with the given inspector.
    fn create_nested_evm_with_inspector<'a, I: FoundryInspectorExt<Self::Types> + 'a>(
        &self,
        db: impl FoundryDatabase + 'a,
        env: EvmEnv<Self::Types>,
        inspector: I,
    ) -> Evm<'a, Self::Types> {
        self.create_foundry_evm_with_inspector(db, env, inspector)
    }

    /// Switches `evm` to `spec`, keeping Foundry's execution toggles and limits.
    ///
    /// Unlike the REVM version, this rebuilds the dispatch tables, the transaction registry and the
    /// precompiles, so opcode and precompile availability follows the new spec. It must not be
    /// called while `evm` is executing.
    fn set_execution_spec(&self, evm: &mut Evm<'_, Self::Types>, spec: SpecOf<Self>) {
        let current = *evm.version();
        let current_default = self.version(evm.config_spec_id(), current.chain_id);
        let next = self.version(spec, current.chain_id);
        let version = carry_execution_policy(&current_default, &current, next);
        let env = EvmEnv::new(spec, version, *evm.block());
        let precompiles = self.precompiles(&env, evm.ext());
        evm.set_execution_config(
            self.execution_config(spec, version),
            spec,
            self.tx_registry(spec),
            precompiles,
        );
    }
}

/// Returns the environment `evm` currently executes with.
pub const fn evm_env<T: EvmTypes>(evm: &Evm<'_, T>) -> EvmEnv<T> {
    EvmEnv::new(evm.config_spec_id(), *evm.version(), *evm.block())
}

/// Database view the factory needs at construction time.
///
/// This is the slice of [`DatabaseExt`](crate::backend::DatabaseExt) that EVM construction reads;
/// fork and snapshot ownership move to the accepted-state backend separately.
pub trait FoundryDatabase: DynDatabase {
    /// Whether the database is currently in forked mode.
    fn is_forked_mode(&self) -> bool {
        false
    }

    /// Returns the source chain ID of the active fork, independent of execution overrides.
    fn active_fork_source_chain_id(&self) -> Option<u64> {
        None
    }
}

impl<D: FoundryDatabase + ?Sized> FoundryDatabase for &mut D {
    fn is_forked_mode(&self) -> bool {
        (**self).is_forked_mode()
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        (**self).active_fork_source_chain_id()
    }
}

impl FoundryDatabase for EmptyDB {}

impl FoundryDatabase for InMemoryDB {}

/// Convenience type aliases for accessing associated types through [`FoundryEvmNetwork`].
pub type EvmFactoryFor<FEN> = <FEN as FoundryEvmNetwork>::EvmFactory;
pub type TypesFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::Types;
pub type SpecOf<F> = <<F as FoundryEvmFactory>::Types as EvmTypesHost>::SpecId;
pub type SpecFor<FEN> = <TypesFor<FEN> as EvmTypesHost>::SpecId;
pub type BlockEnvFor<FEN> = BlockEnv<TypesFor<FEN>>;
pub type TxExtFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::TxExt;
pub type TxEnvFor<FEN> = FoundryTxEnv<TxExtFor<FEN>>;
pub type EvmEnvFor<FEN> = EvmEnv<TypesFor<FEN>>;
pub type ChainFor<FEN> = <TypesFor<FEN> as EvmTypesHost>::EvmExt;
pub type TxResultFor<FEN> = TxResult<TypesFor<FEN>>;
pub type FoundryEvmFor<'a, FEN> = Evm<'a, TypesFor<FEN>>;
pub type NetworkFor<FEN> = <FEN as FoundryEvmNetwork>::Network;
pub type TxEnvelopeFor<FEN> = <NetworkFor<FEN> as Network>::TxEnvelope;
pub type TransactionRequestFor<FEN> = <NetworkFor<FEN> as Network>::TransactionRequest;
pub type TransactionResponseFor<FEN> = <NetworkFor<FEN> as Network>::TransactionResponse;
pub type BlockResponseFor<FEN> = <NetworkFor<FEN> as Network>::BlockResponse;

/// Closure type used by `CheatcodesExecutor` methods that run nested EVM operations.
pub type NestedEvmClosure<'a, F> = &'a mut dyn for<'e> FnMut(
    &mut Evm<'e, <F as FoundryEvmFactory>::Types>,
) -> Result<(), HandlerError>;

/// Nested EVM closure for a Foundry EVM network.
pub type NestedEvmClosureFor<'a, FEN> = NestedEvmClosure<'a, EvmFactoryFor<FEN>>;
