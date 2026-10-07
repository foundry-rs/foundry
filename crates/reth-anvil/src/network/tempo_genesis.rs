//! The genesis state of a Tempo dev chain, as anvil seeds it: Tempo's precompiles and fee tokens,
//! fee token balances, keychain keys, and fee preferences for the dev accounts, and liquidity for
//! every fee token pair.

use super::tempo_storage::TempoStorage;
use alloy_genesis::GenesisAccount;
use alloy_primitives::{Address, B256, Bytes, U256, address};
use foundry_evm_core::tempo::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
    initialize_tempo_genesis_at_hardfork,
};
use std::collections::BTreeMap;
use tempo_hardfork::TempoHardfork;
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

/// The sender anvil initializes Tempo's genesis with.
const SENDER: Address = address!("0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38");
/// The admin anvil initializes Tempo's genesis with.
const ADMIN: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");

/// Returns the genesis accounts of a Tempo dev chain with the given dev accounts, as anvil
/// seeds it.
pub fn tempo_genesis_alloc(
    chain_id: u64,
    timestamp: u64,
    hardfork: TempoHardfork,
    dev_accounts: &[Address],
) -> Result<Vec<(Address, GenesisAccount)>, TempoPrecompileError> {
    let mut storage = TempoStorage::new(None, chain_id, 0, timestamp, hardfork);
    initialize_tempo_genesis_at_hardfork(&mut storage, ADMIN, SENDER, hardfork)?;

    let mint_amount = U256::from(u64::MAX);
    let tokens = [PATH_USD_ADDRESS, ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, THETA_USD_ADDRESS];
    StorageCtx::enter(&mut storage, || -> Result<(), TempoPrecompileError> {
        for &token in &tokens {
            let mut token = TIP20Token::from_address(token)?;
            for &account in dev_accounts {
                token.mint(ADMIN, ITIP20::mintCall { to: account, amount: mint_amount })?;
            }
        }

        // Each dev account signs Tempo transactions with its own secp256k1 key.
        let mut keychain = AccountKeychain::new();
        for &account in dev_accounts {
            keychain.set_tx_origin(account)?;
            keychain.authorize_key(
                account,
                account,
                SignatureType::Secp256k1,
                KeyRestrictions {
                    expiry: u64::MAX,
                    enforceLimits: false,
                    limits: vec![],
                    allowAnyCalls: true,
                    allowedCalls: vec![],
                },
                None,
            )?;
        }

        // The first three dev accounts pay fees in AlphaUSD, BetaUSD, and ThetaUSD, the others in
        // PathUSD, as on anvil.
        let mut fee_manager = TipFeeManager::new();
        fee_manager.initialize()?;
        for (index, &account) in dev_accounts.iter().enumerate() {
            let token = match index {
                0 => ALPHA_USD_ADDRESS,
                1 => BETA_USD_ADDRESS,
                2 => THETA_USD_ADDRESS,
                _ => PATH_USD_ADDRESS,
            };
            fee_manager.set_user_token(account, IFeeManager::setUserTokenCall { token })?;
        }

        for &token in &tokens {
            TIP20Token::from_address(token)?.mint(
                ADMIN,
                ITIP20::mintCall { to: TIP_FEE_MANAGER_ADDRESS, amount: mint_amount },
            )?;
        }

        // Liquidity in both directions between every pair of fee tokens, as in Tempo's genesis.
        let liquidity = U256::from(10u64.pow(10));
        for &user_token in &tokens {
            for &validator_token in &tokens {
                if user_token != validator_token {
                    fee_manager.mint(ADMIN, user_token, validator_token, liquidity, ADMIN)?;
                }
            }
        }
        Ok(())
    })?;

    Ok(storage
        .into_writes()
        .into_iter()
        .map(|(address, writes)| {
            let code = writes.code.map(|code| Bytes::from(code.original_bytes()));
            let storage: BTreeMap<_, _> = writes
                .storage
                .into_iter()
                .filter(|(_, value)| !value.is_zero())
                .map(|(key, value)| (B256::from(key), B256::from(value)))
                .collect();
            (
                address,
                GenesisAccount::default()
                    .with_code(code)
                    .with_storage((!storage.is_empty()).then_some(storage)),
            )
        })
        .collect())
}
