use crate::utils::apply_chain_and_block_specific_env_changes;
use alloy_primitives::{Address, U256};
use alloy_provider::{Network, Provider};
use alloy_rpc_types::{Block, BlockNumberOrTag};
use alloy_transport::Transport;
use eyre::WrapErr;
use foundry_common::NON_ARCHIVE_NODE_WARNING;

use revm::primitives::{BlockEnv, CfgEnv, Env, TxEnv};

/// Initializes a REVM block environment based on a forked
/// ethereum provider.
pub async fn environment<N: Network, T: Transport + Clone, P: Provider<T, N>>(
    provider: &P,
    memory_limit: u64,
    gas_price: Option<u128>,
    override_chain_id: Option<u64>,
    pin_block: Option<u64>,
    origin: Address,
    disable_block_gas_limit: bool,
) -> eyre::Result<(Env, Block)> {
    let block_number = if let Some(pin_block) = pin_block {
        pin_block
    } else {
        provider.get_block_number().await.wrap_err("Failed to get latest block number")?
    };
    let (fork_gas_price, rpc_chain_id, block) = tokio::try_join!(
        provider.get_gas_price(),
        provider.get_chain_id(),
        provider.get_block_by_number(BlockNumberOrTag::Number(block_number), false)
    )?;
    let block = if let Some(block) = block {
        block
    } else {
        if let Ok(latest_block) = provider.get_block_number().await {
            // If the `eth_getBlockByNumber` call succeeds, but returns null instead of
            // the block, and the block number is less than equal the latest block, then
            // the user is forking from a non-archive node with an older block number.
            if block_number <= latest_block {
                error!("{NON_ARCHIVE_NODE_WARNING}");
            }
            eyre::bail!(
                "Failed to get block for block number: {}\nlatest block number: {}",
                block_number,
                latest_block
            );
        }
        eyre::bail!("Failed to get block for block number: {}", block_number)
    };

    let mut cfg = CfgEnv::default();
    cfg.chain_id = override_chain_id.unwrap_or(rpc_chain_id);
    cfg.memory_limit = memory_limit;
    cfg.limit_contract_code_size = Some(usize::MAX);
    // EIP-3607 rejects transactions from senders with deployed code.
    // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the caller
    // is a contract. So we disable the check by default.
    cfg.disable_eip3607 = true;
    cfg.disable_block_gas_limit = disable_block_gas_limit;

    let mut env = Env {
        cfg,
        block: BlockEnv {
            number: U256::from(block.header.number.expect("block number not found")),
            timestamp: U256::from(block.header.timestamp),
            coinbase: block.header.miner,
            difficulty: block.header.difficulty,
            prevrandao: Some(block.header.mix_hash.unwrap_or_default()),
            basefee: U256::from(block.header.base_fee_per_gas.unwrap_or_default()),
            gas_limit: U256::from(block.header.gas_limit),
            ..Default::default()
        },
        tx: TxEnv {
            caller: origin,
            gas_price: U256::from(gas_price.unwrap_or(fork_gas_price)),
            chain_id: Some(override_chain_id.unwrap_or(rpc_chain_id)),
            gas_limit: block.header.gas_limit as u64,
            ..Default::default()
        },
    };

    apply_chain_and_block_specific_env_changes(&mut env, &block);

    Ok((env, block))
}
