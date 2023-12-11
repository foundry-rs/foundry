use crate::utils::apply_chain_and_block_specific_env_changes;
use alloy_primitives::{Address, U256};
use ethers_core::types::{Block, TxHash};
use ethers_providers::Middleware;
use eyre::WrapErr;
use foundry_common::{types::ToAlloy, NON_ARCHIVE_NODE_WARNING};
use futures::TryFutureExt;
use revm::primitives::{BlockEnv, CfgEnv, Env, TxEnv};

/// Initializes a REVM block environment based on a forked
/// ethereum provider.
pub async fn environment<M: Middleware>(
    provider: &M,
    memory_limit: u64,
    gas_price: Option<u64>,
    override_chain_id: Option<u64>,
    pin_block: Option<u64>,
    origin: Address,
) -> eyre::Result<(Env, Block<TxHash>)>
where
    M::Error: 'static,
{
    let block_number = if let Some(pin_block) = pin_block {
        pin_block
    } else {
        provider.get_block_number().await.wrap_err("Failed to get latest block number")?.as_u64()
    };
    let (fork_gas_price, rpc_chain_id, block) = tokio::try_join!(
        provider
            .get_gas_price()
            .map_err(|err| { eyre::Error::new(err).wrap_err("Failed to get gas price") }),
        provider
            .get_chainid()
            .map_err(|err| { eyre::Error::new(err).wrap_err("Failed to get chain id") }),
        provider.get_block(block_number).map_err(|err| {
            eyre::Error::new(err).wrap_err(format!("Failed to get block {block_number}"))
        })
    )?;
    let block = if let Some(block) = block {
        block
    } else {
        if let Ok(latest_block) = provider.get_block_number().await {
            // If the `eth_getBlockByNumber` call succeeds, but returns null instead of
            // the block, and the block number is less than equal the latest block, then
            // the user is forking from a non-archive node with an older block number.
            if block_number <= latest_block.as_u64() {
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
    cfg.chain_id = override_chain_id.unwrap_or(rpc_chain_id.as_u64());
    cfg.memory_limit = memory_limit;
    cfg.limit_contract_code_size = Some(usize::MAX);
    // EIP-3607 rejects transactions from senders with deployed code.
    // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the caller
    // is a contract. So we disable the check by default.
    cfg.disable_eip3607 = true;

    let mut env = Env {
        cfg,
        block: BlockEnv {
            number: U256::from(block.number.expect("block number not found").as_u64()),
            timestamp: block.timestamp.to_alloy(),
            coinbase: block.author.unwrap_or_default().to_alloy(),
            difficulty: block.difficulty.to_alloy(),
            prevrandao: Some(block.mix_hash.map(|h| h.to_alloy()).unwrap_or_default()),
            basefee: block.base_fee_per_gas.unwrap_or_default().to_alloy(),
            gas_limit: block.gas_limit.to_alloy(),
            ..Default::default()
        },
        tx: TxEnv {
            caller: origin,
            gas_price: gas_price.map(U256::from).unwrap_or(fork_gas_price.to_alloy()),
            chain_id: Some(override_chain_id.unwrap_or(rpc_chain_id.as_u64())),
            gas_limit: block.gas_limit.as_u64(),
            ..Default::default()
        },
    };

    apply_chain_and_block_specific_env_changes(&mut env, &block);

    Ok((env, block))
}
