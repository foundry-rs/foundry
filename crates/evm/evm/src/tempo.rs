use alloy_chains::NamedChain;
use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::{
    backend::DatabaseError,
    constants::{CALLER, TEST_CONTRACT_ADDRESS},
    tempo::{TEMPO_TIP20_TOKENS, TempoStorageProvider, initialize_tempo_genesis},
};
use foundry_evm_hardforks::FoundryHardfork;
use revm::state::{AccountInfo, Bytecode};
use tempo_precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, NONCE_PRECOMPILE_ADDRESS, STABLECOIN_DEX_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS, TIP20_FACTORY_ADDRESS, TIP403_REGISTRY_ADDRESS,
    VALIDATOR_CONFIG_ADDRESS, VALIDATOR_CONFIG_V2_ADDRESS, error::TempoPrecompileError,
};

use crate::executors::Executor;

/// Initialize Tempo precompiles and contracts for the given executor.
///
/// This initialization should be kept aligned with Tempo's genesis file to ensure
/// executor environments accurately reflect production behavior.
///
/// Ref: <https://github.com/tempoxyz/tempo/blob/main/xtask/src/genesis_args.rs>
pub fn initialize_tempo_precompiles_and_contracts(
    executor: &mut Executor,
    hardfork: Option<FoundryHardfork>,
) -> Result<(), TempoPrecompileError> {
    let sender = CALLER;
    let admin = TEST_CONTRACT_ADDRESS;

    let chain_id = executor.evm_env().cfg_env.chain_id;
    let timestamp = U256::from(executor.evm_env().block_env.timestamp);
    let block_number = executor.evm_env().block_env.number.to::<u64>();
    let tempo_hardfork = hardfork
        .and_then(|hf| match hf {
            FoundryHardfork::Tempo(t) => Some(t),
            _ => None,
        })
        .unwrap_or_default();
    let mut storage = TempoStorageProvider::new(
        executor.backend_mut(),
        chain_id,
        timestamp,
        block_number,
        tempo_hardfork,
    );

    initialize_tempo_genesis(&mut storage, admin, sender)
}

/// Pre-warm Tempo precompile accounts in the fork backend cache.
///
/// In fork mode, Tempo precompile addresses are Rust-native precompiles on the Tempo node
/// with no real EVM bytecode. The RPC returns empty code for these addresses, causing the
/// fork backend to repeatedly fetch them via RPC on every access. During invariant fuzzing
/// this creates a pathological RPC storm.
///
/// This function inserts sentinel bytecode (`0xef`) into the local fork cache for all
/// known precompile addresses, preventing repeated RPC round-trips.
///
/// Only applies when the fork target is a known Tempo chain (by chain ID).
pub fn warm_tempo_precompile_accounts(executor: &mut Executor) -> Result<(), DatabaseError> {
    let chain_id = executor.evm_env().cfg_env.chain_id;
    if !NamedChain::try_from(chain_id).is_ok_and(|c| c.is_tempo()) {
        return Ok(());
    }

    let sentinel = Bytecode::new_legacy(Bytes::from_static(&[0xef]));
    let precompile_addresses: &[Address] = &[
        NONCE_PRECOMPILE_ADDRESS,
        STABLECOIN_DEX_ADDRESS,
        TIP20_FACTORY_ADDRESS,
        TIP403_REGISTRY_ADDRESS,
        TIP_FEE_MANAGER_ADDRESS,
        VALIDATOR_CONFIG_ADDRESS,
        VALIDATOR_CONFIG_V2_ADDRESS,
        ACCOUNT_KEYCHAIN_ADDRESS,
    ];

    for addr in precompile_addresses.iter().chain(TEMPO_TIP20_TOKENS.iter()) {
        executor.backend_mut().insert_account_info(
            *addr,
            AccountInfo {
                code_hash: sentinel.hash_slow(),
                code: Some(sentinel.clone()),
                nonce: 1,
                ..Default::default()
            },
        );
    }

    Ok(())
}
