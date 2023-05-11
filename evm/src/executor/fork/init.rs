use crate::utils::{
    apply_chain_and_block_specific_env_changes, h160_to_b160, h256_to_b256, u256_to_ru256,
};
use ethers::{
    providers::Middleware,
    types::{Address, Block, TxHash, U256},
};
use eyre::WrapErr;
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
            eyre::bail!(
                "Failed to get block for block number: {}\nlatest block number: {}",
                block_number,
                latest_block
            );
        }
        eyre::bail!("Failed to get block for block number: {}", block_number)
    };

    let mut env = Env {
        cfg: CfgEnv {
            chain_id: u256_to_ru256(override_chain_id.unwrap_or(rpc_chain_id.as_u64()).into()),
            memory_limit,
            limit_contract_code_size: Some(usize::MAX),
            // EIP-3607 rejects transactions from senders with deployed code.
            // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the caller
            // is a contract. So we disable the check by default.
            disable_eip3607: true,
            ..Default::default()
        },
        block: BlockEnv {
            number: u256_to_ru256(block.number.expect("block number not found").as_u64().into()),
            timestamp: block.timestamp.into(),
            coinbase: h160_to_b160(block.author.unwrap_or_default()),
            difficulty: block.difficulty.into(),
            prevrandao: Some(block.mix_hash.unwrap_or_default()).map(h256_to_b256),
            basefee: block.base_fee_per_gas.unwrap_or_default().into(),
            gas_limit: block.gas_limit.into(),
        },
        tx: TxEnv {
            caller: h160_to_b160(origin),
            gas_price: gas_price.map(U256::from).unwrap_or(fork_gas_price).into(),
            chain_id: Some(override_chain_id.unwrap_or(rpc_chain_id.as_u64())),
            gas_limit: block.gas_limit.as_u64(),
            ..Default::default()
        },
    };

    apply_chain_and_block_specific_env_changes(&mut env, &block);

    Ok((env, block))
}
