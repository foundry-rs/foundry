//! evm2 execution for every network family Anvil runs.
//!
//! Anvil keeps one network-agnostic environment ([`EvmEnv`]) and
//! one transaction type ([`FoundryTxEnvelope`]). Right
//! before execution they are converted into the selected family's evm2 environment and transaction
//! through [`AnvilEvmNetwork`], so execution, mining and
//! tracing code is written once, generically, and dispatched once per call with
//! `with_evm_network!`.

use crate::{
    PrecompileFactory,
    eth::{
        backend::{
            cheats::CheatsManager,
            db::{DatabaseRef, EvmDb},
            env::{EvmEnv, SpecId},
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
    interpreter::{GasTracker, Message},
    precompiles::{MovePrecompileError, PrecompileId, PrecompileResult},
};
use foundry_evm_core::{
    evm::celo_transfer,
    precompiles::{EC_RECOVER, P256_VERIFY},
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
    TxPayload, TypesFor, bsc_p256,
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
    /// Per-phase statuses of an included signed Base EIP-8130 transaction.
    pub eip8130_phase_statuses: Option<Vec<u8>>,
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
    /// Acting actor hint (`senderActorId`) of a simulated Base EIP-8130 request.
    pub eip8130_sender_actor_id: Option<B256>,
}

impl PreparedTx {
    /// Prepares a signed or impersonated transaction.
    pub const fn signed(tx: FoundryTxEnvelope, sender: Address) -> Self {
        Self { tx, sender, payload: TxPayload::Signed, tempo: None, eip8130_sender_actor_id: None }
    }

    /// Prepares a simulated request.
    pub const fn simulated(tx: FoundryTxEnvelope, sender: Address) -> Self {
        Self {
            tx,
            sender,
            payload: TxPayload::Simulated,
            tempo: None,
            eip8130_sender_actor_id: None,
        }
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
    /// Whether the transaction is trusted Monad history, such as a mined or forced replay, so a
    /// canonical protocol system envelope executes as a protocol system call.
    #[cfg(feature = "monad")]
    pub monad_protocol_replay: bool,
}

/// Anvil's binding of an evm2 network family.
pub trait AnvilEvmNetwork: FoundryEvmNetwork {
    /// Returns the network spec executing `hardfork`.
    fn spec(hardfork: FoundryHardfork) -> SpecFor<Self>;

    /// Returns the factory configured by `opts`.
    fn factory(_opts: &EvmOptions) -> EvmFactoryFor<Self> {
        Default::default()
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

    /// Converts a prepared transaction, including its network's RPC simulation identity.
    fn prepared_evm_tx(
        prepared: &PreparedTx,
        cheats: Option<&CheatsManager>,
    ) -> Result<Recovered<TxFor<Self>>, BlockchainError> {
        evm_tx::<Self>(&prepared.tx, prepared.sender, prepared.payload, cheats)
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
        // Only canonical envelopes replay as protocol system calls, so other transactions from
        // the system sender, such as impersonated ones, keep their ordinary semantics.
        let protocol = context.monad_protocol_replay
            && <monad_evm2::MonadEvmExt as FoundryEvmExt<_>>::is_protocol_transaction(tx);
        evm.ext_mut().set_protocol_replay(protocol);
    }
}

#[cfg(feature = "optimism")]
impl AnvilEvmNetwork for OpEvmNetwork {
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
    fn spec(hardfork: FoundryHardfork) -> base_evm2::BaseSpecId {
        hardfork.into()
    }

    fn factory(opts: &EvmOptions) -> base_evm2::BaseEvmFactory {
        base_evm2::BaseEvmFactory::default()
            .with_activation_admin_address(opts.base_activation_admin)
    }

    fn prepared_evm_tx(
        prepared: &PreparedTx,
        cheats: Option<&CheatsManager>,
    ) -> Result<Recovered<base_evm2::BaseTxEnvelope>, BlockchainError> {
        let tx = evm_tx::<Self>(&prepared.tx, prepared.sender, prepared.payload, cheats)?;
        // Signed transactions authenticate their actor; only simulations accept the RPC hint.
        if prepared.payload.is_simulated()
            && let Some(sender_actor_id) = prepared.eip8130_sender_actor_id
        {
            let (mut inner, signer) = tx.into_parts();
            inner.set_eip8130_simulation(Some(sender_actor_id));
            return Ok(Recovered::new_unchecked(inner, signer));
        }
        Ok(tx)
    }
}

impl AnvilEvmNetwork for TempoEvmNetwork {
    fn spec(hardfork: FoundryHardfork) -> tempo_hardfork::TempoHardfork {
        hardfork.into()
    }

    fn apply_authority_overrides(tx: &mut tempo_evm::TempoTxEnv, overrides: &[(usize, Address)]) {
        let Some(eip7702) = tx.as_eip7702() else { return };
        let mut eip7702 = eip7702.clone();
        override_authorities(&mut eip7702, overrides);
        // `from_recovered_eth` keeps recovered authorizations. It re-derives the sender-scoped
        // identifier from placeholder signatures, so restore the signed transaction's identifier.
        // The rebuilt environment's `tx_hash` is the placeholder envelope's hash and its execution
        // context is `Simulation`; only Tempo's block executor reads them, not Anvil's.
        let unique_tx_identifier = tx.unique_tx_identifier();
        let eth = Recovered::new_unchecked(
            evm2::ethereum::TxEnvelope::Eip7702(eip7702),
            tx.evm_tx().signer(),
        );
        *tx = tempo_evm::TempoTxEnv::from_recovered_eth(eth)
            .expect("EIP-7702 converts to Tempo")
            .with_simulation_overrides(unique_tx_identifier, None, None);
    }

    fn prepared_evm_tx(
        prepared: &PreparedTx,
        cheats: Option<&CheatsManager>,
    ) -> Result<Recovered<tempo_evm::TempoTxEnv>, BlockchainError> {
        let tx = evm_tx::<Self>(&prepared.tx, prepared.sender, prepared.payload, cheats)?;
        if let Some(simulation) = &prepared.tempo {
            let (inner, signer) = tx.into_parts();
            return Ok(Recovered::new_unchecked(
                inner.with_simulation_overrides(
                    simulation.unique_tx_identifier,
                    simulation.fee_payer,
                    simulation.key_id,
                ),
                signer,
            ));
        }
        Ok(tx)
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
    /// Whether Celo's transfer precompile is installed.
    pub celo_transfer: bool,
    /// Extra precompiles installed by a library user.
    pub precompile_factory: Option<Arc<dyn PrecompileFactory>>,
    /// Cheat state providing ecrecover overrides.
    pub cheats: Option<CheatsManager>,
    /// Arbitrum L2 block number reported by `ArbSys`, for Arbitrum forks.
    pub arbitrum_block_number: Option<u64>,
    /// BSC's `P256VERIFY` on BSC chains: whether it is active, and its Osaka variant.
    pub bsc_p256: Option<Option<bool>>,
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
            celo_transfer: false,
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
            .field("celo_transfer", &self.celo_transfer)
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
    let db = EvmDb::new(db, env.cfg_env.spec, opts.fork_source_chain_id);
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
    let envelope = decode_envelope::<N>(tx)?;
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

/// Executes `prepared` on `evm` and detaches its state changes.
pub fn transact<N: AnvilEvmNetwork>(
    evm: &mut Evm<'_, TypesFor<N>>,
    prepared: &PreparedTx,
    context: &TxContext,
    cheats: Option<&CheatsManager>,
) -> Result<Executed, BlockchainError> {
    let tx = N::prepared_evm_tx(prepared, cheats)?;
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
    // Validate presence before converting to evm2's non-optional blob base fee. Fork setup has
    // already normalized headers from chains that legitimately omit the blob fields.
    if env.cfg_env.spec >= SpecId::CANCUN && env.block_env.blob_excess_gas_and_price.is_none() {
        return Err(BlockchainError::ExcessBlobGasNotSet);
    }
    let mut evm = build_evm::<N>(db, env, opts)?;
    evm.set_inspector(inspector);
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

/// Converts a family result into a network-neutral one.
pub fn exec_result<N: AnvilEvmNetwork>(result: TxResult<TypesFor<N>>) -> ExecResult {
    let ext = ResultExt {
        failed_deposit: result.ext.is_failed_deposit(),
        eip8130_phase_statuses: result.ext.eip8130_phase_statuses().map(<[u8]>::to_vec),
    };
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

/// Wraps a network's precompiles with Anvil's extras, cheat ecrecover and simulation moves.
struct AnvilPrecompiles<'a, T: evm2::EvmTypes> {
    inner: Box<dyn PrecompileProvider<T> + 'a>,
    extra: AddressMap<ExtraPrecompile>,
    cheats: Option<CheatsManager>,
    /// Destination to source of moved precompiles.
    moved_to: AddressMap<Address>,
    /// Sources of moved precompiles, no longer served at their protocol address.
    moved_from: AddressSet,
    /// Network precompiles the chain has not activated, such as BSC's `P256VERIFY` before Haber.
    removed: AddressSet,
}

impl<'a, T: evm2::EvmTypes> AnvilPrecompiles<'a, T> {
    /// Wraps `inner` with the extras configured by `opts`.
    fn new(inner: Box<dyn PrecompileProvider<T> + 'a>, opts: &EvmOptions) -> Self {
        let mut extra = AddressMap::default();
        if opts.celo_transfer {
            extra.insert(CELO_TRANSFER_ADDRESS, ExtraPrecompile::CeloTransfer);
        }
        let mut removed = AddressSet::default();
        match opts.bsc_p256 {
            Some(Some(osaka)) => {
                extra.insert(P256_VERIFY, ExtraPrecompile::P256 { osaka });
            }
            Some(None) => {
                removed.insert(P256_VERIFY);
            }
            None => {}
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
        Self {
            inner,
            extra,
            cheats,
            moved_to: Default::default(),
            moved_from: Default::default(),
            removed,
        }
    }

    fn contains_unmoved(&self, address: &Address) -> bool {
        self.extra.contains_key(address)
            || (!self.removed.contains(address) && self.inner.contains(address))
    }
}

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
            .filter(|address| !self.removed.contains(address))
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
            .filter(|(address, _)| {
                !self.extra.contains_key(address) && !self.removed.contains(address)
            })
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
        // evm2 dispatches native precompiles at the call target.
        let address = match self.moved_to.get(&message.call_target) {
            Some(source) => *source,
            None if self.moved_from.contains(&message.call_target) => return None,
            None => message.call_target,
        };
        if address == EC_RECOVER
            && let Some(cheats) = &self.cheats
            && let Some(result) = cheat_ecrecover(cheats, &message.input, gas)
        {
            return Some(result);
        }
        if let Some(extra) = self.extra.get(&address).cloned() {
            return Some(execute_extra(&extra, evm, message, address, gas));
        }
        if self.removed.contains(&address) {
            return None;
        }
        if address == message.call_target {
            return self.inner.execute(evm, message, gas);
        }
        let mut moved = message.clone();
        moved.call_target = address;
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
        ExtraPrecompile::CeloTransfer => celo_transfer(evm, &message.input, gas),
    }
}
