//! Tempo precompile and contract initialization for Foundry.
//!
//! This module provides the core initialization logic for Tempo-specific precompiles,
//! fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD), and standard contracts.
//!
//! It includes the shared genesis initialization function used by both anvil and forge.

use alloy_primitives::{Address, Bytes, Log, U256, address};
use revm::{
    Inspector,
    context::{
        ContextTr, JournalTr,
        either::Either,
        transaction::{
            Authorization, RecoveredAuthority, RecoveredAuthorization, SignedAuthorization,
        },
    },
    context_interface::CreateScheme,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Gas, InstructionResult, Interpreter,
        InterpreterResult, InterpreterTypes,
    },
    state::Bytecode,
};
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_contracts::{
    ARACHNID_CREATE2_FACTORY_ADDRESS, CREATEX_ADDRESS, CreateX, MULTICALL3_ADDRESS, Multicall3,
    PERMIT2_ADDRESS, Permit2, SAFE_DEPLOYER_ADDRESS, SafeDeployer,
    contracts::ARACHNID_CREATE2_FACTORY_BYTECODE, precompiles::VALIDATOR_CONFIG_ADDRESS,
};
use tempo_precompiles::{
    error::TempoPrecompileError,
    storage::{PrecompileStorageProvider, StorageCtx},
    tip20::{ISSUER_ROLE, ITIP20, TIP20Token},
    tip20_factory::TIP20Factory,
    validator_config,
};
use tempo_primitives::transaction::RecoveredTempoAuthorization;

pub use foundry_evm_networks::{
    TEMPO_PRECOMPILE_ADDRESSES, active_tempo_precompile_addresses, is_tempo_precompile_active_at,
};
pub use tempo_contracts::precompiles::{
    ADDRESS_REGISTRY_ADDRESS, IAddressRegistry, IFeeManager, ISignatureVerifier, IStablecoinDEX,
    ITIP20ChannelReserve, PATH_USD_ADDRESS, RECEIVE_POLICY_GUARD_ADDRESS,
    SIGNATURE_VERIFIER_ADDRESS, STABLECOIN_DEX_ADDRESS, TIP_FEE_MANAGER_ADDRESS,
    TIP20_CHANNEL_RESERVE_ADDRESS, TIP20_FACTORY_ADDRESS,
};
pub use tempo_precompiles::{
    address_registry::{AddressRegistry, IMPLICIT_APPROVAL_LIST, is_implicitly_approved},
    signature_verifier::SignatureVerifier,
    stablecoin_dex::StablecoinDEX,
    tip_fee_manager::TipFeeManager,
    tip20::is_tip20_prefix,
    tip20_channel_reserve::TIP20ChannelReserve,
};

/// All well-known TIP20 fee token addresses on Tempo networks.
pub const TEMPO_TIP20_TOKENS: &[Address] = &[PATH_USD_ADDRESS];

/// Initialize Tempo precompiles and contracts using a storage provider.
///
/// This is the core initialization logic that sets up Tempo-specific precompiles,
/// fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD), and standard contracts.
///
/// This function should be called during genesis setup when running in Tempo mode.
/// It uses the `StorageCtx` pattern to work with any storage backend that implements
/// `PrecompileStorageProvider`.
///
/// # Arguments
/// * `storage` - A mutable reference to a storage provider implementing `PrecompileStorageProvider`
/// * `admin` - The admin address that will have control over tokens and config
/// * `recipient` - The address that will receive minted tokens
///
/// Ref: <https://github.com/tempoxyz/tempo/blob/main/xtask/src/genesis_args.rs>
pub fn initialize_tempo_genesis(
    storage: &mut impl PrecompileStorageProvider,
    admin: Address,
    recipient: Address,
) -> Result<(), TempoPrecompileError> {
    initialize_tempo_genesis_at_hardfork(storage, admin, recipient, TempoHardfork::default())
}

/// Initialize Tempo precompiles and contracts for a specific active hardfork.
pub fn initialize_tempo_genesis_at_hardfork(
    storage: &mut impl PrecompileStorageProvider,
    admin: Address,
    recipient: Address,
    hardfork: TempoHardfork,
) -> Result<(), TempoPrecompileError> {
    StorageCtx::enter(storage, || initialize_tempo_genesis_inner(admin, recipient, hardfork))
}

/// Inner genesis initialization logic. Must be called within a [`StorageCtx`] scope
/// (either via [`StorageCtx::enter`] or [`StorageCtx::enter_evm`]).
pub fn initialize_tempo_genesis_inner(
    admin: Address,
    recipient: Address,
    hardfork: TempoHardfork,
) -> Result<(), TempoPrecompileError> {
    // Idempotent: PATH_USD is the first token created during genesis; if it already exists, skip.
    if TIP20Factory::new().is_tip20(PATH_USD_ADDRESS)? {
        return Ok(());
    }

    let mut ctx = StorageCtx;

    // Set sentinel bytecode for precompile addresses
    let sentinel = Bytecode::new_legacy(Bytes::from_static(&[0xef]));
    for precompile in active_tempo_precompile_addresses(hardfork) {
        ctx.set_code(precompile, sentinel.clone())?;
    }

    // Create PathUSD token: 0x20C0000000000000000000000000000000000000
    let path_usd_token_address = create_and_mint_token(
        PATH_USD_ADDRESS,
        "PathUSD",
        "PathUSD",
        "USD",
        Address::ZERO,
        admin,
        recipient,
        U256::from(u64::MAX),
    )?;

    // Create AlphaUSD token: 0x20C0000000000000000000000000000000000001
    let _alpha_usd_token_address = create_and_mint_token(
        address!("20C0000000000000000000000000000000000001"),
        "AlphaUSD",
        "AlphaUSD",
        "USD",
        path_usd_token_address,
        admin,
        recipient,
        U256::from(u64::MAX),
    )?;

    // Create BetaUSD token: 0x20C0000000000000000000000000000000000002
    let _beta_usd_token_address = create_and_mint_token(
        address!("20C0000000000000000000000000000000000002"),
        "BetaUSD",
        "BetaUSD",
        "USD",
        path_usd_token_address,
        admin,
        recipient,
        U256::from(u64::MAX),
    )?;

    // Create ThetaUSD token: 0x20C0000000000000000000000000000000000003
    let _theta_usd_token_address = create_and_mint_token(
        address!("20C0000000000000000000000000000000000003"),
        "ThetaUSD",
        "ThetaUSD",
        "USD",
        path_usd_token_address,
        admin,
        recipient,
        U256::from(u64::MAX),
    )?;

    // Initialize ValidatorConfig with admin as owner
    ctx.sstore(VALIDATOR_CONFIG_ADDRESS, validator_config::slots::OWNER, admin.into_word().into())?;

    // Set bytecode for standard contracts
    ctx.set_code(
        MULTICALL3_ADDRESS,
        Bytecode::new_legacy(Bytes::from_static(&Multicall3::DEPLOYED_BYTECODE)),
    )?;
    ctx.set_code(
        CREATEX_ADDRESS,
        Bytecode::new_legacy(Bytes::from_static(&CreateX::DEPLOYED_BYTECODE)),
    )?;
    ctx.set_code(
        SAFE_DEPLOYER_ADDRESS,
        Bytecode::new_legacy(Bytes::from_static(&SafeDeployer::DEPLOYED_BYTECODE)),
    )?;
    ctx.set_code(
        PERMIT2_ADDRESS,
        Bytecode::new_legacy(Bytes::from_static(&Permit2::DEPLOYED_BYTECODE)),
    )?;
    ctx.set_code(
        ARACHNID_CREATE2_FACTORY_ADDRESS,
        Bytecode::new_legacy(ARACHNID_CREATE2_FACTORY_BYTECODE),
    )?;

    Ok(())
}

/// Helper function to create and mint a TIP20 token.
#[allow(clippy::too_many_arguments)]
fn create_and_mint_token(
    address: Address,
    symbol: &str,
    name: &str,
    currency: &str,
    quote_token: Address,
    admin: Address,
    recipient: Address,
    mint_amount: U256,
) -> Result<Address, TempoPrecompileError> {
    let mut tip20_factory = TIP20Factory::new();

    let token_address = tip20_factory.create_token_reserved_address(
        address,
        name,
        symbol,
        currency,
        quote_token,
        admin,
    )?;

    let mut token = TIP20Token::from_address(token_address)?;
    token.grant_role_internal(admin, *ISSUER_ROLE)?;
    token.mint(admin, ITIP20::mintCall { to: recipient, amount: mint_amount })?;
    if admin != recipient {
        token.mint(admin, ITIP20::mintCall { to: admin, amount: mint_amount })?;
    }

    Ok(token_address)
}

// TIP-1047 prefix guard.

/// Predicts the address a CREATE / CREATE2 frame would deploy to, using the
/// caller's pre-bump nonce. Returns `None` for `CreateScheme::Custom`.
pub fn predicted_create_address(inputs: &CreateInputs, caller_nonce: u64) -> Option<Address> {
    match inputs.scheme() {
        CreateScheme::Create | CreateScheme::Create2 { .. } => {
            Some(inputs.created_address(caller_nonce))
        }
        CreateScheme::Custom { .. } => None,
    }
}

/// Revert `CreateOutcome` for the prefix guard; short-circuits revm before
/// nonce bump, value transfer, and init-code execution.
pub fn tip20_prefix_revert(inputs: &CreateInputs, addr: Address) -> CreateOutcome {
    CreateOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from(
                format!("TIP-20 prefix create address forbidden: {addr}").into_bytes(),
            ),
            gas: Gas::new_with_regular_gas_and_reservoir(inputs.gas_limit(), inputs.reservoir()),
        },
        address: None,
    }
}

/// Whether `addr` is rejected by the prefix guard. Honors
/// [`test_seam::TIP20_PREFIX_OVERRIDE`] for tests.
pub fn is_tip20_prefix_for_guard(addr: Address) -> bool {
    if let Some(p) = test_seam::TIP20_PREFIX_OVERRIDE.with(|c| c.get()) {
        return addr.as_slice().starts_with(&p);
    }
    is_tip20_prefix(addr)
}

/// Marks prefix-colliding EIP-7702 entries as `RecoveredAuthority::Invalid`,
/// preserving list length so intrinsic gas is unaffected.
pub fn mask_tip20_prefixed_eth_authorizations(
    auths: &mut [Either<SignedAuthorization, RecoveredAuthorization>],
) {
    for slot in auths.iter_mut() {
        let needs_mask = match slot {
            Either::Left(signed) => {
                signed.recover_authority().ok().is_some_and(is_tip20_prefix_for_guard)
            }
            Either::Right(recovered) => {
                recovered.authority().is_some_and(is_tip20_prefix_for_guard)
            }
        };
        if !needs_mask {
            continue;
        }
        let inner = match slot {
            Either::Left(signed) => Authorization {
                chain_id: signed.inner().chain_id,
                address: signed.inner().address,
                nonce: signed.inner().nonce,
            },
            Either::Right(recovered) => Authorization {
                chain_id: recovered.chain_id,
                address: recovered.address,
                nonce: recovered.nonce,
            },
        };
        *slot = Either::Right(RecoveredAuthorization::new_unchecked(
            inner,
            RecoveredAuthority::Invalid,
        ));
    }
}

/// Same as [`mask_tip20_prefixed_eth_authorizations`] for the Tempo AA list.
pub fn mask_tip20_prefixed_tempo_authorizations(
    auths: &mut [tempo_primitives::transaction::RecoveredTempoAuthorization],
) {
    for slot in auths.iter_mut() {
        if !slot.authority().is_some_and(is_tip20_prefix_for_guard) {
            continue;
        }
        let signed = slot.signed().clone();
        *slot = RecoveredTempoAuthorization::new_unchecked(signed, RecoveredAuthority::Invalid);
    }
}

/// Masks both the standard EIP-7702 list and the Tempo AA list of a `TempoTxEnv`.
pub fn mask_tip20_prefixed_authorizations(tx_env: &mut tempo_revm::TempoTxEnv) {
    mask_tip20_prefixed_eth_authorizations(&mut tx_env.inner.authorization_list);
    if let Some(aa_env) = tx_env.tempo_tx_env.as_mut() {
        mask_tip20_prefixed_tempo_authorizations(&mut aa_env.tempo_authorization_list);
    }
}

/// Wraps any inspector to add the TIP-1047 CREATE / CREATE2 guard, delegating all
/// other hooks to inner. Use on Tempo T5+ replay/debug paths whose inspectors
/// (e.g. `TracingInspector`, `JsInspector`) don't carry the guard themselves.
pub struct Tip1047CreateGuard<'a, I> {
    inner: &'a mut I,
}

impl<'a, I> Tip1047CreateGuard<'a, I> {
    pub const fn new(inner: &'a mut I) -> Self {
        Self { inner }
    }
}

impl<CTX, INTR, I> Inspector<CTX, INTR> for Tip1047CreateGuard<'_, I>
where
    CTX: ContextTr,
    INTR: InterpreterTypes,
    I: Inspector<CTX, INTR>,
{
    fn initialize_interp(&mut self, interp: &mut Interpreter<INTR>, ctx: &mut CTX) {
        self.inner.initialize_interp(interp, ctx);
    }
    fn step(&mut self, interp: &mut Interpreter<INTR>, ctx: &mut CTX) {
        self.inner.step(interp, ctx);
    }
    fn step_end(&mut self, interp: &mut Interpreter<INTR>, ctx: &mut CTX) {
        self.inner.step_end(interp, ctx);
    }
    fn log(&mut self, ctx: &mut CTX, log: Log) {
        self.inner.log(ctx, log);
    }
    fn log_full(&mut self, interp: &mut Interpreter<INTR>, ctx: &mut CTX, log: Log) {
        self.inner.log_full(interp, ctx, log);
    }
    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.inner.call(ctx, inputs)
    }
    fn call_end(&mut self, ctx: &mut CTX, inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.inner.call_end(ctx, inputs, outcome);
    }
    fn create(&mut self, ctx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        // Inner first so traces still see the attempted CREATE/CREATE2.
        if let Some(out) = self.inner.create(ctx, inputs) {
            return Some(out);
        }
        let caller = inputs.caller();
        let nonce = ctx.journal_mut().load_account(caller).ok().map(|a| a.info.nonce);
        if let Some(n) = nonce
            && let Some(addr) = predicted_create_address(inputs, n)
            && is_tip20_prefix_for_guard(addr)
        {
            return Some(tip20_prefix_revert(inputs, addr));
        }
        None
    }
    fn create_end(&mut self, ctx: &mut CTX, inputs: &CreateInputs, outcome: &mut CreateOutcome) {
        self.inner.create_end(ctx, inputs, outcome);
    }
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        Inspector::<CTX, INTR>::selfdestruct(self.inner, contract, target, value);
    }
}

/// Test-only seam: override the prefix used by [`is_tip20_prefix_for_guard`].
/// Default `None`; exposed for cross-crate tests, not a stable API.
#[doc(hidden)]
pub mod test_seam {
    use std::cell::Cell;

    thread_local! {
        pub static TIP20_PREFIX_OVERRIDE: Cell<Option<[u8; 12]>> = const { Cell::new(None) };
    }

    /// RAII handle: installs an override on construction, restores on drop.
    pub struct OverrideGuard {
        prev: Option<[u8; 12]>,
    }

    impl OverrideGuard {
        pub fn new(prefix: [u8; 12]) -> Self {
            let prev = TIP20_PREFIX_OVERRIDE.with(|c| {
                let prev = c.get();
                c.set(Some(prefix));
                prev
            });
            Self { prev }
        }
    }

    impl Drop for OverrideGuard {
        fn drop(&mut self) {
            let prev = self.prev;
            TIP20_PREFIX_OVERRIDE.with(|c| c.set(prev));
        }
    }
}
