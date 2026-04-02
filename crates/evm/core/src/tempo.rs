//! Tempo precompile and contract initialization for Foundry.
//!
//! This module provides the core initialization logic for Tempo-specific precompiles,
//! fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD), and standard contracts.
//!
//! It includes a storage provider adapter for Foundry's `Backend` and the shared
//! genesis initialization function used by both anvil and forge.

use std::collections::HashMap;

use alloy_primitives::{Address, Bytes, U256, address};
use revm::{
    Database,
    context::journaled_state::JournalCheckpoint,
    state::{AccountInfo, Bytecode},
};
use tempo_chainspec::hardfork::TempoHardfork;
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

use crate::backend::Backend;

pub use tempo_contracts::precompiles::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
};

/// All well-known TIP20 fee token addresses on Tempo networks.
pub const TEMPO_TIP20_TOKENS: &[Address] =
    &[PATH_USD_ADDRESS, ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, THETA_USD_ADDRESS];

/// Storage provider adapter that wraps Foundry's `Backend` to implement Tempo's
/// [`PrecompileStorageProvider`] trait for precompile initialization.
pub struct TempoStorageProvider<'a> {
    backend: &'a mut Backend,
    chain_id: u64,
    timestamp: U256,
    block_number: u64,
    gas_used: u64,
    gas_refunded: i64,
    transient: HashMap<(Address, U256), U256>,
    beneficiary: Address,
    hardfork: TempoHardfork,
}

impl<'a> TempoStorageProvider<'a> {
    pub fn new(
        backend: &'a mut Backend,
        chain_id: u64,
        timestamp: U256,
        block_number: u64,
        hardfork: TempoHardfork,
    ) -> Self {
        Self {
            backend,
            chain_id,
            timestamp,
            block_number,
            gas_used: 0,
            gas_refunded: 0,
            transient: HashMap::new(),
            beneficiary: Address::ZERO,
            hardfork,
        }
    }
}

impl PrecompileStorageProvider for TempoStorageProvider<'_> {
    fn spec(&self) -> TempoHardfork {
        self.hardfork
    }

    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn timestamp(&self) -> U256 {
        self.timestamp
    }

    fn block_number(&self) -> u64 {
        self.block_number
    }

    fn set_code(&mut self, address: Address, code: Bytecode) -> Result<(), TempoPrecompileError> {
        self.backend.insert_account_info(
            address,
            AccountInfo {
                code_hash: code.hash_slow(),
                code: Some(code),
                nonce: 1,
                ..Default::default()
            },
        );
        Ok(())
    }

    fn with_account_info(
        &mut self,
        address: Address,
        f: &mut dyn FnMut(&AccountInfo),
    ) -> Result<(), TempoPrecompileError> {
        if let Some(info) =
            self.backend.basic(address).map_err(|e| TempoPrecompileError::Fatal(e.to_string()))?
        {
            f(&info);
            Ok(())
        } else {
            Err(TempoPrecompileError::Fatal(format!("account '{address}' not found")))
        }
    }

    fn sstore(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), TempoPrecompileError> {
        self.backend
            .insert_account_storage(address, key, value)
            .map_err(|e| TempoPrecompileError::Fatal(e.to_string()))
    }

    fn sload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        self.backend.storage(address, key).map_err(|e| TempoPrecompileError::Fatal(e.to_string()))
    }

    fn tstore(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), TempoPrecompileError> {
        self.transient.insert((address, key), value);
        Ok(())
    }

    fn tload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        Ok(self.transient.get(&(address, key)).copied().unwrap_or(U256::ZERO))
    }

    fn emit_event(
        &mut self,
        _address: Address,
        _event: alloy_primitives::LogData,
    ) -> Result<(), TempoPrecompileError> {
        Ok(())
    }

    fn deduct_gas(&mut self, gas: u64) -> Result<(), TempoPrecompileError> {
        self.gas_used = self.gas_used.saturating_add(gas);
        Ok(())
    }

    fn gas_used(&self) -> u64 {
        self.gas_used
    }

    fn gas_refunded(&self) -> i64 {
        self.gas_refunded
    }

    fn refund_gas(&mut self, gas: i64) {
        self.gas_refunded = self.gas_refunded.saturating_add(gas);
    }

    fn beneficiary(&self) -> Address {
        self.beneficiary
    }

    fn is_static(&self) -> bool {
        false
    }

    fn checkpoint(&mut self) -> JournalCheckpoint {
        JournalCheckpoint { log_i: 0, journal_i: 0, selfdestructed_i: 0 }
    }

    fn checkpoint_commit(&mut self, _checkpoint: JournalCheckpoint) {}

    fn checkpoint_revert(&mut self, _checkpoint: JournalCheckpoint) {}
}

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
    StorageCtx::enter(storage, || -> Result<(), TempoPrecompileError> {
        let mut ctx = StorageCtx;

        // Set sentinel bytecode for precompile addresses
        let sentinel = Bytecode::new_legacy(Bytes::from_static(&[0xef]));
        for precompile in [
            NONCE_PRECOMPILE_ADDRESS,
            STABLECOIN_DEX_ADDRESS,
            TIP20_FACTORY_ADDRESS,
            TIP403_REGISTRY_ADDRESS,
            TIP_FEE_MANAGER_ADDRESS,
            VALIDATOR_CONFIG_ADDRESS,
            VALIDATOR_CONFIG_V2_ADDRESS,
            ACCOUNT_KEYCHAIN_ADDRESS,
        ] {
            ctx.set_code(precompile, sentinel.clone())?;
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
        ctx.sstore(
            VALIDATOR_CONFIG_ADDRESS,
            validator_config::slots::OWNER,
            admin.into_word().into(),
        )?;

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
    })?;

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
