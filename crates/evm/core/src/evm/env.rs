//! Execution environments and network extension state.

use alloy_consensus::transaction::Recovered;
use alloy_eips::eip4844::{BLOB_TX_MIN_BLOB_GASPRICE, fake_exponential};
use alloy_primitives::U256;
use evm2::{EvmFeatures, EvmTypesHost, Version, env::BlockEnv};
use std::fmt::Debug;

/// The spec, runtime version and block an EVM is built with.
///
/// Foundry's execution toggles map onto [`EvmFeatures`] of the [`Version`]:
/// `disable_nonce_check` is `NONCE_CHECK`, `disable_balance_check` is `BALANCE_CHECK`,
/// `disable_base_fee` is `BASE_FEE_CHECK`, `disable_block_gas_limit` is `BLOCK_GAS_LIMIT_CHECK`,
/// `disable_eip3607` is `EIP3607`, `disable_fee_charge` is `FEE_CHARGE`,
/// `disable_priority_fee_check` is `PRIORITY_FEE_CHECK` and `tx_chain_id_check` is
/// `TX_CHAIN_ID_CHECK`. Size and memory limits are plain [`Version`] fields.
pub struct EvmEnv<T: EvmTypesHost> {
    /// Active network spec.
    pub spec: T::SpecId,
    /// Runtime version: chain ID, features, gas parameters and limits.
    pub version: Version,
    /// Block environment.
    pub block: BlockEnv<T>,
}

impl<T: EvmTypesHost<SpecId: Debug>> Debug for EvmEnv<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvmEnv")
            .field("spec", &self.spec)
            .field("version", &self.version)
            .field("block", &self.block)
            .finish()
    }
}

impl<T: EvmTypesHost> Clone for EvmEnv<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: EvmTypesHost> Copy for EvmEnv<T> {}

impl<T: EvmTypesHost> EvmEnv<T> {
    /// Creates an environment.
    pub const fn new(spec: T::SpecId, version: Version, block: BlockEnv<T>) -> Self {
        Self { spec, version, block }
    }

    /// Returns the chain ID.
    pub const fn chain_id(&self) -> u64 {
        self.version.chain_id
    }

    /// Sets the chain ID used by `CHAINID` and transaction validation.
    pub const fn set_chain_id(&mut self, chain_id: u64) {
        self.version.chain_id = chain_id;
    }

    /// Enables or disables `feature`.
    pub const fn set_feature(&mut self, feature: EvmFeatures, enabled: bool) {
        self.version.features.set(feature, enabled);
    }

    /// Sets the blob base fee from the excess blob gas.
    pub fn set_blob_excess_gas(&mut self, excess_blob_gas: u64, base_fee_update_fraction: u64) {
        self.block.blob_basefee = U256::from(fake_exponential(
            BLOB_TX_MIN_BLOB_GASPRICE,
            excess_blob_gas as u128,
            base_fee_update_fraction as u128,
        ));
    }
}

/// Features Foundry toggles independently of the fork.
pub const FOUNDRY_POLICY_FEATURES: EvmFeatures = {
    let mut features = EvmFeatures::TX_CHAIN_ID_CHECK;
    features.insert(EvmFeatures::NONCE_CHECK);
    features.insert(EvmFeatures::BALANCE_CHECK);
    features.insert(EvmFeatures::BALANCE_TOP_UP);
    features.insert(EvmFeatures::BLOCK_GAS_LIMIT_CHECK);
    features.insert(EvmFeatures::EIP3607);
    features.insert(EvmFeatures::PRIORITY_FEE_CHECK);
    features.insert(EvmFeatures::FEE_CHARGE);
    features.insert(EvmFeatures::BASE_FEE_CHECK);
    features.insert(EvmFeatures::CODE_SIZE_CHECK);
    features
};

/// Returns `next` with Foundry's execution policy carried over from `current`.
///
/// `current_default` is the network's default version for the current spec. A policy feature or
/// limit that differs from it was set by Foundry and survives the spec change; everything else,
/// including gas parameters and fork features, comes from `next`.
pub fn carry_execution_policy(
    current_default: &Version,
    current: &Version,
    mut next: Version,
) -> Version {
    let disabled = current_default.features & FOUNDRY_POLICY_FEATURES & !current.features;
    let enabled = current.features & FOUNDRY_POLICY_FEATURES & !current_default.features;
    next.features = (next.features & !disabled) | enabled;
    next.chain_id = current.chain_id;
    macro_rules! carry_overridden {
        ($($field:ident),*) => {$(
            if current.$field != current_default.$field {
                next.$field = current.$field;
            }
        )*};
    }
    carry_overridden!(tx_gas_limit_cap, memory_limit, max_code_size, max_initcode_size);
    // Foundry disables the EIP-7825 cap with `u64::MAX`, which is also the default of specs
    // before the cap, so a disabled cap survives moving to a spec that has one.
    if current.tx_gas_limit_cap == u64::MAX {
        next.tx_gas_limit_cap = u64::MAX;
    }
    next
}

/// Network-specific block environment fields.
///
/// Common block fields are public on [`BlockEnv`]; only extension fields need accessors.
pub trait FoundryBlockExt: Copy + Debug + Default + Send + Sync + 'static {}

impl FoundryBlockExt for () {}

/// Per-EVM network state, such as OP L1 block info or Monad's reserve-balance context.
pub trait FoundryEvmExt<Tx>: SendableExt + Debug + Default + 'static {
    /// Prepares the state for a standalone synthetic transaction.
    fn for_transaction(&mut self, _tx: &Recovered<Tx>) {}

    /// Prepares the state for the transaction at `index` of `current`, a block whose parent and
    /// grandparent blocks contained `parent` and `grandparent`, such as when replaying a block.
    fn for_block(
        &mut self,
        _grandparent: &[Recovered<Tx>],
        _parent: &[Recovered<Tx>],
        _current: &[Recovered<Tx>],
        _index: usize,
    ) {
    }

    /// Replaces the state of an EVM that may be executing with `ext`, such as one restored by a
    /// snapshot revert, keeping the state of the frames still executing.
    fn replace_while_executing(&mut self, ext: Self) {
        *self = ext;
    }

    /// Returns whether `tx` is a canonical protocol transaction of the network, such as a Monad
    /// staking syscall, which only executes with [`Self::set_protocol_replay`] enabled.
    fn is_protocol_transaction(_tx: &Recovered<Tx>) -> bool {
        false
    }

    /// Sets whether the next transaction continues the transaction state derived so far, such as
    /// Monad's reserve-balance tracker, instead of starting its own. Isolated calls continue the
    /// running transaction's.
    fn set_preserve_transaction_state(&mut self, _preserve: bool) {}

    /// Sets whether transactions from the network's protocol sender execute as protocol
    /// transactions.
    ///
    /// Enable it only to replay trusted canonical chain history, after preparing the transaction
    /// with [`Self::for_transaction`] or [`Self::for_block`], which disable it.
    fn set_protocol_replay(&mut self, _enabled: bool) {}

    /// Prepares the state for a synthetic transaction appended at the retained block cursor,
    /// such as a test call or `vm.executeTransaction`.
    ///
    /// Networks without block-position state treat it as standalone (see
    /// [`Self::for_transaction`]). The retained cursor itself does not move: only canonical
    /// replays advance it, through [`Self::adopt_cursor`].
    fn for_next_transaction(&mut self, tx: &Recovered<Tx>) {
        self.for_transaction(tx);
    }

    /// Moves the retained block cursor to `cursor`'s, keeping the running transaction's own
    /// position at the new cursor and every other state.
    fn adopt_cursor(&mut self, _cursor: &Self) {}

    /// Switches an EVM that may be executing to the parked context state `ext`, such as on a fork
    /// switch.
    ///
    /// Networks whose context state is only a block cursor, such as Monad, keep the running
    /// transaction's own state and adopt the context's cursor.
    fn resume_context(&mut self, ext: Self) {
        self.replace_while_executing(ext);
    }
}

impl<Tx> FoundryEvmExt<Tx> for () {}

/// Network extension state that can be copied without sharing thread-unsafe state.
///
/// Some extensions, such as Tempo's, share transaction-local state with their precompiles
/// through `Rc`. [`StoredExt`] keeps such state outside a running EVM and across threads.
///
/// # Safety
///
/// [`Self::detached`] must return a value that shares no thread-unsafe state with `self` or any
/// other value, and must not access thread-unsafe state of `self`.
pub unsafe trait SendableExt: Clone {
    /// Returns a copy that shares no thread-unsafe state with `self`.
    fn detached(&self) -> Self {
        self.clone()
    }
}

// SAFETY: `()` holds no state.
unsafe impl SendableExt for () {}

/// Network extension state stored outside a running EVM, such as by a parked context or a
/// snapshot.
///
/// The value enters and leaves only through [`SendableExt::detached`], so it never shares
/// thread-unsafe state with another value and can move between threads.
pub struct StoredExt<E>(E);

impl<E: SendableExt> StoredExt<E> {
    /// Stores a detached copy of `ext`.
    pub fn new(ext: &E) -> Self {
        Self(ext.detached())
    }

    /// Returns a detached copy of the stored state.
    pub fn get(&self) -> E {
        self.0.detached()
    }

    /// Returns the stored state.
    pub fn into_inner(self) -> E {
        self.0
    }
}

impl<E: SendableExt> Clone for StoredExt<E> {
    fn clone(&self) -> Self {
        Self(self.0.detached())
    }
}

impl<E> Debug for StoredExt<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredExt").finish_non_exhaustive()
    }
}

// SAFETY: the stored value is created by `SendableExt::detached`, so no other value shares its
// thread-unsafe state, and shared access only calls `detached`, which reads none of it.
unsafe impl<E: SendableExt> Send for StoredExt<E> {}

// SAFETY: see the `Send` implementation.
unsafe impl<E: SendableExt> Sync for StoredExt<E> {}

/// Network-only transaction outcome flags.
///
/// The shared `TxResult::stop` carries the outcome.
pub trait FoundryTxResultExt: Clone + Debug + Default + 'static {
    /// Returns whether an OP-stack deposit failed and kept only its mint and nonce bump.
    fn is_failed_deposit(&self) -> bool {
        false
    }

    /// Returns the per-phase statuses of an included signed Base EIP-8130 transaction.
    ///
    /// Each entry is `0x01` if the phase committed and `0x00` if it reverted. Other transactions,
    /// including EIP-8130 simulations, return `None`.
    fn eip8130_phase_statuses(&self) -> Option<&[u8]> {
        None
    }
}

impl FoundryTxResultExt for () {}
