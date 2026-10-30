//! Foundry's execution stack on [evm2](https://github.com/alloy-rs/evm2).
//!
//! [`FoundryEvmNetwork`] binds an Alloy network to a [`FoundryEvmFactory`], which owns the
//! network's evm2 type family ([`EvmTypesHost`]): spec, transaction, block and result extensions,
//! the transaction registry and the precompiles. One marker exists per network family:
//! [`EthEvmNetwork`], [`OpEvmNetwork`], [`BaseEvmNetwork`], [`MonadEvmNetwork`] and
//! [`TempoEvmNetwork`].
//!
//! Transactions stay network-native until execution (see [`tx`](self::TxPayload)): tools keep
//! [`TransactionRequestFor`] or [`TxEnvelopeFor`] values and convert once with
//! [`FoundryEvmNetwork::tx_from_request`], [`FoundryEvmNetwork::tx_from_signed`],
//! [`FoundryEvmNetwork::decode_tx`] or [`FoundryEvmNetwork::tx_from_any_rpc`].
//!
//! State ownership is split three ways:
//!
//! - evm2's `State` owns transaction scratch, journaling, warmth, transient storage, logs and the
//!   running context's accepted overlay;
//! - a [`BackingDb`] serves reads that miss the overlay and never receives writes;
//! - [`ExecutionLifecycle`] owns the accepted state of inactive forks, persistent accounts,
//!   snapshots and nested execution.
//!
//! [`crate::backend::Backend`] layers RPC forks and Foundry's cheatcode policy on top of the
//! lifecycle.

use alloy_consensus::{
    SignableTransaction, Signed,
    transaction::{Recovered, SignerRecoverable},
};
use alloy_eips::{Decodable2718, Typed2718};
use alloy_network::{
    AnyRpcTransaction, Network, NetworkTransactionBuilder, TransactionBuilder, TransactionResponse,
};
use alloy_primitives::{Address, Signature, map::AddressMap};
use alloy_rlp::Decodable;
use evm2::{
    Evm, EvmTypes, EvmTypesHost, ExecutionConfig, TxResult, Version,
    env::BlockEnv,
    evm::{AccountInfo, precompile::PrecompileProvider},
    registry::TxRegistry,
};
use foundry_common::{FoundryReceiptResponse, FoundryTransactionBuilder, fmt::UIfmt};
use foundry_evm_hardforks::ExecutionSpec;
use foundry_evm_networks::NetworkConfigs;
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

#[cfg(feature = "base")]
pub use base::BaseEvmNetwork;
pub use eth::*;
#[cfg(feature = "monad")]
pub use monad::MonadEvmNetwork;
#[cfg(feature = "optimism")]
pub use op::OpEvmNetwork;
pub use tempo::TempoEvmNetwork;

#[cfg(test)]
pub(crate) mod tests;

/// Binds an Alloy network to an evm2 factory.
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
                                    + Default
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

    /// Converts a transaction fetched over RPC into this network's envelope.
    ///
    /// Fork RPC responses are network-agnostic ([`AnyRpcTransaction`]); envelopes this network
    /// cannot represent fail with an error rather than being projected onto another type.
    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<TxEnvelopeFor<Self>>;

    /// Converts a transaction fetched over RPC into an evm2 transaction sent by its RPC sender,
    /// as it executed on chain.
    fn tx_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<Recovered<TxFor<Self>>> {
        Self::evm_tx(Self::envelope_from_any_rpc(tx)?, tx.from(), TxPayload::Signed)
    }
}

/// Returns the error for an RPC transaction whose envelope `target` cannot represent.
pub(crate) fn unknown_transaction_type(tx: &AnyRpcTransaction, target: &str) -> eyre::Report {
    eyre::eyre!("cannot convert unknown transaction type {:#04x} to {target}", tx.inner.inner.ty())
}

/// Builds Foundry-configured EVMs for one evm2 [`EvmTypesHost`] family.
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

    /// Returns this factory configured for `networks`, such as Celo's transfer precompile on
    /// Ethereum.
    fn with_networks(&self, _networks: NetworkConfigs) -> Self {
        self.clone()
    }

    /// Folds the running transaction's execution so far into network state derived from it, such
    /// as Monad's reserve-balance tracker. Call it before Foundry changes that state outside
    /// execution, then call [`Self::refresh_transaction`].
    fn sync_transaction(&self, _evm: &mut Evm<'_, Self::Types>) {}

    /// Rebases network state derived from the running transaction's state onto that state after
    /// Foundry changed it outside execution, such as cheatcode writes and isolated calls. The
    /// change itself is not execution: Monad's reserve-balance tracker recomputes the thresholds
    /// of its tracked accounts without recording debits or credits.
    fn refresh_transaction(&self, _evm: &mut Evm<'_, Self::Types>) {}

    /// Like [`Self::refresh_transaction`], after a fork switch accepted the running transaction's
    /// changes: `original` holds the transaction-start info of the persistent accounts it carried.
    fn refresh_switched_transaction(
        &self,
        evm: &mut Evm<'_, Self::Types>,
        _original: &AddressMap<Option<AccountInfo>>,
    ) {
        self.refresh_transaction(evm);
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
        factory.new_evm_with_ext(db, env, ext)
    }

    /// Creates a Foundry-configured EVM with the given network extension state, such as the
    /// running EVM's for a nested frame.
    fn new_evm_with_ext<'a>(
        &self,
        db: impl FoundryDatabase + 'a,
        env: EvmEnv<Self::Types>,
        ext: <Self::Types as EvmTypesHost>::EvmExt,
    ) -> Evm<'a, Self::Types> {
        db.set_eip161(env.version.feature(evm2::EvmFeatures::EIP161));
        let precompiles = self.precompiles(&env, &ext);
        let EvmEnv { spec, version, block } = env;
        Evm::new_with_execution_config_and_ext(
            self.execution_config(spec, version),
            spec,
            block,
            self.tx_registry(spec),
            db,
            precompiles,
            ext,
        )
    }

    /// Returns `env` with `spec`, keeping Foundry's execution toggles and limits.
    fn env_with_spec(&self, env: EvmEnv<Self::Types>, spec: SpecOf<Self>) -> EvmEnv<Self::Types> {
        let current_default = self.version(env.spec, env.chain_id());
        let next = self.version(spec, env.chain_id());
        EvmEnv::new(spec, carry_execution_policy(&current_default, &env.version, next), env.block)
    }

    /// Switches `evm` to `spec`, keeping Foundry's execution toggles and limits.
    ///
    /// This rebuilds the dispatch tables, the transaction registry and the precompiles, so opcode
    /// and precompile availability follows the new spec. It must not be called while `evm` is
    /// executing.
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
pub type InterpreterFor<'frame, 'host, FEN> =
    evm2::interpreter::Interpreter<'frame, 'host, TypesFor<FEN>>;
pub type MessageFor<FEN> = evm2::interpreter::Message<TypesFor<FEN>>;
pub type MessageResultFor<FEN> = evm2::interpreter::MessageResult<TypesFor<FEN>>;
pub type NetworkFor<FEN> = <FEN as FoundryEvmNetwork>::Network;
pub type TxEnvelopeFor<FEN> = <NetworkFor<FEN> as Network>::TxEnvelope;
pub type TransactionRequestFor<FEN> = <NetworkFor<FEN> as Network>::TransactionRequest;
