use ethers::{
    providers::Middleware,
    types::{Address, U256},
};
use revm::{BlockEnv, CfgEnv, Env, TxEnv};

/// Initializes a REVM block environment based on a forked
/// ethereum provider.
pub async fn environment<M: Middleware>(
    provider: &M,
    memory_limit: u64,
    gas_price: Option<u64>,
    override_chain_id: Option<u64>,
    pin_block: Option<u64>,
    origin: Address,
) -> eyre::Result<Env>
where
    M::Error: 'static,
{
    let block_number = if let Some(pin_block) = pin_block {
        pin_block
    } else {
        provider.get_block_number().await?.as_u64()
    };
    let (fork_gas_price, rpc_chain_id, block) = tokio::try_join!(
        provider.get_gas_price(),
        provider.get_chainid(),
        provider.get_block(block_number)
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

    Ok(Env {
        cfg: CfgEnv {
            chain_id: override_chain_id.unwrap_or(rpc_chain_id.as_u64()).into(),
            memory_limit,
            ..Default::default()
        },
        block: BlockEnv {
            number: block.number.expect("block number not found").as_u64().into(),
            timestamp: block.timestamp,
            coinbase: block.author.unwrap_or_default(),
            difficulty: block.difficulty,
            basefee: block.base_fee_per_gas.unwrap_or_default(),
            gas_limit: block.gas_limit,
        },
        tx: TxEnv {
            caller: origin,
            gas_price: gas_price.map(U256::from).unwrap_or(fork_gas_price),
            chain_id: Some(override_chain_id.unwrap_or(rpc_chain_id.as_u64())),
            gas_limit: block.gas_limit.as_u64(),
            ..Default::default()
        },
    })
}
