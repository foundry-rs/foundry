use crate::utils::apply_chain_and_block_specific_env_changes;
use alloy_primitives::{Address, U256, U64};
use alloy_providers::provider::TempProvider;
use alloy_rpc_types::{Block, BlockNumberOrTag};
use eyre::WrapErr;
use foundry_common::NON_ARCHIVE_NODE_WARNING;
use revm::primitives::{BlockEnv, CfgEnv, Env, TxEnv};

/// Initializes a REVM block environment based on a forked
/// ethereum provider.
// todo(onbjerg): these bounds needed cus of the bounds in `Provider`, can simplify?
pub async fn environment<P: TempProvider>(
    provider: &P,
    memory_limit: u64,
    gas_price: Option<u64>,
    override_chain_id: Option<u64>,
    pin_block: Option<u64>,
    origin: Address,
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
    cfg.chain_id = override_chain_id.unwrap_or(rpc_chain_id.to::<u64>());
    cfg.memory_limit = memory_limit;
    cfg.limit_contract_code_size = Some(usize::MAX);
    // EIP-3607 rejects transactions from senders with deployed code.
    // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the caller
    // is a contract. So we disable the check by default.
    cfg.disable_eip3607 = true;

    let mut env = Env {
        cfg,
        block: BlockEnv {
            number: block.header.number.expect("block number not found"),
            timestamp: block.header.timestamp,
            coinbase: block.header.miner,
            difficulty: block.header.difficulty,
            prevrandao: Some(block.header.mix_hash),
            basefee: block.header.base_fee_per_gas.unwrap_or_default(),
            gas_limit: block.header.gas_limit,
            ..Default::default()
        },
        tx: TxEnv {
            caller: origin,
            gas_price: gas_price.map(U256::from).unwrap_or(fork_gas_price),
            chain_id: Some(override_chain_id.unwrap_or(rpc_chain_id.to::<u64>())),
            gas_limit: block.header.gas_limit.to::<u64>(),
            ..Default::default()
        },
    };

    apply_chain_and_block_specific_env_changes(&mut env, &block);

    Ok((env, block))
}
