//! Tempo precompile and fee token initialization for Anvil.
//!
//! When running in Tempo mode, Anvil needs to set up Tempo-specific precompiles
//! and fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD) to enable proper
//! transaction validation.
//!
//! Tempo precompile storage runs on a Tempo evm2 EVM over Anvil's `Db`, using the shared
//! initialization logic from `foundry-evm-core`; the changed state is written back to the `Db`.

use super::{
    db::{DatabaseRef, Db, EvmDb},
    env::EvmEnv,
    evm::{EvmOptions, TempoEvmNetwork, build_evm},
};
use crate::eth::error::BlockchainError;
use alloy_primitives::{Address, U256, address};
use anvil_core::types::TempoCreateTokenRequest;
use evm2::evm::{AccountChangeRef, BlockStateAccumulator, StateChangeSink, StorageChange};
use foundry_evm_core::tempo::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
    initialize_tempo_genesis_inner,
};
use foundry_evm_hardforks::FoundryHardfork;
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, ADDRESS_REGISTRY_ADDRESS, CONFIGURABLE_ACCOUNT_ADDRESS,
    CURRENT_COMMITTEE_ADDRESS, NONCE_PRECOMPILE_ADDRESS, RECEIVE_POLICY_GUARD_ADDRESS,
    SIGNATURE_VERIFIER_ADDRESS, STABLECOIN_DEX_ADDRESS, STORAGE_CREDITS_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS, TIP20_CHANNEL_RESERVE_ADDRESS, TIP20_FACTORY_ADDRESS,
    TIP403_REGISTRY_ADDRESS, VALIDATOR_CONFIG_ADDRESS, VALIDATOR_CONFIG_V2_ADDRESS,
    ZONE_FACTORY_ADDRESS, ZONE_VERIFIER_ADDRESS,
    account_keychain::{
        AccountKeychain,
        IAccountKeychain::{KeyRestrictions, SignatureType},
    },
    error::TempoPrecompileError,
    storage::StorageCtx,
    tip_fee_manager::{IFeeManager, TipFeeManager},
    tip20::{IRolesAuth, ITIP20, TIP20Error, TIP20Token, USD_CURRENCY, roles::UNGRANTABLE_ROLE},
    tip20_factory::{TIP20Factory, TIP20FactoryError},
};
use tempo_primitives::TempoAddressExt;

const SENDER: Address = address!("0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38");
const ADMIN: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");

/// Runs `f` in a Tempo precompile storage context over `db` at `env`, then writes the state it
/// changed back to `db` only if every operation succeeds.
///
/// The context executes on a Tempo evm2 EVM, so precompile storage follows the same rules as
/// transactions.
pub fn with_tempo_storage<R>(
    db: &mut dyn Db,
    env: &EvmEnv,
    hardfork: FoundryHardfork,
    f: impl FnOnce() -> Result<R, BlockchainError>,
) -> Result<R, BlockchainError> {
    let opts = EvmOptions { hardfork, ..Default::default() };
    let (result, changes) = {
        let state: &dyn DatabaseRef = &*db;
        let mut evm = build_evm::<TempoEvmNetwork>(state, env, &opts)?;
        evm.set_database(EvmDb::for_storage(state));
        let result = StorageCtx::enter_evm_without_tip1060_accounting(&mut evm, f)?;
        evm.state_mut().commit_transaction();
        let changes = overlay_changes(state, &evm.overlay_db().cache)?;
        (result, changes)
    };
    db.commit(&changes);
    Ok(result)
}

/// Returns the accounts and slots of `cache` that differ from `db`.
fn overlay_changes(
    db: &dyn DatabaseRef,
    cache: &evm2::evm::Cache,
) -> Result<BlockStateAccumulator, BlockchainError> {
    let mut changes = BlockStateAccumulator::new();
    for (address, info) in &cache.accounts {
        let Some(info) = info else { continue };
        let original = db.basic_ref(*address)?;
        if original.as_ref() == Some(info) {
            continue;
        }
        if let Some(code) = info.code.as_ref().or_else(|| cache.contracts.get(&info.code_hash)) {
            let Ok(()) = changes.bytecode(info.code_hash, code);
        }
        // Keep Anvil's original account, including empty accounts with unrelated storage.
        let Ok(()) = changes.account(AccountChangeRef {
            address: *address,
            original: original.as_ref(),
            current: Some(info),
            created: false,
            selfdestructed: false,
        });
    }
    for (address, slots) in &cache.storage {
        for (slot, value) in &slots.slots {
            let original = db.storage_ref(*address, *slot)?;
            if original != *value {
                let Ok(()) = StateChangeSink::storage(
                    &mut changes,
                    StorageChange { address: *address, key: *slot, original, current: *value },
                );
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
    with_tempo_storage(db, env, hardfork, || {
        initialize_tempo_genesis(test_accounts, hardfork)
            .map_err(|err| BlockchainError::Message(format!("failed to initialize Tempo: {err}")))
    })
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
            // From T14 `authorize_key` reads the caller's account without loading it, as a
            // transaction's sender is already warm; load it here the same way.
            StorageCtx.with_account_info(account, |_| Ok(()))?;
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

/// Returns the name of a native precompile active at `hardfork`, using Tempo's execution lookup.
pub(crate) fn native_precompile_name(address: Address, hardfork: TempoHardfork) -> Option<String> {
    if !address.is_precompile(hardfork) {
        return None;
    }
    // evm2's `TempoPrecompiles` dispatches by address without naming its native precompiles.
    let name = match address {
        _ if address.is_tip20() => "TIP20Token",
        TIP20_FACTORY_ADDRESS => "TIP20Factory",
        TIP20_CHANNEL_RESERVE_ADDRESS => "TIP20ChannelReserve",
        ADDRESS_REGISTRY_ADDRESS => "AddressRegistry",
        TIP403_REGISTRY_ADDRESS => "TIP403Registry",
        TIP_FEE_MANAGER_ADDRESS => "TipFeeManager",
        STABLECOIN_DEX_ADDRESS => "StablecoinDEX",
        NONCE_PRECOMPILE_ADDRESS => "NonceManager",
        CONFIGURABLE_ACCOUNT_ADDRESS => "ConfigurableAccount",
        VALIDATOR_CONFIG_ADDRESS => "ValidatorConfig",
        ACCOUNT_KEYCHAIN_ADDRESS => "AccountKeychain",
        VALIDATOR_CONFIG_V2_ADDRESS => "ValidatorConfigV2",
        SIGNATURE_VERIFIER_ADDRESS => "SignatureVerifier",
        RECEIVE_POLICY_GUARD_ADDRESS => "ReceivePolicyGuard",
        STORAGE_CREDITS_ADDRESS => "StorageCredits",
        CURRENT_COMMITTEE_ADDRESS => "CurrentCommittee",
        ZONE_FACTORY_ADDRESS => "ZoneFactory",
        ZONE_VERIFIER_ADDRESS => "ZoneVerifier",
        _ => return None,
    };
    Some(name.to_string())
}

/// Creates a native TIP-20 at the requested address inside the current storage context.
///
/// Unlike genesis's reserved-address factory entry point, this development operation accepts any
/// TIP-20 address. All other quote-token checks follow the ordinary factory creation path.
/// Previously initialized tokens remain occupied even if their code has been cleared.
pub(super) fn create_token(
    request: &TempoCreateTokenRequest,
) -> Result<Address, TempoPrecompileError> {
    let mut token = TIP20Token::from_address(request.address)?;
    let factory = TIP20Factory::new();
    // Role initialization leaves this self-admin record even at zero supply and after code removal.
    if factory.is_tip20(request.address)?
        || token.get_role_admin(IRolesAuth::getRoleAdminCall { role: UNGRANTABLE_ROLE })?
            == UNGRANTABLE_ROLE
    {
        return Err(TIP20FactoryError::token_already_exists(request.address).into());
    }
    if !factory.is_tip20(request.quote_token)?
        || (request.currency == USD_CURRENCY
            && TIP20Token::from_address(request.quote_token)?.currency()? != USD_CURRENCY)
    {
        return Err(TIP20Error::invalid_quote_token().into());
    }

    token.initialize(
        request.admin,
        &request.name,
        &request.symbol,
        &request.currency,
        request.quote_token,
        request.admin,
    )?;
    token.grant_role_internal(request.admin, TIP20Token::issuer_role())?;
    for mint in &request.mint {
        token.mint(request.admin, ITIP20::mintCall { to: mint.to, amount: mint.amount })?;
    }
    Ok(request.address)
}
