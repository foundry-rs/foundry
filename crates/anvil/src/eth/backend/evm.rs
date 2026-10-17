//! evm2 execution for every network family Anvil runs.
//!
//! Anvil keeps one network-agnostic environment ([`EvmEnv`](crate::eth::backend::env::EvmEnv)) and
//! one transaction type ([`FoundryTxEnvelope`](foundry_primitives::FoundryTxEnvelope)). Right
//! before execution they are converted into the selected family's evm2 environment and transaction
//! through [`AnvilEvmNetwork`](crate::eth::backend::evm::AnvilEvmNetwork), so execution, mining and
//! tracing code is written once, generically, and dispatched once per call with
//! `with_evm_network!`.

use crate::{
    PrecompileFactory,
    eth::{
        backend::{
            cheats::CheatsManager,
            db::{DatabaseRef, EvmDb},
            env::EvmEnv,
        },
        error::BlockchainError,
    },
};
use alloy_consensus::{Signed, transaction::Recovered};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_primitives::{
    Address, B256, Bytes, U256,
    map::{AddressMap, AddressSet},
};
use evm2::{
    Evm, EvmFeatures, EvmTypesHost, Inspector, PrecompileError,
    env::BlockEnv as NetBlockEnv,
    evm::{PendingState, TxResult, TxResultExt, precompile::PrecompileProvider},
    interpreter::{GasTracker, Interpreter, Message, MessageResult},
    precompiles::{MovePrecompileError, PrecompileId, PrecompileResult},
};
use foundry_evm_hardforks::FoundryHardfork;
use foundry_evm_networks::{
    NetworkConfigs,
    arbitrum::{ARB_BLOCK_NUMBER_SELECTOR, ARB_SYS_ADDRESS, arb_block_number_call},
    celo::transfer::CELO_TRANSFER_ADDRESS,
};
use foundry_primitives::FoundryTxEnvelope;
use std::{fmt, sync::Arc};

pub use evm2::evm::precompile::PrecompileOutput;
pub use foundry_evm_core::evm::{
    EthEvmNetwork, EvmFactoryFor, FoundryDatabase, FoundryEvmExt, FoundryEvmFactory,
    FoundryEvmNetwork, FoundryTxResultExt, SpecFor, TempoEvmNetwork, TxEnvelopeFor, TxFor,
    TxPayload, TypesFor,
};

#[cfg(feature = "base")]
pub use foundry_evm_core::evm::BaseEvmNetwork;
#[cfg(feature = "monad")]
pub use foundry_evm_core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
pub use foundry_evm_core::evm::OpEvmNetwork;

/// The evm2 type family a network executes with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvmFamily {
    /// Ethereum and Ethereum-equivalent chains, including Celo.
    #[default]
    Eth,
    /// OP Stack chains.
    #[cfg(feature = "optimism")]
    Op,
    /// Base.
    #[cfg(feature = "base")]
    Base,
    /// Monad.
    #[cfg(feature = "monad")]
    Monad,
    /// Tempo.
    Tempo,
}

impl EvmFamily {
    /// Returns the family executing `networks`.
    pub const fn from_networks(networks: &NetworkConfigs) -> Self {
        #[cfg(feature = "base")]
        if networks.is_base() {
            return Self::Base;
        }
        #[cfg(feature = "optimism")]
        if networks.is_optimism() {
            return Self::Op;
        }
        #[cfg(feature = "monad")]
        if networks.is_monad() {
            return Self::Monad;
        }
        if networks.is_tempo() {
            return Self::Tempo;
        }
        Self::Eth
    }
}

/// Runs `$body` with `$N` bound to the [`AnvilEvmNetwork`] of `$family`.
macro_rules! with_evm_network {
    ($family:expr, $N:ident => $body:expr) => {
        match $family {
            $crate::eth::backend::evm::EvmFamily::Eth => {
                type $N = $crate::eth::backend::evm::EthEvmNetwork;
                $body
            }
            #[cfg(feature = "optimism")]
            $crate::eth::backend::evm::EvmFamily::Op => {
                type $N = $crate::eth::backend::evm::OpEvmNetwork;
                $body
            }
            #[cfg(feature = "base")]
            $crate::eth::backend::evm::EvmFamily::Base => {
                type $N = $crate::eth::backend::evm::BaseEvmNetwork;
                $body
            }
            #[cfg(feature = "monad")]
            $crate::eth::backend::evm::EvmFamily::Monad => {
                type $N = $crate::eth::backend::evm::MonadEvmNetwork;
                $body
            }
            $crate::eth::backend::evm::EvmFamily::Tempo => {
                type $N = $crate::eth::backend::evm::TempoEvmNetwork;
                $body
            }
        }
    };
}
pub(crate) use with_evm_network;

/// Network-neutral outcome flags of an executed transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultExt {
    /// Whether an OP-stack deposit failed and kept only its mint and nonce bump.
    pub failed_deposit: bool,
}

/// Network-neutral transaction result.
pub type ExecResult = TxResultExt<ResultExt>;

/// Tempo's identity for a simulated RPC request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TempoSimulation {
    /// Transaction identifier used for expiring nonces and channel reserves.
    pub unique_tx_identifier: B256,
    /// Fee payer recovered from the request's fee payer signature.
    pub fee_payer: Option<Address>,
    /// Access key that prices the simulation.
    pub key_id: Option<Address>,
}

/// A transaction prepared for execution.
#[derive(Clone, Debug)]
pub struct PreparedTx {
    /// The transaction, signed with a placeholder signature for simulated requests.
    pub tx: FoundryTxEnvelope,
    /// The sender.
    pub sender: Address,
    /// Whether the transaction is signed or a simulation.
    pub payload: TxPayload,
    /// Tempo simulation identity of a Tempo RPC request.
    pub tempo: Option<TempoSimulation>,
}

impl PreparedTx {
    /// Prepares a signed or impersonated transaction.
    pub const fn signed(tx: FoundryTxEnvelope, sender: Address) -> Self {
        Self { tx, sender, payload: TxPayload::Signed, tempo: None }
    }

    /// Prepares a simulated request.
    pub const fn simulated(tx: FoundryTxEnvelope, sender: Address) -> Self {
        Self { tx, sender, payload: TxPayload::Simulated, tempo: None }
    }
}

/// A transaction executed against a state without committing it.
#[derive(Clone, Debug, Default)]
pub struct Executed {
    /// Network-neutral result.
    pub result: ExecResult,
    /// State changes of the transaction.
    pub state: PendingState,
}

/// Per-transaction network context that is not part of the transaction itself.
#[derive(Clone, Debug, Default)]
pub struct TxContext {
    /// Monad's block-level sender and authority context for the reserve-balance rules.
    #[cfg(feature = "monad")]
    pub monad: Option<monad_evm2::MonadChainContext>,
}

/// Anvil's binding of an evm2 network family.
pub trait AnvilEvmNetwork: FoundryEvmNetwork {
    /// The family this network executes with.
    const FAMILY: EvmFamily;

    /// Returns the network spec executing `hardfork`.
    fn spec(hardfork: FoundryHardfork) -> SpecFor<Self>;

    /// Returns the factory configured by `opts`.
    fn factory(_opts: &EvmOptions) -> EvmFactoryFor<Self> {
        Default::default()
    }

    /// Converts a signed or impersonated Anvil transaction into the network's envelope.
    fn envelope(tx: &FoundryTxEnvelope) -> Result<TxEnvelopeFor<Self>, BlockchainError> {
        decode_envelope::<Self>(tx)
    }

    /// Replaces EIP-7702 authorities by authorization index, see [`authority_overrides`].
    fn apply_authority_overrides(_tx: &mut TxFor<Self>, _overrides: &[(usize, Address)]) {}

    /// Applies per-transaction context to `evm`.
    fn set_tx_context(
        evm: &mut Evm<'_, TypesFor<Self>>,
        tx: &Recovered<TxFor<Self>>,
        _context: &TxContext,
    ) {
        evm.ext_mut().for_transaction(tx);
    }

    /// Marks a simulated transaction with Tempo's RPC simulation identity.
    fn apply_tempo_simulation(tx: TxFor<Self>, _simulation: &TempoSimulation) -> TxFor<Self> {
        tx
    }

    /// Returns the network-neutral outcome flags.
    fn result_ext(ext: &<TypesFor<Self> as EvmTypesHost>::TxResultExt) -> ResultExt {
        ResultExt { failed_deposit: ext.is_failed_deposit() }
    }
}

/// Converts `tx` into `N`'s envelope through its exact EIP-2718 encoding.
///
/// Celo CIP-64 transactions execute as EIP-1559 transactions with the same fields: fee currencies
/// are not supported by evm2.
fn decode_envelope<N: FoundryEvmNetwork>(
    tx: &FoundryTxEnvelope,
) -> Result<TxEnvelopeFor<N>, BlockchainError> {
    let encoded = match tx {
        FoundryTxEnvelope::Celo(signed) => FoundryTxEnvelope::Eip1559(Signed::new_unchecked(
            signed.tx().inner.clone(),
            *signed.signature(),
            *signed.hash(),
        ))
        .encoded_2718(),
        tx => tx.encoded_2718(),
    };
    TxEnvelopeFor::<N>::decode_2718_exact(&encoded)
        .map_err(|_| BlockchainError::FailedToDecodeTransaction)
}

/// Returns the ecrecover cheat overrides of `tx`'s EIP-7702 authorizations that do not recover.
pub fn authority_overrides(
    tx: &FoundryTxEnvelope,
    cheats: &CheatsManager,
) -> Vec<(usize, Address)> {
    use alloy_consensus::Transaction;

    if !cheats.has_recover_overrides() {
        return Vec::new();
    }
    tx.authorization_list()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, auth)| auth.recover_authority().is_err())
        .filter_map(|(index, auth)| {
            let signature = auth.signature().ok()?;
            Some((index, cheats.get_recover_override(&signature.as_bytes().into())?))
        })
        .collect()
}

/// Applies authority overrides to an evm2 EIP-7702 transaction.
fn override_authorities(tx: &mut evm2::ethereum::LazyTxEip7702, overrides: &[(usize, Address)]) {
    use alloy_eips::eip7702::{RecoveredAuthority, RecoveredAuthorization};
    use evm2::ethereum::LazyAuthorization;

    for (index, authority) in overrides {
        if let Some(auth) = tx.authorization_list.get_mut(*index) {
            *auth = LazyAuthorization::Recovered(RecoveredAuthorization::new_unchecked(
                auth.inner().clone(),
                RecoveredAuthority::Valid(*authority),
            ));
        }
    }
}

impl AnvilEvmNetwork for EthEvmNetwork {
    const FAMILY: EvmFamily = EvmFamily::Eth;

    fn spec(hardfork: FoundryHardfork) -> evm2::SpecId {
        hardfork.into()
    }

    fn apply_authority_overrides(
        tx: &mut evm2::ethereum::TxEnvelope,
        overrides: &[(usize, Address)],
    ) {
        if let evm2::ethereum::TxEnvelope::Eip7702(tx) = tx {
            override_authorities(tx, overrides);
        }
    }
}

#[cfg(feature = "monad")]
impl AnvilEvmNetwork for MonadEvmNetwork {
    const FAMILY: EvmFamily = EvmFamily::Monad;

    fn spec(hardfork: FoundryHardfork) -> monad_evm2::MonadHardfork {
        hardfork.into()
    }

    fn apply_authority_overrides(
        tx: &mut evm2::ethereum::TxEnvelope,
        overrides: &[(usize, Address)],
    ) {
        EthEvmNetwork::apply_authority_overrides(tx, overrides);
    }

    fn set_tx_context(
        evm: &mut Evm<'_, monad_evm2::MonadEvmTypes>,
        tx: &Recovered<evm2::ethereum::TxEnvelope>,
        context: &TxContext,
    ) {
        match &context.monad {
            Some(chain) => evm.ext_mut().chain = chain.clone(),
            None => evm.ext_mut().for_transaction(tx),
        }
    }
}

#[cfg(feature = "optimism")]
impl AnvilEvmNetwork for OpEvmNetwork {
    const FAMILY: EvmFamily = EvmFamily::Op;

    fn spec(hardfork: FoundryHardfork) -> op_evm2::OpSpecId {
        hardfork.into()
    }

    fn apply_authority_overrides(tx: &mut op_evm2::OpTxEnvelope, overrides: &[(usize, Address)]) {
        if let op_evm2::OpTypedTx::Eip7702(tx) = &mut tx.tx {
            override_authorities(tx, overrides);
        }
    }
}

#[cfg(feature = "base")]
impl AnvilEvmNetwork for BaseEvmNetwork {
    const FAMILY: EvmFamily = EvmFamily::Base;

    fn spec(hardfork: FoundryHardfork) -> base_evm2::BaseSpecId {
        hardfork.into()
    }

    fn factory(opts: &EvmOptions) -> base_evm2::BaseEvmFactory {
        base_evm2::BaseEvmFactory::default()
            .with_activation_admin_address(opts.base_activation_admin)
    }
}

impl AnvilEvmNetwork for TempoEvmNetwork {
    const FAMILY: EvmFamily = EvmFamily::Tempo;

    fn spec(hardfork: FoundryHardfork) -> tempo_hardfork::TempoHardfork {
        hardfork.into()
    }

    fn apply_tempo_simulation(
        tx: tempo_evm::TempoTxEnv,
        simulation: &TempoSimulation,
    ) -> tempo_evm::TempoTxEnv {
        tx.with_simulation_overrides(
            simulation.unique_tx_identifier,
            simulation.fee_payer,
            simulation.key_id,
        )
    }
}

/// A precompile installed by an Anvil library user.
///
/// Precompiles are network-neutral: they see the call input and charge gas through evm2's
/// [`GasTracker`], so the same precompile runs on every network family.
pub type DynPrecompile =
    Arc<dyn Fn(PrecompileInput<'_>, &mut GasTracker) -> PrecompileResult + Send + Sync>;

/// The call a [`DynPrecompile`] executes.
#[derive(Clone, Copy, Debug)]
pub struct PrecompileInput<'a> {
    /// Call data.
    pub data: &'a [u8],
    /// Immediate caller.
    pub caller: Address,
    /// Value sent with the call.
    pub value: U256,
    /// The precompile's address.
    pub target: Address,
}

/// Everything besides the environment that configures an Anvil EVM.
#[derive(Clone)]
pub struct EvmOptions {
    /// Executed hardfork.
    pub hardfork: FoundryHardfork,
    /// Active network configuration.
    pub networks: NetworkConfigs,
    /// Extra precompiles installed by a library user.
    pub precompile_factory: Option<Arc<dyn PrecompileFactory>>,
    /// Cheat state providing ecrecover overrides.
    pub cheats: Option<CheatsManager>,
    /// Arbitrum L2 block number reported by `ArbSys`, for Arbitrum forks.
    pub arbitrum_block_number: Option<u64>,
    /// Whether the BSC `P256VERIFY` precompile is active, and its Osaka variant.
    pub bsc_p256: Option<bool>,
    /// Precompile moves requested by `eth_simulateV1` state overrides.
    pub precompile_moves: Vec<(Address, Address)>,
    /// Source chain ID of the active fork.
    pub fork_source_chain_id: Option<u64>,
    /// Base activation registry admin.
    pub base_activation_admin: Option<Address>,
}

impl Default for EvmOptions {
    fn default() -> Self {
        Self {
            hardfork: FoundryHardfork::Ethereum(Default::default()),
            networks: Default::default(),
            precompile_factory: None,
            cheats: None,
            arbitrum_block_number: None,
            bsc_p256: None,
            precompile_moves: Vec::new(),
            fork_source_chain_id: None,
            base_activation_admin: None,
        }
    }
}

impl fmt::Debug for EvmOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmOptions")
            .field("hardfork", &self.hardfork)
            .field("networks", &self.networks)
            .field("arbitrum_block_number", &self.arbitrum_block_number)
            .field("precompile_moves", &self.precompile_moves)
            .finish_non_exhaustive()
    }
}

/// Builds `N`'s evm2 runtime version for `env`, applying Anvil's execution toggles.
pub fn version<N: AnvilEvmNetwork>(
    factory: &EvmFactoryFor<N>,
    spec: SpecFor<N>,
    env: &EvmEnv,
) -> evm2::Version {
    let cfg = &env.cfg_env;
    let mut version = factory.version(spec, cfg.chain_id);
    for (disabled, feature) in [
        (!cfg.tx_chain_id_check, EvmFeatures::TX_CHAIN_ID_CHECK),
        (cfg.disable_nonce_check, EvmFeatures::NONCE_CHECK),
        // evm2's sender balance check includes the gas fee even when the fee is not charged, while
        // REVM's only covered the value without a fee charge.
        (cfg.disable_balance_check || cfg.disable_fee_charge, EvmFeatures::BALANCE_CHECK),
        (cfg.disable_block_gas_limit, EvmFeatures::BLOCK_GAS_LIMIT_CHECK),
        (cfg.disable_eip3607, EvmFeatures::EIP3607),
        (cfg.disable_base_fee, EvmFeatures::BASE_FEE_CHECK),
        (cfg.disable_blob_base_fee, EvmFeatures::BLOB_BASE_FEE_CHECK),
        (cfg.disable_priority_fee_check, EvmFeatures::PRIORITY_FEE_CHECK),
        (cfg.disable_fee_charge, EvmFeatures::FEE_CHARGE),
    ] {
        if disabled {
            version.features.remove(feature);
        }
    }
    if let Some(cap) = cfg.tx_gas_limit_cap {
        version.tx_gas_limit_cap = cap;
    }
    version.memory_limit = cfg.memory_limit;
    if let Some(limit) = cfg.limit_contract_code_size {
        version.max_code_size = limit;
        version.max_initcode_size = 2 * limit;
    }
    if let Some(limit) = cfg.limit_contract_initcode_size {
        version.max_initcode_size = limit;
    }
    version
}

/// Converts Anvil's block environment into `T`'s.
pub fn block_env<T: EvmTypesHost>(env: &EvmEnv) -> NetBlockEnv<T> {
    let block = &env.block_env;
    NetBlockEnv::<T> {
        number: block.number,
        beneficiary: block.beneficiary,
        timestamp: block.timestamp,
        gas_limit: U256::from(block.gas_limit),
        basefee: U256::from(block.basefee),
        difficulty: block.difficulty,
        prevrandao: block.prevrandao.unwrap_or_default().into(),
        blob_basefee: U256::from(block.blob_gasprice().unwrap_or_default()),
        slot_num: U256::from(block.slot_num),
        ..Default::default()
    }
}

/// Builds an EVM of family `N` over `db`, configured for `env` and `opts`.
pub fn build_evm<'a, N: AnvilEvmNetwork>(
    db: &'a dyn DatabaseRef,
    env: &EvmEnv,
    opts: &EvmOptions,
) -> Result<Evm<'a, TypesFor<N>>, BlockchainError> {
    let spec = N::spec(opts.hardfork);
    let factory = N::factory(opts);
    let version = version::<N>(&factory, spec, env);
    let net_env = foundry_evm_core::evm::EvmEnv::new(spec, version, block_env::<TypesFor<N>>(env));
    let db = EvmDb::new(db, env.cfg_env.spec).forked(opts.fork_source_chain_id);
    let mut evm = factory.new_evm(db, net_env);
    let inner = factory.precompiles(&net_env, evm.ext());
    let mut precompiles = AnvilPrecompiles::new(inner, opts);
    if !opts.precompile_moves.is_empty() {
        precompiles.move_precompiles(&opts.precompile_moves).map_err(
            |MovePrecompileError::NotAPrecompile(address)| {
                BlockchainError::Message(format!("account {address} is not a precompile"))
            },
        )?;
    }
    evm.set_precompiles(precompiles);
    Ok(evm)
}

/// Converts an Anvil transaction into `N`'s evm2 transaction.
pub fn evm_tx<N: AnvilEvmNetwork>(
    tx: &FoundryTxEnvelope,
    sender: Address,
    payload: TxPayload,
    cheats: Option<&CheatsManager>,
) -> Result<Recovered<TxFor<N>>, BlockchainError> {
    let tx_ref = tx;
    let envelope = N::envelope(tx)?;
    let mut tx = N::evm_tx(envelope, sender, payload)
        .map_err(|err| BlockchainError::Message(err.to_string()))?;
    if let Some(cheats) = cheats {
        let overrides = authority_overrides(tx_ref, cheats);
        if !overrides.is_empty() {
            let (mut inner, signer) = tx.into_parts();
            N::apply_authority_overrides(&mut inner, &overrides);
            tx = Recovered::new_unchecked(inner, signer);
        }
    }
    Ok(tx)
}

/// Converts a prepared transaction into `N`'s evm2 transaction.
pub fn prepared_evm_tx<N: AnvilEvmNetwork>(
    prepared: &PreparedTx,
    cheats: Option<&CheatsManager>,
) -> Result<Recovered<TxFor<N>>, BlockchainError> {
    let tx = evm_tx::<N>(&prepared.tx, prepared.sender, prepared.payload, cheats)?;
    match &prepared.tempo {
        Some(simulation) => {
            let (tx, signer) = tx.into_parts();
            Ok(Recovered::new_unchecked(N::apply_tempo_simulation(tx, simulation), signer))
        }
        None => Ok(tx),
    }
}

/// Executes `prepared` on `evm` and detaches its state changes.
pub fn transact<N: AnvilEvmNetwork>(
    evm: &mut Evm<'_, TypesFor<N>>,
    prepared: &PreparedTx,
    context: &TxContext,
    cheats: Option<&CheatsManager>,
) -> Result<Executed, BlockchainError> {
    let tx = prepared_evm_tx::<N>(prepared, cheats)?;
    N::set_tx_context(evm, &tx, context);
    let detached = evm.transact(&tx)?.detach();
    Ok(Executed { result: exec_result::<N>(detached.result), state: detached.pending_state })
}

/// Executes `prepared` over `db` with a fresh EVM inspected by `inspector`.
pub fn execute<N: AnvilEvmNetwork, I: Inspector<TypesFor<N>>>(
    db: &dyn DatabaseRef,
    env: &EvmEnv,
    opts: &EvmOptions,
    prepared: &PreparedTx,
    context: &TxContext,
    inspector: &mut I,
) -> Result<Executed, BlockchainError> {
    let mut evm = build_evm::<N>(db, env, opts)?;
    evm.set_inspector(InspectorRef(inspector));
    transact::<N>(&mut evm, prepared, context, opts.cheats.as_ref())
}

/// An inspector usable with every network family.
pub trait AnyInspector:
    Inspector<TypesFor<EthEvmNetwork>>
    + Inspector<TypesFor<TempoEvmNetwork>>
    + OpInspector
    + BaseInspector
    + MonadInspector
{
}

impl<I> AnyInspector for I where
    I: Inspector<TypesFor<EthEvmNetwork>>
        + Inspector<TypesFor<TempoEvmNetwork>>
        + OpInspector
        + BaseInspector
        + MonadInspector
{
}

/// An inspector usable with OP Stack chains.
#[cfg(feature = "optimism")]
pub trait OpInspector: Inspector<TypesFor<OpEvmNetwork>> {}
#[cfg(feature = "optimism")]
impl<I: Inspector<TypesFor<OpEvmNetwork>>> OpInspector for I {}
/// An inspector usable with OP Stack chains.
#[cfg(not(feature = "optimism"))]
pub trait OpInspector {}
#[cfg(not(feature = "optimism"))]
impl<I> OpInspector for I {}

/// An inspector usable with Base.
#[cfg(feature = "base")]
pub trait BaseInspector: Inspector<TypesFor<BaseEvmNetwork>> {}
#[cfg(feature = "base")]
impl<I: Inspector<TypesFor<BaseEvmNetwork>>> BaseInspector for I {}
/// An inspector usable with Base.
#[cfg(not(feature = "base"))]
pub trait BaseInspector {}
#[cfg(not(feature = "base"))]
impl<I> BaseInspector for I {}

/// An inspector usable with Monad.
#[cfg(feature = "monad")]
pub trait MonadInspector: Inspector<TypesFor<MonadEvmNetwork>> {}
#[cfg(feature = "monad")]
impl<I: Inspector<TypesFor<MonadEvmNetwork>>> MonadInspector for I {}
/// An inspector usable with Monad.
#[cfg(not(feature = "monad"))]
pub trait MonadInspector {}
#[cfg(not(feature = "monad"))]
impl<I> MonadInspector for I {}

/// Lends an inspector to an EVM.
#[derive(Debug)]
pub struct InspectorRef<'r, I>(pub &'r mut I);

impl<T: evm2::EvmTypes, I: Inspector<T>> Inspector<T> for InspectorRef<'_, I> {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.0.initialize_interp(interp);
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.0.step(interp);
    }

    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.0.step_end(interp);
    }

    fn log(&mut self, log: &alloy_primitives::Log, host: &mut T::Host<'_>) {
        self.0.log(log, host);
    }

    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        self.0.call(interp, message)
    }

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        self.0.call_end(interp, message, result);
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        self.0.create(interp, message)
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        self.0.create_end(interp, message, result);
    }

    fn selfdestruct(
        &mut self,
        contract: &Address,
        target: &Address,
        value: &U256,
        host: &mut T::Host<'_>,
    ) {
        self.0.selfdestruct(contract, target, value, host);
    }
}

/// Converts a family result into a network-neutral one.
pub fn exec_result<N: AnvilEvmNetwork>(result: TxResult<TypesFor<N>>) -> ExecResult {
    let ext = N::result_ext(&result.ext);
    result.with_ext(ext)
}

/// Extra precompile installed by Anvil on top of the network's precompiles.
#[derive(Clone)]
enum ExtraPrecompile {
    /// A library user's precompile.
    User(PrecompileId, DynPrecompile),
    /// Arbitrum's `ArbSys.arbBlockNumber()`.
    ArbSys(u64),
    /// BSC's `P256VERIFY`, with the Osaka gas schedule when set.
    P256 { osaka: bool },
    /// Celo's native token transfer.
    CeloTransfer,
}

impl fmt::Debug for ExtraPrecompile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(id, _) => f.debug_tuple("User").field(id).finish(),
            Self::ArbSys(number) => f.debug_tuple("ArbSys").field(number).finish(),
            Self::P256 { osaka } => f.debug_struct("P256").field("osaka", osaka).finish(),
            Self::CeloTransfer => f.write_str("CeloTransfer"),
        }
    }
}

/// Wraps a network's precompiles with Anvil's extras, cheat ecrecover and simulation moves.
pub struct AnvilPrecompiles<'a, T: evm2::EvmTypes> {
    inner: Box<dyn PrecompileProvider<T> + 'a>,
    extra: AddressMap<ExtraPrecompile>,
    cheats: Option<CheatsManager>,
    /// Destination to source of moved precompiles.
    moved_to: AddressMap<Address>,
    /// Sources of moved precompiles, no longer served at their protocol address.
    moved_from: AddressSet,
}

impl<T: evm2::EvmTypes> fmt::Debug for AnvilPrecompiles<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilPrecompiles")
            .field("extra", &self.extra)
            .field("moved_to", &self.moved_to)
            .finish_non_exhaustive()
    }
}

impl<'a, T: evm2::EvmTypes> AnvilPrecompiles<'a, T> {
    /// Wraps `inner` with the extras configured by `opts`.
    pub fn new(inner: Box<dyn PrecompileProvider<T> + 'a>, opts: &EvmOptions) -> Self {
        let mut extra = AddressMap::default();
        if opts.networks.is_celo() {
            extra.insert(CELO_TRANSFER_ADDRESS, ExtraPrecompile::CeloTransfer);
        }
        if let Some(osaka) = opts.bsc_p256 {
            extra.insert(BSC_P256_ADDRESS, ExtraPrecompile::P256 { osaka });
        }
        if let Some(factory) = &opts.precompile_factory {
            for (address, id, precompile) in factory.precompiles() {
                extra.insert(address, ExtraPrecompile::User(id, precompile));
            }
        }
        if let Some(number) = opts.arbitrum_block_number {
            extra.insert(ARB_SYS_ADDRESS, ExtraPrecompile::ArbSys(number));
        }
        let cheats = opts.cheats.clone().filter(CheatsManager::has_recover_overrides);
        Self { inner, extra, cheats, moved_to: Default::default(), moved_from: Default::default() }
    }

    fn contains_unmoved(&self, address: &Address) -> bool {
        self.extra.contains_key(address) || self.inner.contains(address)
    }
}

/// Address of BSC's `P256VERIFY` precompile.
const BSC_P256_ADDRESS: Address =
    alloy_primitives::address!("0x0000000000000000000000000000000000000100");

/// Address of the ecrecover precompile.
const ECRECOVER_ADDRESS: Address = Address::with_last_byte(1);

/// Gas Celo's transfer precompile charges.
const CELO_TRANSFER_GAS_COST: u64 = 9000;

impl<T: evm2::EvmTypes> PrecompileProvider<T> for AnvilPrecompiles<'_, T> {
    /// Returns the protocol precompile addresses.
    ///
    /// EIP-2929 warms the protocol addresses, so a precompile moved by `eth_simulateV1` stays warm
    /// at its source and its destination is not warmed.
    fn addresses(&self) -> Vec<Address> {
        let mut addresses = self
            .inner
            .addresses()
            .into_iter()
            .chain(self.extra.keys().copied())
            .collect::<Vec<_>>();
        addresses.sort_unstable();
        addresses.dedup();
        addresses
    }

    fn precompile_ids(&self) -> Vec<(Address, PrecompileId)> {
        let mut ids = self
            .inner
            .precompile_ids()
            .into_iter()
            .filter(|(address, _)| !self.extra.contains_key(address))
            .collect::<Vec<_>>();
        ids.extend(self.extra.iter().map(|(address, extra)| {
            let id = match extra {
                ExtraPrecompile::User(id, _) => id.clone(),
                ExtraPrecompile::ArbSys(_) => PrecompileId::custom("ArbSys"),
                ExtraPrecompile::P256 { .. } => PrecompileId::P256Verify,
                ExtraPrecompile::CeloTransfer => PrecompileId::custom("celo transfer"),
            };
            (*address, id)
        }));
        ids
    }

    fn contains(&self, address: &Address) -> bool {
        if self.moved_to.contains_key(address) {
            return true;
        }
        !self.moved_from.contains(address) && self.contains_unmoved(address)
    }

    fn move_precompiles(
        &mut self,
        moves: &[(Address, Address)],
    ) -> Result<(), MovePrecompileError> {
        for (source, destination) in moves {
            if source != destination && !self.contains_unmoved(source) {
                return Err(MovePrecompileError::NotAPrecompile(*source));
            }
        }
        for (source, destination) in moves {
            if source != destination {
                self.moved_from.insert(*source);
                self.moved_to.insert(*destination, *source);
            }
        }
        Ok(())
    }

    fn execute(
        &mut self,
        evm: &mut Evm<'_, T>,
        message: &Message<T>,
        gas: &mut GasTracker,
    ) -> Option<Result<PrecompileOutput, PrecompileError>> {
        let address = match self.moved_to.get(&message.code_address) {
            Some(source) => *source,
            None if self.moved_from.contains(&message.code_address) => return None,
            None => message.code_address,
        };
        if address == ECRECOVER_ADDRESS
            && let Some(cheats) = &self.cheats
            && let Some(result) = cheat_ecrecover(cheats, &message.input, gas)
        {
            return Some(result);
        }
        if let Some(extra) = self.extra.get(&address).cloned() {
            return Some(execute_extra(&extra, evm, message, address, gas));
        }
        if address == message.code_address {
            return self.inner.execute(evm, message, gas);
        }
        let mut moved = message.clone();
        moved.code_address = address;
        moved.destination = address;
        self.inner.execute(evm, &moved, gas)
    }
}

/// Answers ecrecover from the cheat overrides, if the signature has one.
fn cheat_ecrecover(
    cheats: &CheatsManager,
    input: &[u8],
    gas: &mut GasTracker,
) -> Option<PrecompileResult> {
    const ECRECOVER_BASE: u64 = 3_000;
    let mut padded = [0u8; 128];
    let len = input.len().min(128);
    padded[..len].copy_from_slice(&input[..len]);
    let mut signature = [0u8; 65];
    signature[..64].copy_from_slice(&padded[64..128]);
    signature[64] = padded[63];
    let address = cheats.get_recover_override(&Bytes::from(signature))?;
    if let Err(err) = gas.spend(ECRECOVER_BASE) {
        return Some(Err(err.into()));
    }
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(address.as_slice());
    Some(Ok(PrecompileOutput::new(out.into())))
}

fn execute_extra<T: evm2::EvmTypes>(
    extra: &ExtraPrecompile,
    evm: &mut Evm<'_, T>,
    message: &Message<T>,
    address: Address,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let halt = |reason: &'static str| {
        Err(PrecompileError::Halt(evm2::precompiles::PrecompileHalt::Other(reason.into())))
    };
    match extra {
        ExtraPrecompile::User(_, precompile) => precompile(
            PrecompileInput {
                data: &message.input,
                caller: message.caller,
                value: message.value,
                target: address,
            },
            gas,
        ),
        ExtraPrecompile::ArbSys(number) => {
            if message.input.get(..4) != Some(&ARB_BLOCK_NUMBER_SELECTOR) {
                return halt("unsupported ArbSys selector");
            }
            let Some((cost, output)) = arb_block_number_call(gas.remaining(), *number) else {
                return Err(PrecompileError::Halt(evm2::precompiles::PrecompileHalt::OutOfGas));
            };
            gas.spend(cost)?;
            Ok(PrecompileOutput::new(output))
        }
        ExtraPrecompile::P256 { osaka } => {
            if *osaka {
                evm2::precompiles::secp256r1::run_osaka(&message.input, gas)
            } else {
                evm2::precompiles::secp256r1::run(&message.input, gas)
            }
        }
        ExtraPrecompile::CeloTransfer => {
            gas.spend(CELO_TRANSFER_GAS_COST)?;
            let input = &message.input;
            if input.len() != 96 {
                return halt("invalid input length for Celo transfer precompile");
            }
            let from = Address::from_slice(&input[12..32]);
            let to = Address::from_slice(&input[44..64]);
            let value = U256::from_be_slice(&input[64..96]);
            let state = evm.state_mut();
            let balance = state.account_info_untracked(&from)?.map(|info| info.balance);
            if balance.unwrap_or_default() < value {
                return halt("insufficient balance");
            }
            if !state.transfer(&from, &to, &value)? {
                return halt("balance overflow in to account");
            }
            Ok(PrecompileOutput::new(Bytes::new()))
        }
    }
}

const BSC_MAINNET_CHAIN_ID: u64 = 56;
const BSC_TESTNET_CHAIN_ID: u64 = 97;
const BSC_MAINNET_HABER_TIMESTAMP: u64 = 1_718_863_500;
const BSC_TESTNET_HABER_TIMESTAMP: u64 = 1_716_962_820;
const BSC_MAINNET_OSAKA_TIMESTAMP: u64 = 1_777_343_400;
const BSC_TESTNET_OSAKA_TIMESTAMP: u64 = 1_774_319_400;

/// Returns whether BSC's `P256VERIFY` precompile is active on `chain_id` at `timestamp`, and
/// whether it uses the Osaka gas schedule.
pub const fn bsc_p256(chain_id: u64, timestamp: u64) -> Option<bool> {
    let (haber_timestamp, osaka_timestamp) = match chain_id {
        BSC_MAINNET_CHAIN_ID => (BSC_MAINNET_HABER_TIMESTAMP, BSC_MAINNET_OSAKA_TIMESTAMP),
        BSC_TESTNET_CHAIN_ID => (BSC_TESTNET_HABER_TIMESTAMP, BSC_TESTNET_OSAKA_TIMESTAMP),
        _ => return None,
    };
    if timestamp < haber_timestamp { None } else { Some(timestamp >= osaka_timestamp) }
}
