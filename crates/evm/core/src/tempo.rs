//! Tempo precompile and contract initialization for Foundry.
//!
//! This module provides the core initialization logic for Tempo-specific precompiles,
//! fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD), and standard contracts.
//!
//! It includes the shared genesis initialization function used by both anvil and forge.

use alloy_primitives::{Address, Bytes, U256, address};
use revm::state::Bytecode;
use tempo_contracts::{
    ARACHNID_CREATE2_FACTORY_ADDRESS, CREATEX_ADDRESS, CreateX, MULTICALL3_ADDRESS, Multicall3,
    PERMIT2_ADDRESS, Permit2, SAFE_DEPLOYER_ADDRESS, SafeDeployer,
    contracts::ARACHNID_CREATE2_FACTORY_BYTECODE,
};
use tempo_precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, NONCE_PRECOMPILE_ADDRESS, STABLECOIN_DEX_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS, TIP20_FACTORY_ADDRESS, TIP403_REGISTRY_ADDRESS,
    VALIDATOR_CONFIG_ADDRESS, VALIDATOR_CONFIG_V2_ADDRESS,
    error::TempoPrecompileError,
    storage::{PrecompileStorageProvider, StorageCtx},
    tip20::{ISSUER_ROLE, ITIP20, TIP20Token},
    tip20_factory::TIP20Factory,
    validator_config,
};

pub use tempo_contracts::precompiles::PATH_USD_ADDRESS;

// TODO: remove once we can re-export from tempo_precompiles instead.
pub const SIGNATURE_VERIFIER_ADDRESS: Address =
    address!("0x5165300000000000000000000000000000000000");
pub const ADDRESS_REGISTRY_ADDRESS: Address =
    address!("0xFDC0000000000000000000000000000000000000");

/// All well-known Tempo precompile addresses.
pub const TEMPO_PRECOMPILE_ADDRESSES: &[Address] = &[
    NONCE_PRECOMPILE_ADDRESS,
    STABLECOIN_DEX_ADDRESS,
    TIP20_FACTORY_ADDRESS,
    TIP403_REGISTRY_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS,
    VALIDATOR_CONFIG_ADDRESS,
    VALIDATOR_CONFIG_V2_ADDRESS,
    ACCOUNT_KEYCHAIN_ADDRESS,
    SIGNATURE_VERIFIER_ADDRESS,
    ADDRESS_REGISTRY_ADDRESS,
];

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
    StorageCtx::enter(storage, || initialize_tempo_genesis_inner(admin, recipient))
}

/// Inner genesis initialization logic. Must be called within a [`StorageCtx`] scope
/// (either via [`StorageCtx::enter`] or [`StorageCtx::enter_evm`]).
pub fn initialize_tempo_genesis_inner(
    admin: Address,
    recipient: Address,
) -> Result<(), TempoPrecompileError> {
    let mut ctx = StorageCtx;

    // Set sentinel bytecode for precompile addresses
    let sentinel = Bytecode::new_legacy(Bytes::from_static(&[0xef]));
    for precompile in TEMPO_PRECOMPILE_ADDRESSES {
        ctx.set_code(*precompile, sentinel.clone())?;
    }

    // Create PathUSD token: 0x20C0000000000000000000000000000000000000
    let path_usd_token_address = create_and_mint_token(
        address!("20C0000000000000000000000000000000000000"),
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
