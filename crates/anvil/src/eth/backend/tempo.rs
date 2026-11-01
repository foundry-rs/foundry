//! Tempo precompile and fee token initialization for Anvil.
//!
//! When running in Tempo mode, Anvil needs to set up Tempo-specific precompiles
//! and fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD) to enable proper
//! transaction validation.
//!
//! Tempo precompile storage runs on a Tempo evm2 EVM over Anvil's `Db`, using the shared
//! initialization logic from `foundry-evm-core`; the changed state is written back to the `Db`.

use super::{
    db::{AccountInfo, DatabaseRef, Db, KECCAK_EMPTY},
    env::EvmEnv,
    evm::{EvmOptions, TempoEvmNetwork, build_evm},
};
use crate::eth::error::BlockchainError;
use alloy_primitives::{Address, B256, U256, address};
use foundry_evm_core::tempo::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
    initialize_tempo_genesis_inner,
};
use foundry_evm_hardforks::FoundryHardfork;
use tempo_precompiles::{
    TIP_FEE_MANAGER_ADDRESS,
    account_keychain::{
        AccountKeychain,
        IAccountKeychain::{KeyRestrictions, SignatureType},
    },
    error::TempoPrecompileError,
    storage::StorageCtx,
    tip_fee_manager::{IFeeManager, TipFeeManager},
    tip20::{ITIP20, TIP20Token},
};

const SENDER: Address = address!("0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38");
const ADMIN: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");

/// State changed by a Tempo storage context.
#[derive(Debug, Default)]
struct TempoChanges {
    accounts: Vec<(Address, AccountInfo)>,
    storage: Vec<(Address, U256, U256)>,
}

/// Runs `f` in a Tempo precompile storage context over `db` at `env`, then writes the state it
/// changed back to `db`.
///
/// The context executes on a Tempo evm2 EVM, so precompile storage follows the same rules as
/// transactions.
pub fn with_tempo_storage<R>(
    db: &mut dyn Db,
    env: &EvmEnv,
    hardfork: FoundryHardfork,
    f: impl FnOnce() -> R,
) -> Result<R, BlockchainError> {
    let opts = EvmOptions { hardfork, ..Default::default() };
    let (result, changes) = {
        let state: &dyn DatabaseRef = &*db;
        let mut evm = build_evm::<TempoEvmNetwork>(state, env, &opts)?;
        let result = StorageCtx::enter_evm(&mut evm, f);
        evm.state_mut().commit_transaction();
        let changes = overlay_changes(state, &evm.overlay_db().cache)?;
        (result, changes)
    };
    for (address, info) in changes.accounts {
        db.insert_account(address, info);
    }
    for (address, slot, value) in changes.storage {
        db.set_storage_at(address, B256::from(slot), B256::from(value))?;
    }
    Ok(result)
}

/// Returns the accounts and slots of `cache` that differ from `db`.
fn overlay_changes(
    db: &dyn DatabaseRef,
    cache: &evm2::evm::Cache,
) -> Result<TempoChanges, BlockchainError> {
    let mut changes = TempoChanges::default();
    for (address, info) in &cache.accounts {
        let Some(info) = info else { continue };
        let current = db.basic_ref(*address)?;
        if current.as_ref().is_some_and(|current| {
            current.balance == info.balance
                && current.nonce == info.nonce
                && current.code_hash == info.code_hash
        }) {
            continue;
        }
        let mut info = info.clone();
        if info.code.is_none() && info.code_hash != KECCAK_EMPTY {
            info.code = cache.contracts.get(&info.code_hash).cloned();
        }
        changes.accounts.push((*address, info));
    }
    for (address, slots) in &cache.storage {
        for (slot, value) in &slots.slots {
            if db.storage_ref(*address, *slot)? != *value {
                changes.storage.push((*address, *slot, *value));
            }
        }
    }
    Ok(changes)
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
    env: &EvmEnv,
    test_accounts: &[Address],
    hardfork: FoundryHardfork,
) -> Result<(), BlockchainError> {
    with_tempo_storage(db, env, hardfork, || initialize_tempo_genesis(test_accounts, hardfork))?
        .map_err(|err| BlockchainError::Message(format!("failed to initialize Tempo: {err}")))
}

fn initialize_tempo_genesis(
    test_accounts: &[Address],
    hardfork: FoundryHardfork,
) -> Result<(), TempoPrecompileError> {
    // Initialize base Tempo genesis (precompiles and tokens)
    initialize_tempo_genesis_inner(ADMIN, SENDER, hardfork.into())?;

    // Mint fee tokens to test accounts
    // u64::MAX per account - safe since u128::MAX can hold ~18 quintillion u64::MAX values
    let mint_amount = U256::from(u64::MAX);
    let tokens = [PATH_USD_ADDRESS, ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, THETA_USD_ADDRESS];

    (|| -> Result<(), TempoPrecompileError> {
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
                account, // key ID = account address for secp256k1
                SignatureType::Secp256k1,
                KeyRestrictions {
                    expiry: u64::MAX,     // never expires
                    enforceLimits: false, // no spending limits
                    limits: vec![],
                    allowAnyCalls: true,
                    allowedCalls: vec![],
                },
                None,
            )?;
        }

        // Initialize TipFeeManager and set default fee tokens for test accounts
        // Alice (0) -> AlphaUSD, Bob (1) -> BetaUSD, Charlie (2) -> ThetaUSD, others -> PathUSD
        let mut fee_manager = TipFeeManager::new();
        fee_manager.initialize()?;

        for (i, &account) in test_accounts.iter().enumerate() {
            let fee_token = match i {
                0 => ALPHA_USD_ADDRESS, // Alice
                1 => BETA_USD_ADDRESS,  // Bob
                2 => THETA_USD_ADDRESS, // Charlie
                _ => PATH_USD_ADDRESS,  // Everyone else
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
    })()
}
