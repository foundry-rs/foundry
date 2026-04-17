//! Tempo precompile and fee token initialization for Anvil.
//!
//! When running in Tempo mode, Anvil needs to set up Tempo-specific precompiles
//! and fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD) to enable proper
//! transaction validation.
//!
//! This module provides a storage provider adapter for Anvil's `Db` trait and
//! uses the shared initialization logic from `foundry-evm-core`.

use alloy_primitives::{Address, U256, address};
use foundry_evm::core::tempo::{PATH_USD_ADDRESS, initialize_tempo_genesis};
use revm::{
    context::journaled_state::JournalCheckpoint,
    state::{AccountInfo, Bytecode},
};
use std::collections::HashMap;
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_precompiles::{
    TIP_FEE_MANAGER_ADDRESS,
    account_keychain::{
        AccountKeychain,
        IAccountKeychain::{KeyRestrictions, SignatureType},
        authorizeKeyCall,
    },
    error::TempoPrecompileError,
    storage::{PrecompileStorageProvider, StorageCtx},
    tip_fee_manager::{IFeeManager, TipFeeManager},
    tip20::{ITIP20, TIP20Token},
};

use super::db::Db;

/// Sender address used for genesis initialization.
const SENDER: Address = address!("0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38");
/// Admin address used for genesis initialization.
const ADMIN: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");

const PATH_USD: Address = PATH_USD_ADDRESS;
const ALPHA_USD: Address = address!("0x20C0000000000000000000000000000000000001");
const BETA_USD: Address = address!("0x20C0000000000000000000000000000000000002");
const THETA_USD: Address = address!("0x20C0000000000000000000000000000000000003");

/// Storage provider adapter for Anvil's Db to work with Tempo precompiles.
pub struct AnvilStorageProvider<'a> {
    db: &'a mut dyn Db,
    chain_id: u64,
    timestamp: U256,
    block_number: u64,
    gas_used: u64,
    gas_refunded: i64,
    reservoir: u64,
    transient: HashMap<(Address, U256), U256>,
    hardfork: TempoHardfork,
}

impl<'a> AnvilStorageProvider<'a> {
    pub fn new(
        db: &'a mut dyn Db,
        chain_id: u64,
        timestamp: U256,
        block_number: u64,
        hardfork: TempoHardfork,
    ) -> Self {
        Self {
            db,
            chain_id,
            timestamp,
            block_number,
            gas_used: 0,
            gas_refunded: 0,
            reservoir: 0,
            transient: HashMap::new(),
            hardfork,
        }
    }
}

impl PrecompileStorageProvider for AnvilStorageProvider<'_> {
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
        self.db.insert_account(
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
        use revm::DatabaseRef;
        if let Some(info) =
            self.db.basic_ref(address).map_err(|e| TempoPrecompileError::Fatal(e.to_string()))?
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
        use alloy_primitives::B256;
        self.db
            .set_storage_at(address, B256::from(key), B256::from(value))
            .map_err(|e| TempoPrecompileError::Fatal(e.to_string()))
    }

    fn sload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        revm::Database::storage(self.db, address, key)
            .map_err(|e| TempoPrecompileError::Fatal(e.to_string()))
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

    fn state_gas_used(&self) -> u64 {
        0
    }

    fn gas_limit(&self) -> u64 {
        u64::MAX
    }

    fn gas_refunded(&self) -> i64 {
        self.gas_refunded
    }

    fn reservoir(&self) -> u64 {
        self.reservoir
    }

    fn refund_gas(&mut self, gas: i64) {
        self.gas_refunded = self.gas_refunded.saturating_add(gas);
    }

    fn beneficiary(&self) -> Address {
        Address::ZERO
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

/// Initialize Tempo precompiles and fee tokens for Anvil.
///
/// This sets up the same precompiles and tokens as Tempo's genesis, enabling
/// proper fee token validation for transactions.
///
/// Additionally, mints fee tokens to the provided test accounts so they can
/// send transactions in Tempo mode.
pub fn initialize_tempo_precompiles(
    db: &mut dyn Db,
    chain_id: u64,
    timestamp: u64,
    test_accounts: &[Address],
    hardfork: TempoHardfork,
) -> Result<(), TempoPrecompileError> {
    let timestamp = U256::from(timestamp);

    let mut storage = AnvilStorageProvider::new(db, chain_id, timestamp, 0, hardfork);

    // Initialize base Tempo genesis (precompiles and tokens)
    initialize_tempo_genesis(&mut storage, ADMIN, SENDER)?;

    // Mint fee tokens to test accounts
    // u64::MAX per account - safe since u128::MAX can hold ~18 quintillion u64::MAX values
    let mint_amount = U256::from(u64::MAX);
    let tokens = [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD];

    StorageCtx::enter(&mut storage, || -> Result<(), TempoPrecompileError> {
        // Mint fee tokens to test accounts
        for &token_address in &tokens {
            let mut token = TIP20Token::from_address(token_address)?;
            for &account in test_accounts {
                token.mint(ADMIN, ITIP20::mintCall { to: account, amount: mint_amount })?;
            }
        }

        // Register secp256k1 keys for test accounts in the AccountKeychain
        // This allows them to sign Tempo transactions using their private keys.
        // The key ID is the account address itself (standard for secp256k1 keys).
        let mut keychain = AccountKeychain::new();
        for &account in test_accounts {
            // Seed tx_origin so ensure_admin_caller passes on T2+ (requires
            // tx_origin != zero && tx_origin == msg_sender).
            keychain.set_tx_origin(account)?;
            keychain.authorize_key(
                account, // msg_sender (root account authorizes its own key)
                authorizeKeyCall {
                    keyId: account, // key ID = account address for secp256k1
                    signatureType: SignatureType::Secp256k1,
                    config: KeyRestrictions {
                        expiry: u64::MAX,     // never expires
                        enforceLimits: false, // no spending limits
                        limits: vec![],
                        allowAnyCalls: true,
                        allowedCalls: vec![],
                    },
                },
            )?;
        }

        // Initialize TipFeeManager and set default fee tokens for test accounts
        // Alice (0) -> AlphaUSD, Bob (1) -> BetaUSD, Charlie (2) -> ThetaUSD, others -> PathUSD
        let mut fee_manager = TipFeeManager::new();
        fee_manager.initialize()?;

        for (i, &account) in test_accounts.iter().enumerate() {
            let fee_token = match i {
                0 => ALPHA_USD, // Alice
                1 => BETA_USD,  // Bob
                2 => THETA_USD, // Charlie
                _ => PATH_USD,  // Everyone else
            };
            fee_manager
                .set_user_token(account, IFeeManager::setUserTokenCall { token: fee_token })?;
        }

        // Mint fee tokens to the FeeManager contract for liquidity operations
        for &token_address in &tokens {
            let mut token = TIP20Token::from_address(token_address)?;
            token.mint(
                ADMIN,
                ITIP20::mintCall { to: TIP_FEE_MANAGER_ADDRESS, amount: mint_amount },
            )?;
        }

        // Mint pairwise FeeAMM liquidity for all fee token pairs (both directions)
        // This enables EIP-1559/legacy transactions by allowing fee swaps between tokens
        // Liquidity amount: 10^10 tokens (matching Tempo genesis)
        let liquidity_amount = U256::from(10u64.pow(10));

        // Create bidirectional liquidity pools between all fee tokens
        // Pools are directional: user_token -> validator_token
        for &user_token in &tokens {
            for &validator_token in &tokens {
                if user_token != validator_token {
                    fee_manager.mint(
                        ADMIN,
                        user_token,
                        validator_token,
                        liquidity_amount,
                        ADMIN,
                    )?;
                }
            }
        }

        Ok(())
    })?;

    Ok(())
}
