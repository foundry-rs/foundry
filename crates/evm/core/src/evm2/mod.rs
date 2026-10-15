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
//! | `FoundryTransaction` on network `TxEnv`s        | native requests + [`FoundryEvmNetwork::evm_tx`]  |
//! | `FoundryChain`                                  | `EvmTypesHost::EvmExt` + [`FoundryEvmExt`]       |
//! | `FoundryJournal` / `JournaledState`             | evm2 `State`, reached through [`evm2::Evm`]      |
//! | `NestedEvm` trait objects                       | [`ExecutionLifecycle::nested_transaction`]       |
//! | `IntoInstructionResult` for halt reasons        | `TxResult::stop` + [`FoundryTxResultExt`]        |
//! | `DatabaseExt` reads                             | [`FoundryDatabase`] and [`BackingDb`]            |
//! | `DatabaseExt` forks and persistent accounts     | [`ForkRegistry`]                                 |
//! | `DatabaseExt` snapshots and nested execution    | [`ExecutionLifecycle`] and [`nested_frame`]      |

use alloy_consensus::{
    SignableTransaction, Signed,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_eips::{Decodable2718, Typed2718};
use alloy_network::{Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, Signature};
use alloy_rlp::Decodable;
use evm2::{
    Evm, EvmTypes, EvmTypesHost, ExecutionConfig, TxResult, Version,
    env::BlockEnv,
    evm::precompile::PrecompileProvider,
    registry::{HandlerError, TxRegistry},
};
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_evm_hardforks::ExecutionSpec;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

mod db;
pub use db::*;

mod env;
pub use env::*;

mod fork;
pub use fork::*;

mod lifecycle;
pub use lifecycle::*;

mod snapshot;
pub use snapshot::*;

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

#[cfg(test)]
mod tests;

/// Successor of [`crate::evm::FoundryEvmNetwork`]: binds an Alloy network to an evm2 factory.
///
/// The network owns the only conversion from its native Alloy transactions to its evm2
/// transaction, so requests and signed envelopes stay network-native until execution.
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

    /// Converts a network envelope sent by `sender` into the evm2 transaction, once, right before
    /// execution.
    fn evm_tx(
        envelope: TxEnvelopeFor<Self>,
        sender: Address,
        payload: TxPayload,
    ) -> eyre::Result<Recovered<TxFor<Self>>>;

    /// Converts a complete unsigned request into a simulated evm2 transaction.
    ///
    /// The request must name its sender and carry every field its transaction type needs; Foundry
    /// fills defaults through [`TransactionBuilder`] before converting, never afterwards.
    fn tx_from_request(
        request: TransactionRequestFor<Self>,
    ) -> eyre::Result<Recovered<TxFor<Self>>> {
        let sender =
            request.from().ok_or_else(|| eyre::eyre!("transaction request has no sender"))?;
        let unsigned =
            request.build_unsigned().map_err(|err| eyre::eyre!(err.error.to_string()))?;
        let envelope = TxEnvelopeFor::<Self>::from(unsigned.into_signed(placeholder_signature()));
        Self::evm_tx(envelope, sender, TxPayload::Simulated)
    }

    /// Converts a signed envelope into an evm2 transaction sent by its recovered signer.
    fn tx_from_signed(envelope: TxEnvelopeFor<Self>) -> eyre::Result<Recovered<TxFor<Self>>> {
        let sender = envelope.recover_signer()?;
        Self::evm_tx(envelope, sender, TxPayload::Signed)
    }

    /// Decodes exactly the EIP-2718 bytes that were signed and converts them for execution.
    fn decode_tx(mut bytes: &[u8]) -> eyre::Result<Recovered<TxFor<Self>>> {
        let envelope = TxEnvelopeFor::<Self>::decode_2718(&mut bytes)?;
        eyre::ensure!(bytes.is_empty(), "trailing bytes after the encoded transaction");
        Self::tx_from_signed(envelope)
    }
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
            SpecId: ExecutionSpec + Default + Debug + PartialEq + Send + Sync + Unpin,
            Tx: Typed2718 + Clone + Debug + Send + Sync,
            BlockEnvExt: FoundryBlockExt,
            EvmExt: FoundryEvmExt<<Self::Types as EvmTypesHost>::Tx>,
            TxResultExt: FoundryTxResultExt,
        >;

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
        let mut evm = factory.new_evm(db, env);
        factory.initialize_evm(&mut evm, is_forked);
        evm
    }

    /// Creates a Foundry-configured EVM without seeding network state.
    ///
    /// Use this when the EVM's state is replaced right away, as for nested transactions.
    fn new_evm<'a>(
        &self,
        db: impl FoundryDatabase + 'a,
        env: EvmEnv<Self::Types>,
    ) -> Evm<'a, Self::Types> {
        let factory = self.configure_for(&db, &env);
        let ext = factory.evm_ext(&env);
        let precompiles = factory.precompiles(&env, &ext);
        let EvmEnv { spec, version, block } = env;
        Evm::new_with_execution_config_and_ext(
            factory.execution_config(spec, version),
            spec,
            block,
            factory.tx_registry(spec),
            db,
            precompiles,
            ext,
        )
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
        let env = EvmEnv::new(spec, *evm.version(), *evm.block());
        self.set_env(evm, env);
    }

    /// Switches `evm` to `env`: its block, spec and chain ID, keeping Foundry's execution toggles
    /// and limits.
    ///
    /// Like [`Self::set_execution_spec`], this must not be called while `evm` is executing.
    fn set_env(&self, evm: &mut Evm<'_, Self::Types>, env: EvmEnv<Self::Types>) {
        let current = *evm.version();
        let current_default = self.version(evm.config_spec_id(), current.chain_id);
        let next = self.version(env.spec, env.chain_id());
        let mut version = carry_execution_policy(&current_default, &current, next);
        version.chain_id = env.chain_id();
        let EvmEnv { spec, block, .. } = env;
        let precompiles = self.precompiles(&EvmEnv::new(spec, version, block), evm.ext());
        evm.set_block_and_execution_config(
            block,
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

/// Convenience type aliases for accessing associated types through [`FoundryEvmNetwork`].
pub type EvmFactoryFor<FEN> = <FEN as FoundryEvmNetwork>::EvmFactory;
pub type TypesFor<FEN> = <EvmFactoryFor<FEN> as FoundryEvmFactory>::Types;
pub type SpecOf<F> = <<F as FoundryEvmFactory>::Types as EvmTypesHost>::SpecId;
pub type SpecFor<FEN> = <TypesFor<FEN> as EvmTypesHost>::SpecId;
pub type BlockEnvFor<FEN> = BlockEnv<TypesFor<FEN>>;
pub type TxFor<FEN> = <TypesFor<FEN> as EvmTypesHost>::Tx;
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
